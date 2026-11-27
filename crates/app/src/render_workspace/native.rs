use super::{gpu, raytrace};
use crate::{Dialog, PcbApp, ViewId, editor_layout::HostId, editor_windows};
use eframe::egui_wgpu::RenderState;
use egui::{Context, Rect, Ui};
use pcbcraft_board_solid::{Thickness, display_board};
use pcbcraft_chrome::{
    Tokens,
    render_workspace::{self as chrome, Drag, ViewAction},
    workspace::{self, Placement, RibbonTool, ToolIcon},
};
use pcbcraft_engine::{Project, board::view::BoardTarget};
use solidcraft_viewport::{Camera, StandardView, vec::Bounds};
use solidcraft_viewport_gpu::view::{GpuScene, SceneSlot};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

/// A job may publish only to the original view, board and immutable project revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) view: ViewId,
    pub(super) target: BoardTarget,
    pub(super) revision: u64,
}

/// Report background work to its pane, including when the host is a detached window.
#[derive(Clone)]
pub(super) struct Progress {
    ctx: Context,
    host: HostId,
    message: Arc<Mutex<&'static str>>,
}

impl Progress {
    pub(super) fn new(ctx: &Context, host: HostId, message: &'static str) -> Self {
        Self { ctx: ctx.clone(), host, message: Arc::new(Mutex::new(message)) }
    }

    pub(super) fn report(&self, message: &'static str) {
        if let Ok(mut current) = self.message.lock() {
            *current = message;
        }
        self.repaint();
    }

    pub(super) fn message(&self) -> Option<&'static str> {
        self.message.lock().ok().map(|message| *message)
    }

    pub(super) fn repaint(&self) {
        self.ctx.request_repaint_of(editor_windows::viewport(self.host));
        self.ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

struct Mesh {
    bounds: Bounds,
    thickness: Thickness,
    scene: GpuScene,
}

#[derive(Default)]
pub(crate) struct State {
    target: Option<BoardTarget>,
    request: Option<Request>,
    mesh: Option<(Bounds, Thickness, SceneSlot)>,
    error: Option<String>,
    pipeline: Option<gpu::Pipeline>,
    key: u64,
    camera: Camera,
    fit: bool,
    compass_drag: Drag,
    trace: LiveTrace,
}

#[derive(Default)]
struct LiveTrace {
    enabled: bool,
    pose: Option<(raytrace::Pose, Instant)>,
    submitted: Option<raytrace::Pose>,
    navigating: bool,
    pending: bool,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

impl LiveTrace {
    const SETTLE: Duration = Duration::from_millis(650);

    fn clear_frames(&mut self) {
        *self = Self { enabled: self.enabled, ..Self::default() };
    }

    fn observe(&mut self, pose: raytrace::Pose, navigating: bool, now: Instant) -> bool {
        if self.pose.is_some_and(|(previous, _)| previous == pose) && self.navigating == navigating {
            return false;
        }
        self.pose = Some((pose, now));
        self.navigating = navigating;
        self.submitted = None;
        self.texture = None;
        true
    }

    /// Return the outstanding wait for a new pose. An active drag has no deadline until release.
    fn settle_remaining(&self, now: Instant) -> Option<Duration> {
        let (pose, changed_at) = self.pose?;
        if self.navigating || self.pending || self.submitted == Some(pose) {
            return None;
        }
        Some(Self::SETTLE.saturating_sub(now.saturating_duration_since(changed_at)))
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderState").field("target", &self.target).field("request", &self.request).finish_non_exhaustive()
    }
}

impl State {
    /// Copy the board and camera into another pane with independent input and GPU resources.
    pub(crate) fn fork(&self) -> Self {
        Self { target: self.target, camera: self.camera, fit: self.fit, ..Self::default() }
    }

    /// Retarget only this view. Clear displayed geometry until the new target is ready.
    fn set_target(&mut self, target: BoardTarget) {
        if self.target != Some(target) {
            self.target = Some(target);
            self.request = None;
            self.mesh = None;
            self.error = None;
            self.trace.clear_frames();
            self.camera = Camera::default();
            self.fit();
        } else if self.error.take().is_some() {
            self.request = None;
        }
    }

    /// Fit the displayed board on the next paint with the pane's current aspect ratio.
    pub(crate) fn fit(&mut self) {
        self.fit = true;
    }

    /// Describe this read-only presentation to live clients.
    pub(crate) fn inspect(&self) -> serde_json::Value {
        serde_json::json!({
            "board": self.target,
            "status": if self.error.is_some() {"error"} else if self.mesh.is_some() {"ready"} else if self.target.is_some() {"building"} else {"empty"},
            "error": self.error,
            "thickness_nm": self.mesh.as_ref().map(|(_, thickness, _)| thickness.nm()),
            "ray_tracing": if !self.trace.enabled {"off"} else if self.trace.error.is_some() {"error"} else if self.trace.pending {"rendering"} else if self.trace.texture.is_none() {"settling"} else {"ready"},
            "ray_tracing_error": self.trace.error,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Browse {
    view: ViewId,
    host: HostId,
    root_revision: u64,
    selected: Option<BoardTarget>,
}

type Built = Result<(Mesh, Option<gpu::Pipeline>), String>;

struct PreviewJob {
    request: Request,
    receiver: mpsc::Receiver<Built>,
    progress: Progress,
}

/// Serialize mesh generation and initial shader compilation across panes. Closing a pane
/// discards its result and does not queue more workers behind the abandoned computation.
#[derive(Default)]
pub(crate) struct Worker {
    render: Option<RenderState>,
    pending: Option<PreviewJob>,
    raytrace: raytrace::Worker,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderWorker").field("pending", &self.pending.as_ref().map(|pending| pending.request)).finish_non_exhaustive()
    }
}

impl Worker {
    /// Retain the renderer device supplied by eframe without initializing any viewport shaders.
    pub(crate) fn new(render: Option<RenderState>) -> Self {
        let raytrace = raytrace::Worker::new(render.as_ref());
        Self { render, raytrace, ..Self::default() }
    }

    /// Start one job if the worker is idle. A busy pane retries against the latest snapshot.
    fn start(&mut self, ctx: &Context, host: HostId, request: Request, project: Arc<Project>, needs_pipeline: bool) -> Result<bool, String> {
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
            return Ok(false);
        }
        let (tx, rx) = mpsc::channel();
        let render = self.render.clone().filter(|_| needs_pipeline);
        let progress = Progress::new(ctx, host, "Building board preview…");
        let background = progress.clone();
        std::thread::Builder::new()
            .name("pcbcraft-board-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mesh = build(&project, request.target)?;
                    let pipeline = render.as_ref().map(|render| {
                        background.report("Preparing 3D preview shaders…");
                        gpu::pipeline(render)
                    });
                    Ok((mesh, pipeline))
                }))
                .unwrap_or_else(|_| Err("The board render stopped before it completed.".into()));
                let _ = tx.send(result);
                background.repaint();
            })
            .map_err(|error| format!("Could not start 3D rendering. {error}"))?;
        self.pending = Some(PreviewJob { request, receiver: rx, progress });
        Ok(true)
    }

    fn preview_progress(&self, request: Option<Request>) -> Option<&'static str> {
        let pending = self.pending.as_ref()?;
        (Some(pending.request) == request).then(|| pending.progress.message()).flatten()
    }

    /// Release the worker after success, failure, or a job that exited before returning a result.
    fn poll(&mut self) -> Option<(Request, Built)> {
        let pending = self.pending.as_ref()?;
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("The board render stopped before it completed.".into()),
        };
        let request = pending.request;
        self.pending = None;
        Some((request, result))
    }
}

/// Resolve the captured board and keep SolidCore errors local to this preview.
fn build(project: &Project, target: BoardTarget) -> Result<Mesh, String> {
    let board = project.board_view(target).ok_or("This board is no longer available.")?;
    let (board_mesh, thickness) = display_board(board, 10_000).map_err(|err| err.to_string())?;
    let (bounds, scene) = gpu::scene(board_mesh, board, thickness.nm())?;
    Ok(Mesh { bounds, thickness, scene })
}

impl PcbApp {
    /// Open on the focused PCB if available, otherwise on the project's sole registered board.
    pub(crate) fn place_render(&mut self, ctx: &Context, placement: Placement) -> Result<(), String> {
        let project = self.session.project().ok_or("Open or create a project to inspect a board.")?;
        let target = self
            .views
            .active
            .render
            .as_ref()
            .and_then(|state| state.target)
            .or(self.views.active.board.target)
            .filter(|&target| project.board_view(target).is_some())
            .or_else(|| project.board_target());
        let source = self.views.active.id();
        let (id, host) = match placement {
            Placement::Here => (source, self.views.host),
            Placement::Beside => (self.views.open_beside(source).ok_or("Could not create a 3D render pane.")?, self.views.host),
            Placement::Window => {
                let host = self.views.open_window(source).ok_or("Could not create a 3D render window.")?;
                (self.views.hosts[&host].focused, host)
            }
        };
        let view = self.views.view_mut(id).ok_or("The 3D render pane is unavailable.")?;
        view.release_input();
        view.definition = None;
        view.library = None;
        view.manufacturing = None;
        let state = view.render.get_or_insert_with(State::default);
        if let Some(target) = target {
            state.set_target(target);
        }
        self.library_editors.focus(None)?;
        self.home = false;
        if host == self.views.host {
            self.activate_editor(ctx, id);
        }
        ctx.send_viewport_cmd_to(editor_windows::viewport(host), egui::ViewportCommand::Focus);
        self.repaint_editors(ctx);
        Ok(())
    }

    /// Apply a board choice through the same entry point for Browse and live commands.
    pub(crate) fn set_render_board(&mut self, ctx: &Context, view: ViewId, target: BoardTarget) -> Result<(), String> {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return Err("Finish the current project operation before choosing a board.".into());
        }
        self.session.project().and_then(|project| project.board_view(target)).ok_or("This board is no longer available.")?;
        let render = self.views.view_mut(view).and_then(|view| view.render.as_mut()).ok_or("Choose a 3D render pane.")?;
        render.set_target(target);
        self.repaint_editors(ctx);
        Ok(())
    }

    /// Adopt only geometry that still matches the viewer and session. Retain camera on edits and undo.
    pub(crate) fn poll_render(&mut self, ctx: &Context) {
        self.poll_board_image(ctx);
        let Some((request, result)) = self.render_worker.poll() else { return };
        if let Some(state) = self.views.view_mut(request.view).and_then(|view| view.render.as_mut())
            && state.request == Some(request)
            && request.revision == self.session.revision()
        {
            match result {
                Ok((mesh, pipeline)) => {
                    state.key = state.key.wrapping_add(1);
                    state.mesh = Some((mesh.bounds, mesh.thickness, Arc::new(Mutex::new(Some(mesh.scene)))));
                    state.error = None;
                    if let Some(pipeline) = pipeline {
                        state.pipeline = Some(pipeline);
                    }
                }
                Err(error) => {
                    state.mesh = None;
                    state.error = Some(error);
                }
            }
        }
        self.repaint_editors(ctx);
    }

    /// Enable or disable GPU tracing in the addressed pane. Navigation and history stay shared
    /// with the raster preview.
    pub(crate) fn board_image_action(&mut self, ctx: &Context, view: ViewId, action: &str) -> Result<(), String> {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return Err("Finish the current project operation before rendering.".into());
        }
        let state = self.views.view_mut(view).and_then(|view| view.render.as_mut()).ok_or("Choose a 3D render pane.")?;
        let enabled = match action {
            "render" if self.render_worker.raytrace.available() => true,
            "render" => return Err("GPU ray tracing requires a compatible hardware Vulkan or Metal adapter.".into()),
            "cancel" | "preview" => false,
            _ => return Err("Unknown board render action.".into()),
        };
        if state.trace.enabled != enabled {
            if !enabled && let Some(request) = state.request {
                self.render_worker.raytrace.cancel(request);
            }
            state.trace.enabled = enabled;
            state.trace.clear_frames();
            self.repaint_editors(ctx);
        }
        Ok(())
    }

    /// Only the active board revision may publish. Turning tracing off invalidates even a
    /// completion already sent by the worker.
    fn poll_board_image(&mut self, ctx: &Context) {
        let Some(request) = self.render_worker.raytrace.request() else { return };
        let state = self.views.view_mut(request.view).and_then(|view| view.render.as_mut());
        let current = request.revision == self.session.revision()
            && state.as_ref().is_some_and(|state| state.trace.enabled && state.trace.pending && state.request == Some(request));
        if !current {
            self.render_worker.raytrace.cancel(request);
        }
        let Some((_, result)) = self.render_worker.raytrace.poll() else { return };
        if current && let Some(state) = state {
            let trace = &mut state.trace;
            trace.pending = false;
            match result {
                Some(Ok(image)) if trace.submitted.is_some() => {
                    trace.texture = Some(ctx.load_texture("Ray-traced board", image, egui::TextureOptions::LINEAR));
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    trace.texture = None;
                    trace.error = Some(error);
                }
                None => trace.submitted = None,
            }
        }
        self.repaint_editors(ctx);
    }

    /// Keep a Browse dialog in the window whose 3D pane opened it.
    pub(crate) fn render_browse_host(&self) -> Option<HostId> {
        match &self.dialog {
            Dialog::BrowseRenderBoard(browse) => Some(browse.host),
            _ => None,
        }
    }

    /// Choose from live boards in this opening. Closing the dialog leaves the display unchanged.
    pub(crate) fn render_board_dialog(&mut self, ctx: &Context, mut browse: Browse) {
        if browse.root_revision != self.session.history_root_revision() || self.views.view(browse.view).is_none_or(|view| view.render.is_none()) {
            return;
        }
        let Some(project) = self.session.project() else { return };
        let boards: Vec<_> = project.boards().map(|board| (board.target, board.name())).collect();
        let response = chrome::browse_boards(ctx, &boards, &mut browse.selected);
        if let (true, Some(target)) = (response.primary, browse.selected) {
            self.message = self.set_render_board(ctx, browse.view, target).err();
        } else if !response.close {
            self.dialog = Dialog::BrowseRenderBoard(browse);
        }
    }

    /// Paint the standard shell in a read-only host, with no PCB edit shortcuts.
    pub(crate) fn render_frame(&mut self, ui: &mut Ui, integrated: bool) {
        let ctx = ui.ctx().clone();
        let enabled = matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
        let previous = std::mem::replace(&mut self.integrated_titlebar, integrated);
        self.app_bar(ui, enabled);
        self.integrated_titlebar = previous;
        let main = self.views.host == self.views.main;
        if main {
            self.assistant_shortcuts(&ctx);
        }
        ui.add_enabled_ui(enabled, |ui| {
            self.render_ribbon(ui);
            let area = ui.available_rect_before_wrap();
            self.editor_workspace(ui, area);
            if main {
                self.host_assistant(&ctx, Some(area), false, self.session.project().is_some());
            }
        });
        let dialog_host =
            self.arrangement_dialog_host().or_else(|| self.rule_dialog_host()).or_else(|| self.render_browse_host()).unwrap_or(self.views.main);
        if self.views.host == dialog_host {
            self.project_dialogs(&ctx);
        }
    }

    /// Offer board selection and Fit through the standard workspace ribbon.
    fn render_ribbon(&mut self, ui: &mut Ui) {
        let entries = self.workspace_entries();
        let id = self.views.active.id();
        let Some(state) = self.views.active.render.as_ref() else { return };
        let selected = state.target;
        let available = self.render_worker.raytrace.available();
        let enabled = state.trace.enabled;
        let toggle = RibbonTool {
            active: enabled,
            menu_check: Some(pcbcraft_chrome::menu::CheckKind::Toggle),
            ..tool(
                ToolIcon::Shared("camera"),
                "Ray tracing",
                if available || enabled { "Toggle live GPU ray tracing" } else { "GPU ray tracing requires a hardware Vulkan or Metal adapter" },
                enabled || available && state.mesh.is_some(),
            )
        };
        let tools = [
            tool(ToolIcon::Shared("folder"), "Browse…", "Choose a board in this project", self.session.project().is_some()),
            tool(ToolIcon::Fit, "Fit", "Fit the board", state.mesh.is_some()),
            toggle,
        ];
        let mut action = None;
        let response =
            workspace::ribbon(ui, "3D render", &["View"], &mut 0, workspace::Workspaces { entries: &entries, ..crate::WORKSPACES }, &[], |ui, _| {
                action = workspace::ribbon_group(ui, "Board", &tools)
            });
        if let Some(pick) = response.workspace {
            self.pick_workspace(ui.ctx(), pick);
        }
        match action {
            Some(0) => {
                self.dialog = Dialog::BrowseRenderBoard(Browse {
                    view: id,
                    host: self.views.host,
                    root_revision: self.session.history_root_revision(),
                    selected,
                });
            }
            Some(1) => self.fit_view(id),
            Some(2) => {
                if let Err(error) = self.board_image_action(ui.ctx(), id, if enabled { "preview" } else { "render" })
                    && let Some(state) = self.views.view_mut(id).and_then(|view| view.render.as_mut())
                {
                    state.trace.error = Some(error);
                }
            }
            _ => {}
        }
    }

    /// Present the current read-only snapshot. A busy worker never blocks pane navigation.
    pub(crate) fn render_pane(&mut self, ui: &mut Ui, id: ViewId) {
        let host = self.views.host;
        let Some(state) = self.views.view_mut(id).and_then(|view| view.render.as_mut()) else { return };
        let snapshot = self.session.snapshot();
        if let Some(project) = snapshot.as_deref()
            && state.target.is_none()
            && let Some(target) = project.board_target()
        {
            state.set_target(target);
        }
        let board_name =
            snapshot.as_ref().and_then(|project| state.target.and_then(|target| project.board_view(target))).map(|board| board.name().to_owned());
        let current = state.target.map(|target| Request { view: id, target, revision: self.session.revision() });
        if let (Some(request), Some(project)) = (current, snapshot.as_ref())
            && state.request != current
        {
            state.mesh = None;
            state.error = None;
            state.trace.clear_frames();
            match self.render_worker.start(ui.ctx(), host, request, Arc::clone(project), state.pipeline.is_none()) {
                Ok(true) => state.request = current,
                Ok(false) => {}
                Err(error) => {
                    state.request = current;
                    state.error = Some(error);
                }
            }
        }
        let tokens = Tokens::get();
        let (canvas, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(canvas);
        solidcraft_viewport_egui::paint_background(&painter, canvas, tokens.canvas, tokens.board_canvas);
        if !canvas.is_positive() {
            return;
        }
        if let Some((bounds, thickness, scene)) = state.mesh.as_ref()
            && let Some(pipeline) = state.pipeline.as_ref()
        {
            let navigator = chrome::compass_rect(canvas);
            let dragging = navigate(ui, canvas, navigator, &mut state.camera);
            let compass_held = ui.is_enabled()
                && ui.input(|input| input.pointer.primary_down() && input.pointer.press_origin().is_some_and(|origin| navigator.contains(origin)));
            let navigating = dragging || compass_held;
            let aspect = canvas.width() / canvas.height();
            if std::mem::take(&mut state.fit) {
                state.camera.fit_aspect(bounds, f64::from(aspect));
            }
            let pose = raytrace::Pose { camera: state.camera, aspect };
            if !navigating
                && state.trace.submitted == Some(pose)
                && let Some(texture) = &state.trace.texture
            {
                painter.image(texture.id(), canvas, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            } else {
                gpu::paint(ui, canvas, &state.camera, bounds, state.key, scene, pipeline);
            }
            // Keep the same compass and input in both rendering modes.
            let (right, up, back) = state.camera.basis();
            let (view, orbit) = chrome::compass(ui, navigator, [array(right), array(up), array(back)], &mut state.compass_drag);
            if let Some(step) = orbit {
                state.camera.orbit(step.dx, step.dy);
                ui.ctx().request_repaint();
            }
            if let Some(view) = view {
                match view {
                    ViewAction::Home => {
                        state.camera.set_view(StandardView::Iso);
                        state.fit = true;
                    }
                    ViewAction::Snap { axis, .. } => snap_view(&mut state.camera, axis),
                    // The compass offers Turn Around only with a hover label, and chrome passes none.
                    ViewAction::TurnAround => {}
                }
                ui.ctx().request_repaint();
            }
            if let Some(name) = &board_name {
                workspace::canvas_heading(&painter, canvas, Rect::NOTHING, navigator, name, "", [tokens.text, tokens.text_dim]);
            }
            if *thickness == Thickness::NominalFallback {
                painter.text(
                    canvas.left_bottom() + egui::vec2(12.0, -12.0),
                    egui::Align2::LEFT_BOTTOM,
                    "Nominal thickness · 1.6 mm",
                    egui::FontId::proportional(tokens.font_label),
                    tokens.text_dim,
                );
            }
            let trace = &mut state.trace;
            if trace.enabled
                && trace.error.is_none()
                && let (Some(request), Some(project)) = (current, snapshot)
            {
                let now = Instant::now();
                let pose = raytrace::Pose { camera: state.camera, aspect };
                if trace.observe(pose, navigating, now) && trace.pending {
                    self.render_worker.raytrace.cancel(request);
                }
                update_trace(ui.ctx(), &mut self.render_worker.raytrace, trace, host, request, project, now);
            }
            if let Some(error) = &trace.error {
                chrome::render_notice(ui, canvas, error, false);
            } else if trace.pending
                && trace.submitted.is_some()
                && let Some(message) = current.and_then(|request| self.render_worker.raytrace.progress(request))
            {
                chrome::render_notice(ui, canvas, message, true);
            }
        } else {
            match (&state.error, state.target, &state.mesh) {
                (Some(error), _, _) => chrome::render_notice(ui, canvas, error, false),
                (None, None, _) => chrome::render_notice(ui, canvas, "This project has no PCB.", false),
                (None, _, Some(_)) => chrome::render_notice(ui, canvas, "3D rendering requires a graphics device.", false),
                (None, Some(_), None) => {
                    let progress = self.render_worker.preview_progress(current).unwrap_or("Waiting for 3D preview…");
                    chrome::render_notice(ui, canvas, progress, true);
                }
            }
        }
    }
}

/// A ribbon control uses the shared tool presentation and icon set.
fn tool(icon: ToolIcon, label: &'static str, description: &'static str, enabled: bool) -> RibbonTool<'static> {
    RibbonTool { icon, label, description, shortcut: "", enabled, active: false, menu_check: None }
}

/// Schedule at most one GPU frame while retaining the newest camera and canvas aspect.
fn update_trace(
    ctx: &Context,
    worker: &mut raytrace::Worker,
    trace: &mut LiveTrace,
    host: HostId,
    request: Request,
    project: Arc<Project>,
    now: Instant,
) {
    let Some(settle) = trace.settle_remaining(now) else { return };
    if !settle.is_zero() {
        ctx.request_repaint_after(settle);
        return;
    }
    if worker.request().is_some() {
        ctx.request_repaint_after(Duration::from_millis(40));
        return;
    }
    let Some((pose, _)) = trace.pose else { return };
    match worker.start(ctx, host, request, project, pose) {
        Ok(()) => {
            trace.submitted = Some(pose);
            trace.pending = true;
        }
        Err(error) => {
            trace.error = Some(error);
            ctx.request_repaint();
        }
    }
}

/// Read pointer motion from this canvas only. Left drag orbits, middle drag pans.
fn navigate(ui: &Ui, canvas: Rect, navigator: Rect, camera: &mut Camera) -> bool {
    if !ui.is_enabled() {
        return false;
    }
    let response = ui.interact(canvas, ui.scope_id().with("orbit_board"), egui::Sense::drag());
    let (origin, hovered, scroll, button_held) = ui.input(|input| {
        (
            input.pointer.press_origin(),
            input.pointer.hover_pos(),
            input.smooth_scroll_delta.y,
            input.pointer.primary_down() || input.pointer.middle_down(),
        )
    });
    if origin.is_some_and(|p| !navigator.contains(p)) {
        let d = response.drag_delta();
        if response.dragged_by(egui::PointerButton::Primary) {
            camera.orbit(f64::from(d.x), f64::from(d.y));
        } else if response.dragged_by(egui::PointerButton::Middle) {
            camera.pan(f64::from(d.x), f64::from(d.y), f64::from(canvas.height()));
        }
    }
    if response.contains_pointer() && hovered.is_some_and(|p| !navigator.contains(p)) && scroll != 0.0 {
        camera.zoom_at((-f64::from(scroll) * 0.003).exp(), None);
    }
    button_held && response.is_pointer_button_down_on() && origin.is_some_and(|point| !navigator.contains(point))
}

/// Match the shared compass gesture by reversing when the selected axis already faces the camera.
fn snap_view(camera: &mut Camera, axis: &str) {
    let (view, opposite) = match axis {
        "x" => (StandardView::Right, StandardView::Left),
        "-x" => (StandardView::Left, StandardView::Right),
        "y" => (StandardView::Back, StandardView::Front),
        "-y" => (StandardView::Front, StandardView::Back),
        "z" => (StandardView::Top, StandardView::Bottom),
        "-z" => (StandardView::Bottom, StandardView::Top),
        _ => return,
    };
    camera.set_view(if camera.aligned_standard_view(1e-5) == Some(view) { opposite } else { view });
}

/// Pass the shared camera's basis to the toolkit without a renderer dependency in chrome.
fn array(v: solidcore_record::Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

/// Step the harness until the current revision's preview result arrives or the wait expires.
#[cfg(test)]
pub(crate) fn wait_for_preview(h: &mut egui_kittest::Harness<'_, PcbApp>) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        h.step();
        let app = h.state();
        if let Some(state) = &app.views.active.render
            && state.request.is_some_and(|request| request.revision == app.session.revision())
            && (state.error.is_some() || state.mesh.is_some())
        {
            break;
        }
        assert!(Instant::now() < until, "3D preview did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click, harness, queue_click};
    use egui_kittest::kittest::Queryable;
    use pcbcraft_engine::{
        Session,
        board::{drawing::ADD_DRAWING, setup::LayerPurpose},
    };
    use serde_json::json;

    /// Add an outline contour through the same revision-checked command used by the PCB editor.
    fn outline(session: &mut Session, points: [[i64; 2]; 4]) {
        let project = session.project().unwrap();
        let board = project.board_view(project.board_target().unwrap()).unwrap();
        let layer = board.layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let points = points.map(|[x_nm, y_nm]| json!({"x_nm": x_nm, "y_nm": y_nm}));
        session
            .apply(ADD_DRAWING, &json!({"layer":layer, "points":points, "closed":true, "stroke_nm":250_000, "expected_revision":session.revision()}))
            .unwrap();
    }

    #[test]
    fn live_tracing_waits_through_a_drag_and_restarts_the_delay_on_release() {
        let mut trace = LiveTrace { enabled: true, ..LiveTrace::default() };
        let started = Instant::now();
        let first = raytrace::Pose { camera: Camera::default(), aspect: 1.5 };
        assert!(trace.observe(first, false, started));
        assert_eq!(trace.settle_remaining(started), Some(LiveTrace::SETTLE));
        let first_ready = started + LiveTrace::SETTLE;
        assert_eq!(trace.settle_remaining(first_ready), Some(Duration::ZERO));
        trace.submitted = Some(first);
        trace.texture =
            Some(Context::default().load_texture("test frame", egui::ColorImage::filled([1, 1], egui::Color32::BLACK), Default::default()));
        trace.pending = true;

        let mut moved = first;
        moved.camera.orbit(25.0, -10.0);
        assert!(trace.observe(moved, true, first_ready));
        assert!(trace.submitted.is_none());
        assert!(trace.texture.is_none());
        trace.pending = false;
        let released = first_ready + Duration::from_secs(2);
        assert_eq!(trace.settle_remaining(released), None);
        assert!(trace.observe(moved, false, released));

        // Another zoom in the same burst starts a fresh delay from the latest camera.
        let zoomed = released + Duration::from_millis(400);
        moved.camera.zoom_at(0.9, None);
        assert!(trace.observe(moved, false, zoomed));
        assert_eq!(trace.settle_remaining(zoomed), Some(LiveTrace::SETTLE));
        let settled = zoomed + LiveTrace::SETTLE;
        assert_eq!(trace.settle_remaining(settled), Some(Duration::ZERO));
        trace.submitted = Some(moved);
        assert_eq!(trace.settle_remaining(settled), None);
        trace.clear_frames();
        assert!(trace.enabled);
        assert_eq!(trace.settle_remaining(settled), None);
    }

    /// Build an offset board with a cutout and a drill, then exercise opening, Browse and undo.
    #[test]
    fn workspace_previews_the_board_and_recovers_after_undo() {
        let mut h = harness();
        h.state_mut().run("project.new", &json!({"create_board":true, "name":"Render test"}));
        {
            let session = &mut h.state_mut().session;
            session.apply("pcb.initialize_board_setup", &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
            outline(session, [[5_000_000, -3_000_000], [55_000_000, -3_000_000], [55_000_000, 27_000_000], [5_000_000, 27_000_000]]);
            outline(session, [[30_000_000, 8_000_000], [38_000_000, 8_000_000], [38_000_000, 14_000_000], [30_000_000, 14_000_000]]);
            session
                .apply(
                    "pcb.add_via",
                    &json!({"at":{"x_nm":12_000_001,"y_nm":6_000_003},"via_diameter_nm":4_000_000,"via_drill_nm":3_000_000,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
        h.run();
        let before = h.state().session.inspect();
        let target = h.state().session.project().unwrap().board_target().unwrap();

        click(&mut h, "Choose workspace");
        // The preview spinner repaints until the worker finishes. Step through the click
        // in the bounded worker wait instead of asking the harness to settle first.
        queue_click(&mut h, "3D render");
        wait_for_preview(&mut h);
        let state = h.state().views.active.render.as_ref().unwrap();
        assert_eq!(state.target, Some(target));
        assert_eq!(h.state().workspace_key(), "render");
        assert!(h.state().views.active.input(&h.state().session).is_none());
        let (bounds, thickness, _) = state.mesh.as_ref().unwrap();
        let extent = bounds.max - bounds.min;
        assert!((extent.x - 50.0).abs() < 0.001 && (extent.y - 30.0).abs() < 0.001 && (extent.z - 1.6).abs() < 0.001);
        assert_eq!(thickness.nm(), 1_600_000);

        click(&mut h, "Choose a board in this project");
        assert!(matches!(h.state().dialog, Dialog::BrowseRenderBoard(_)));
        h.get_by_label("Browse Boards");
        h.get_by_label("Board");
        click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        let valid = h.state().views.active.render.as_ref().unwrap();
        let key = valid.key;
        let camera = valid.camera;
        click(&mut h, "Choose a board in this project");
        click(&mut h, "Open");
        let valid = h.state().views.active.render.as_ref().unwrap();
        assert_eq!(valid.key, key);
        assert_eq!(valid.camera, camera);

        // The engine supports one physical board per project. A recoverable error on that
        // board makes confirmation observable even when the selected target stays the same.
        let failed = h.state_mut().views.active.render.as_mut().unwrap();
        failed.mesh = None;
        failed.error = Some("Preview worker unavailable".into());
        click(&mut h, "Choose a board in this project");
        queue_click(&mut h, "Open");
        wait_for_preview(&mut h);
        assert!(matches!(h.state().dialog, Dialog::None));
        let rebuilt = h.state().views.active.render.as_ref().unwrap();
        assert_eq!(rebuilt.target, Some(target));
        assert_eq!(rebuilt.camera, camera);
        assert!(rebuilt.mesh.is_some());
        assert!(rebuilt.error.is_none());
        assert_eq!(rebuilt.key, key + 1);
        assert_eq!(h.state().session.inspect(), before);

        click(&mut h, "Assistant");
        h.get_by_role(egui::accesskit::Role::MultilineTextInput);
        click(&mut h, "Close Assistant");

        let camera = {
            let state = h.state_mut().views.active.render.as_mut().unwrap();
            state.camera.set_view(StandardView::Bottom);
            state.camera
        };
        outline(&mut h.state_mut().session, [[50_000_000, 20_000_000], [60_000_000, 20_000_000], [60_000_000, 30_000_000], [50_000_000, 30_000_000]]);
        wait_for_preview(&mut h);
        assert!(
            h.state()
                .views
                .active
                .render
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Board Outline contours cross or touch"))
        );
        h.state_mut().session.execute("edit.undo", &serde_json::Value::Null).unwrap();
        wait_for_preview(&mut h);
        let state = h.state().views.active.render.as_ref().unwrap();
        assert!(state.mesh.is_some());
        assert_eq!(state.camera, camera);
    }
}
