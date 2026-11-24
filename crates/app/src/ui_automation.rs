//! Opt-in, window-addressed automation through ordinary egui input and Craft targets.
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use pcbcraft_chrome::automation::Target;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{PcbApp, ViewId, editor_layout::HostId, editor_windows::viewport};

#[cfg(not(target_arch = "wasm32"))]
mod detached;
#[cfg(test)]
mod tests;
mod window;

const SCHEMA: &str = "pcbcraft-ui/1";

/// Endpoint/Assistant-turn authority, retained after a queue acknowledgement has been sent.
#[derive(Clone, Debug)]
pub(super) struct Owner {
    pub(super) port: u16,
    pub(super) generation: u64,
    pub(super) chat: bool,
    pub(super) stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The ordinary input destination, before a step and again after its own accepted UI pass.
#[derive(Clone, Debug, PartialEq)]
struct Authority {
    target: Option<crate::editor_view::DocumentTarget>,
    workspace: crate::Workspace,
    definition: Option<u64>,
    focused: ViewId,
    visible: Vec<ViewId>,
    revision: u64,
    opening: u64,
    home: bool,
    dialog: std::mem::Discriminant<crate::Dialog>,
}

/// One visible canvas's model-to-window mapping. Pan, zoom, flipping or a pane resize
/// changes it, which makes coordinates projected from an older paint stale.
#[derive(Clone, Debug, PartialEq)]
struct Canvas {
    view: ViewId,
    rect: Rect,
    origin: Pos2,
    unit: f32,
    mirrored: bool,
    /// Floating cards over the canvas; definition panes have none.
    cards: Vec<Rect>,
}

#[derive(Debug)]
struct Snapshot {
    layout: u64,
    rect: Rect,
    focused: bool,
    modifiers: Modifiers,
    /// A pointer button was held when this window last painted.
    pointer_down: bool,
    scale: f32,
    targets: Vec<Target>,
    authority: Authority,
    canvases: Vec<Canvas>,
    /// Popup, menu, window and tooltip areas painted over the editor background.
    overlays: Vec<Rect>,
    window: window::Measured,
}

/// The painted mapping and layers a coordinate press was admitted against.
#[derive(Debug)]
struct CoordinateGuard {
    canvases: Vec<Canvas>,
    overlays: Vec<Rect>,
}

impl CoordinateGuard {
    /// Compare the layers at the press point, allowing unrelated overlays to change elsewhere.
    fn overlay_changed_at(&self, snapshot: &Snapshot, at: Pos2) -> bool {
        self.overlays.iter().filter(|rect| rect.contains(at)).ne(snapshot.overlays.iter().filter(|rect| rect.contains(at)))
    }
}

#[derive(Debug)]
struct Job {
    ticket: u64,
    host: HostId,
    owner: Option<Owner>,
    authority: Authority,
    deadline: Instant,
    frames: VecDeque<Vec<Event>>,
    modifiers: Modifiers,
    issued: bool,
    held: Option<(PointerButton, Pos2)>,
    rect: Rect,
    scale: f32,
    restore: Modifiers,
    guards: Vec<(String, Pos2)>,
    coordinates: Option<CoordinateGuard>,
}

/// At most one bounded input operation and one completion record per editor session.
#[derive(Debug, Default)]
pub(super) struct Automation {
    pub(super) observing: bool,
    snapshots: BTreeMap<HostId, Snapshot>,
    next_ticket: u64,
    next_layout: u64,
    job: Option<Job>,
    last: Value,
    release: Option<(HostId, Option<PointerButton>, Modifiers)>,
    /// The framework retains raw input over logic-only callbacks. Remove this exact owned
    /// batch before reinjection, preserving any newly appended physical events.
    injected: Option<(egui::ViewportId, Vec<Event>, bool)>,
    /// Input already entered egui on this pass; cancel it after that pass, rather than clearing
    /// a gesture before the same pass can recreate it from its pending button press.
    cancel_after_paint: Option<&'static str>,
    resizes: BTreeMap<HostId, window::Resize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    window: u64,
    #[serde(default)]
    prefix: String,
    handle: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    view: u64,
    at: ModelPoint,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelPoint {
    x_nm: i64,
    y_nm: i64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Point {
    Named(Named),
    Position(Position),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    handle: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Button {
    #[default]
    Left,
    Right,
    Middle,
}
impl Button {
    /// Translate the public vocabulary to ordinary egui pointer buttons.
    fn egui(self) -> PointerButton {
        match self {
            Self::Left => PointerButton::Primary,
            Self::Right => PointerButton::Secondary,
            Self::Middle => PointerButton::Middle,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Keys {
    ctrl: bool,
    shift: bool,
    alt: bool,
    command: bool,
}
impl Keys {
    /// Preserve platform shortcut semantics: Command means Ctrl except on macOS.
    fn egui(self) -> Modifiers {
        let mac = cfg!(target_os = "macos");
        Modifiers {
            ctrl: self.ctrl || (self.command && !mac),
            shift: self.shift,
            alt: self.alt,
            mac_cmd: self.command && mac,
            command: self.command || (self.ctrl && !mac),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Move {
        target: Point,
    },
    Click {
        target: Point,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Keys,
    },
    Drag {
        from: Point,
        to: Point,
        steps: u16,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Keys,
    },
    Key {
        key: String,
        #[serde(default)]
        modifiers: Keys,
    },
    Text {
        text: String,
    },
    Scroll {
        target: Point,
        delta_x: f32,
        delta_y: f32,
        #[serde(default)]
        modifiers: Keys,
    },
}

impl Snapshot {
    /// Resolve only a unique currently visible handle, or finite in-window logical coordinates.
    fn point(&self, point: Point) -> Result<Pos2, String> {
        let point = match point {
            Point::Position(p) => egui::pos2(p.x, p.y),
            Point::Named(name) => {
                let mut matches = self.targets.iter().filter(|target| target.name == name.handle);
                let target = matches.next().ok_or_else(|| format!("No visible UI handle `{}` in this window", name.handle))?;
                if matches.next().is_some() {
                    return Err(format!("UI handle `{}` is ambiguous", name.handle));
                }
                if !target.enabled {
                    return Err(format!("UI handle `{}` is disabled", name.handle));
                }
                target.rect.center()
            }
        };
        if !point.is_finite() || !self.rect.contains(point) {
            return Err("Input coordinates must be finite logical points inside the addressed window".into());
        }
        Ok(point)
    }
}

impl Automation {
    /// Attach the originating endpoint only to a newly accepted job; inspection cannot reassign it.
    pub(super) fn authorize(&mut self, owner: Owner) {
        if let Some(job) = &mut self.job {
            job.owner.get_or_insert(owner);
        }
    }

    /// Another command must not move the input destination between injection and UI handling.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Only an injected held button (or its pending release) owns an editor pointer gesture.
    /// An idle or accepted-but-unstarted operation must not clear the person's previews.
    fn owns_pointer_gesture(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.held.is_some()) || self.release.as_ref().is_some_and(|(_, button, _)| button.is_some())
    }

    /// Retain a failure and schedule an off-window button release, which cannot activate a control.
    fn cancel(&mut self, reason: &str) {
        self.cancel_after_paint = None;
        if let Some(job) = self.job.take() {
            self.release = Some((job.host, job.held.map(|(button, _)| button), job.restore));
            self.last = json!({"ticket":job.ticket,"state":"cancelled","reason":reason});
        }
    }

    /// Return bounded queue state; clients must wait for completion before another input call.
    fn status(&self) -> Value {
        self.job.as_ref().map_or_else(|| self.last.clone(), |job| json!({"ticket":job.ticket,"state":"pending","remaining_frames":job.frames.len()}))
    }
}

impl PcbApp {
    /// Capture view/opening/revision and modal ownership without mutating presentation or data.
    fn automation_authority(&self, host: HostId) -> Option<Authority> {
        let host = self.views.hosts.get(&host)?;
        let view = self.views.view(host.focused)?;
        Some(Authority {
            target: view.target(),
            workspace: view.workspace,
            definition: view.definition,
            focused: host.focused,
            visible: host.layout.visible_views(),
            revision: self.session.revision(),
            opening: self.session.history_root_revision(),
            home: self.home,
            dialog: std::mem::discriminant(&self.dialog),
        })
    }

    /// The camera that paints `view`: its own for schematic and PCB panes, or its definition
    /// presentation's. Library browsing panes and unavailable-document placeholders, which keep
    /// their last camera, have no model canvas.
    fn automation_camera(&self, view: ViewId) -> Option<&crate::canvas_camera::Camera> {
        let pane = self.views.view(view).filter(|pane| pane.library.is_none() && !pane.unavailable())?;
        match pane.definition {
            Some(definition) => self.library_editors.view_camera(definition, view),
            None => Some(&pane.camera),
        }
    }

    /// The model-to-window mapping of visible canvases with up-to-date paints.
    fn automation_canvases(&self, host: HostId) -> Vec<Canvas> {
        let Some(host) = self.views.hosts.get(&host) else { return Vec::new() };
        host.layout
            .visible_views()
            .into_iter()
            .filter_map(|view| {
                let camera = self.automation_camera(view).filter(|camera| !camera.navigation_pending())?;
                let rect = camera.painted_rect()?;
                let pane = self.views.view(view)?;
                let cards = if pane.definition.is_some() { Vec::new() } else { pane.canvas_cards.clone() };
                Some(Canvas {
                    view,
                    rect,
                    origin: camera.screen(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }, rect),
                    unit: camera.unit_step(),
                    mirrored: camera.mirrored(),
                    cards,
                })
            })
            .collect()
    }

    /// Whether this host still matches its last snapshot: the same input destination and the
    /// same canvas mappings. Navigation or a pending resize makes its coordinates stale.
    fn automation_current(&self, host: HostId, snapshot: &Snapshot) -> bool {
        self.automation_authority(host).as_ref() == Some(&snapshot.authority)
            && self.automation_canvases(host) == snapshot.canvases
            && !self.automation.resize_pending(host)
    }

    /// Cancel queued input now, or after the pass that already received its events.
    fn automation_cancel(&mut self, reason: &'static str) {
        if self.automation.job.as_ref().is_some_and(|job| job.issued) {
            self.automation.cancel_after_paint = Some(reason);
        } else {
            let owned = self.automation.owns_pointer_gesture();
            self.automation.cancel(reason);
            if owned {
                self.views.cancel_gestures();
            }
        }
    }

    /// Project a model point in an explicitly addressed view to window-local logical points
    /// from the last accepted paint, without changing navigation, selection or the project.
    /// `uncovered` means the point lies in the view's uncovered canvas and no popup, menu or
    /// window area covers it; covered and off-canvas points still return their coordinates.
    fn automation_project(&mut self, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
        let query: Projection = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
        // Pins and pads can sit up to the pin offset beyond the largest placement center.
        let max = pcbcraft_engine::schematic::MAX_CENTER_NM + pcbcraft_engine::schematic::MAX_PIN_OFFSET_NM;
        if !(-max..=max).contains(&query.at.x_nm) || !(-max..=max).contains(&query.at.y_nm) {
            return Err(format!("Model coordinates must be within ±{max} nm"));
        }
        let view = ViewId(query.view);
        let pane = self.views.view(view).ok_or("Unknown view; it may have been closed")?;
        if pane.library.is_some() {
            return Err("The addressed view shows Parts Library browsing, which has no model canvas".into());
        }
        if pane.unavailable() {
            return Err("The addressed view's document is unavailable, so it has no model canvas".into());
        }
        let (&host, _) =
            self.views.hosts.iter().find(|(_, host)| host.layout.visible_views().contains(&view)).ok_or("The addressed view is not visible")?;
        if self.home && host == self.views.main {
            return Err("Home hides the workspace; return to it before projecting".into());
        }
        self.repaint_editors(ctx);
        let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is not ready; wait for a paint and retry")?;
        if !self.automation_current(host, snapshot) {
            return Err("UI geometry is stale; wait for a paint and retry".into());
        }
        let canvas = snapshot
            .canvases
            .iter()
            .find(|canvas| canvas.view == view)
            .ok_or("The addressed view has not painted its canvas yet; wait for a paint and retry")?;
        let camera = self.automation_camera(view).ok_or("The addressed view has no model canvas")?;
        let at = camera.screen(pcbcraft_engine::schematic::Point { x_nm: query.at.x_nm, y_nm: query.at.y_nm }, canvas.rect);
        // Coverage is checked against the actual cards and overlays, not the largest clear
        // rectangle that the `canvas:` handle publishes, so clear corners beside a card count.
        let on_canvas = canvas.rect.contains(at);
        let enabled = snapshot.authority.dialog == std::mem::discriminant(&crate::Dialog::None);
        let uncovered = enabled && on_canvas && !canvas.cards.iter().chain(&snapshot.overlays).any(|cover| cover.contains(at));
        Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"view":view.0,"x":at.x,"y":at.y,
            "on_canvas":on_canvas,"uncovered":uncovered,"scale":snapshot.scale}))
    }

    /// Snapshot each native host separately after its accepted UI pass, including definition views.
    /// Logical coordinates belong to this window; another window can publish identical names.
    /// Retire injected button ownership only after its release frame has been accepted.
    pub(super) fn automation_painted(&mut self, ctx: &egui::Context) {
        if !self.automation.observing || ctx.will_discard() {
            return;
        }
        let host = self.views.host;
        let released = self.automation.injected.as_ref().is_some_and(|(id, _, releasing)| *id == viewport(host) && *releasing);
        if let Some(reason) = self.automation.cancel_after_paint
            && self.automation.job.as_ref().is_some_and(|job| job.host == host)
        {
            let owned = self.automation.owns_pointer_gesture();
            self.automation.cancel(&format!("{reason} after the in-flight UI pass"));
            if owned {
                self.views.cancel_gestures();
            }
        }
        if self.automation.injected.as_ref().is_some_and(|(id, _, _)| *id == viewport(host)) {
            self.automation.injected = None;
        }
        if released {
            self.automation.release = None;
        }
        let Some(authority) = self.automation_authority(host) else { return };
        let targets = pcbcraft_chrome::automation::targets(ctx);
        let rect = ctx.content_rect();
        let canvases = self.automation_canvases(host);
        let overlays: Vec<Rect> = ctx.memory(|memory| {
            memory
                .layer_ids()
                .filter(|layer| layer.order != egui::Order::Background && memory.areas().is_visible(layer))
                .filter_map(|layer| memory.area_rect(layer.id))
                .collect()
        });
        let window = self.automation_measure(ctx, host);
        let changed =
            self.automation.snapshots.get(&host).is_none_or(|s| {
                s.rect != rect || s.targets != targets || s.authority != authority || s.canvases != canvases || s.overlays != overlays
            });
        if changed {
            self.automation.next_layout = self.automation.next_layout.saturating_add(1);
        }
        let layout = self.automation.snapshots.get(&host).filter(|_| !changed).map_or(self.automation.next_layout, |s| s.layout);
        self.automation.snapshots.insert(
            host,
            Snapshot {
                layout,
                rect,
                focused: ctx.input(|i| i.focused),
                modifiers: ctx.input(|i| i.modifiers),
                pointer_down: ctx.input(|i| i.pointer.any_down()),
                scale: ctx.pixels_per_point(),
                targets,
                authority: authority.clone(),
                canvases,
                overlays,
                window,
            },
        );
        if let Some(job) = &mut self.automation.job
            && job.host == host
            && job.issued
        {
            if let Some(events) = job.frames.pop_front() {
                for event in events {
                    if let Event::PointerButton { button, pressed: false, .. } = event
                        && job.held.is_some_and(|(held, _)| held == button)
                    {
                        job.held = None;
                    }
                }
            }
            job.issued = false;
            job.authority = authority;
            if job.frames.is_empty() {
                self.automation.last = json!({"ticket":job.ticket,"state":"completed","revision":self.session.revision()});
                self.automation.job = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        self.automation.snapshots.retain(|host, _| self.views.hosts.contains_key(host));
        // A closed window cannot receive its off-window release; its gestures were already cancelled.
        if self.automation.release.is_some_and(|(host, _, _)| !self.views.hosts.contains_key(&host)) {
            self.automation.release = None;
        }
    }

    /// Dispatch live-only UI commands. Querying opts into target collection on subsequent paints.
    /// Input acknowledges queue acceptance; ui.inspect reports actual accepted-frame completion.
    pub(super) fn automation_command(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<Value, String> {
        self.automation.observing = true;
        if matches!(id, "ui.inspect" | "ui.cancel") {
            if !params.is_null() && params != &json!({}) {
                return Err(format!("{id} takes no parameters"));
            }
            if id == "ui.cancel" {
                self.automation_cancel("Cancelled by client");
            }
            self.repaint_editors(ctx);
            return Ok(
                json!({"schema":SCHEMA,"operation":self.automation.status(),"coordinates":"window-local logical points","ready_windows":self.automation.snapshots.keys().map(|h|h.0).collect::<Vec<_>>()}),
            );
        }
        if id == "ui.project" {
            return self.automation_project(ctx, params);
        }
        if matches!(id, "ui.window" | "ui.resize") {
            return self.automation_window(ctx, id, params);
        }
        if matches!(id, "ui.handles" | "ui.at") {
            let allowed: &[&str] = if id == "ui.at" { &["window", "handle"] } else { &["window", "prefix"] };
            if params.as_object().is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str()))) {
                return Err(format!("{id} takes only {}", allowed.join(", ")));
            }
            let query: Query = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
            let host = HostId(query.window);
            if !self.views.hosts.contains_key(&host) {
                return Err("Unknown editor window".into());
            }
            self.repaint_editors(ctx);
            let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is not ready; wait for a paint and retry")?;
            if !self.automation_current(host, snapshot) {
                return Err("UI geometry is stale; wait for a paint and retry".into());
            }
            if id == "ui.at" {
                let point = snapshot.point(Point::Named(Named { handle: query.handle.ok_or("ui.at needs handle")? }))?;
                return Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"x":point.x,"y":point.y}));
            }
            if query.handle.is_some() {
                return Err("ui.handles takes prefix, not handle".into());
            }
            let targets = snapshot
                .targets
                .iter()
                .filter(|t| t.name.starts_with(&query.prefix))
                .map(|t| {
                    json!({"name":t.name,"enabled":t.enabled,
                "x":t.rect.center().x,"y":t.rect.center().y,"rect":[t.rect.min.x,t.rect.min.y,t.rect.width(),t.rect.height()]})
                })
                .collect::<Vec<_>>();
            return Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"focused":snapshot.focused,"handles":targets}));
        }
        let name = id.strip_prefix("ui.").ok_or("Unknown UI command")?;
        let mut object = params.as_object().ok_or("UI input takes an object")?.clone();
        let host = HostId(object.remove("window").and_then(|v| v.as_u64()).ok_or("UI input needs an integer window ID")?);
        if host != HostId::ROOT && ctx.embed_viewports() {
            return Err("Detached editors are embedded in the main window on this platform; send input to window 0".into());
        }
        let layout = object.remove("layout").map(|v| v.as_u64().ok_or("layout must be an integer")).transpose()?;
        if object.contains_key("action") {
            return Err("Use the command ID to choose the input action".into());
        }
        object.insert("action".into(), json!(name));
        let action: Action = serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())?;
        if self.automation.job.is_some() || self.automation.release.is_some() {
            return Err("UI input is busy; wait for ui.inspect or cancel the pending operation".into());
        }
        if !self.views.hosts.contains_key(&host) {
            return Err("Unknown editor window".into());
        }
        let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is unavailable; call ui.handles and wait for a paint")?;
        if !self.automation_current(host, snapshot) || layout.is_some_and(|layout| layout != snapshot.layout) {
            return Err("UI geometry is stale; inspect handles again".into());
        }
        if !snapshot.focused {
            return Err("Focus the addressed editor window before sending UI input".into());
        }
        if snapshot.pointer_down || ctx.input(|input| input.pointer.any_down()) {
            return Err("Release held pointer buttons before sending UI input".into());
        }
        if self.files.busy() {
            return Err("Wait for the current file operation before sending UI input".into());
        }
        let guards = action.guards(snapshot)?;
        let coordinates =
            action.coordinate_press().then(|| CoordinateGuard { canvases: snapshot.canvases.clone(), overlays: snapshot.overlays.clone() });
        let (frames, modifiers) = action.frames(snapshot)?;
        self.automation.next_ticket = self.automation.next_ticket.checked_add(1).ok_or("UI ticket limit reached")?;
        let ticket = self.automation.next_ticket;
        self.automation.job = Some(Job {
            ticket,
            host,
            owner: None,
            authority: snapshot.authority.clone(),
            deadline: Instant::now() + Duration::from_secs(5),
            frames: frames.into(),
            modifiers,
            issued: false,
            held: None,
            rect: snapshot.rect,
            scale: snapshot.scale,
            restore: snapshot.modifiers,
            guards,
            coordinates,
        });
        ctx.request_repaint_of(viewport(host));
        Ok(json!({"schema":SCHEMA,"ticket":ticket,"state":"queued"}))
    }

    /// Inject one bounded frame before egui processes input. Keep it across logic-only callbacks;
    /// only an accepted host paint advances the queue. Physical input always takes precedence.
    /// This is eframe's hook for window 0; detached windows use the plugin in `detached`.
    pub(super) fn automation_input(&mut self, ctx: &egui::Context, input: &mut RawInput) {
        pcbcraft_chrome::automation::mark_synthetic_input(ctx, false);
        if let Some((id, events, _)) = &self.automation.injected
            && *id == input.viewport_id
        {
            if let Some(start) = input.events.windows(events.len()).position(|window| window == events) {
                input.events.drain(start..start + events.len());
            }
            self.automation.injected = None;
        }
        let checks = self.automation.job.as_ref().filter(|job| input.viewport_id == viewport(job.host)).map(|job| Checks {
            rect: job.rect,
            scale: job.scale,
            held: job.held.is_some(),
            pointer_down: ctx.input(|input| input.pointer.any_down()),
        });
        if let Some(reason) = self.automation_invalid().or_else(|| checks.and_then(|checks| interrupted(input, ctx.pixels_per_point(), checks))) {
            self.automation_stop(reason);
        }
        let (release, frame) = self.automation_events(input.viewport_id);
        if release.is_empty() && frame.is_empty() {
            return;
        }
        pcbcraft_chrome::automation::mark_synthetic_input(ctx, true);
        input.events.splice(0..0, release.iter().cloned());
        input.events.extend_from_slice(&frame);
        self.automation_issued(ctx, input.viewport_id, release, frame);
    }

    /// Application-state reasons that end the queued job before its next frame, in any window:
    /// a revoked connection, the deadline, a changed destination, a file job, or a moved target.
    fn automation_invalid(&self) -> Option<&'static str> {
        let job = self.automation.job.as_ref()?;
        let owner_valid = job.owner.as_ref().is_some_and(|owner| {
            if owner.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return false;
            }
            if owner.chat {
                self.assistant.turn_open() && self.assistant.generation() == Some(owner.generation)
            } else {
                self.live_control.as_ref().is_some_and(|live| live.port() == owner.port && live.current() == owner.generation)
            }
        });
        let press = job.frames.front().and_then(|events| {
            events.iter().find_map(|event| match event {
                Event::PointerButton { pos, pressed: true, .. } => Some(*pos),
                _ => None,
            })
        });
        if !owner_valid {
            Some("Automation connection or Assistant turn ended")
        } else if Instant::now() >= job.deadline {
            Some("UI input timed out before completion")
        } else if self.automation_authority(job.host).as_ref() != Some(&job.authority) {
            Some("Project or input destination changed")
        } else if self.files.busy() {
            Some("A file operation owns the editor")
        } else if job.frames.front().is_some_and(|events| events.iter().any(|e| matches!(e, Event::PointerButton { .. })))
            && self.automation.snapshots.get(&job.host).is_none_or(|snapshot| {
                job.guards.iter().any(|(name, point)| snapshot.point(Point::Named(Named { handle: name.clone() })).ok() != Some(*point))
            })
        {
            Some("Named input target moved, became disabled or was covered")
        } else if let Some(guard) = &job.coordinates
            && let Some(at) = press
            && (guard.canvases != self.automation_canvases(job.host)
                || self.automation.snapshots.get(&job.host).is_none_or(|snapshot| guard.overlay_changed_at(snapshot, at)))
        {
            Some("Canvas mapping or overlay changed under a coordinate target")
        } else {
            None
        }
    }

    /// Cancel the queued job now, ending any gesture its injected press started.
    fn automation_stop(&mut self, reason: &str) {
        let owned = self.automation.owns_pointer_gesture();
        self.automation.cancel(reason);
        if owned {
            self.views.cancel_gestures();
        }
    }

    /// The events one pass of `viewport` receives: an off-window release ahead of physical
    /// input after a cancellation, or the queued job's next frame after it.
    fn automation_events(&self, viewport_id: egui::ViewportId) -> (Vec<Event>, Vec<Event>) {
        let mut release = Vec::new();
        if let Some((host, button, modifiers)) = self.automation.release
            && viewport_id == viewport(host)
        {
            let pos = egui::pos2(-1000.0, -1000.0);
            release.push(Event::ModifiersChanged(modifiers));
            if let Some(button) = button {
                release.extend([Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: false, modifiers }, Event::PointerGone]);
            }
        }
        let mut frame = Vec::new();
        if let Some(job) = &self.automation.job
            && viewport_id == viewport(job.host)
            && let Some(events) = job.frames.front()
        {
            frame.push(Event::ModifiersChanged(job.modifiers));
            frame.extend_from_slice(events);
        }
        (release, frame)
    }

    /// Record events that entered `viewport`'s pass: the accepted paint retires a release or
    /// advances the job, and a press keeps its button owned until that paint.
    fn automation_issued(&mut self, ctx: &egui::Context, viewport_id: egui::ViewportId, release: Vec<Event>, frame: Vec<Event>) {
        if !release.is_empty() {
            self.automation.injected = Some((viewport_id, release, true));
            ctx.request_repaint_of(viewport_id);
        }
        if let Some(job) = self.automation.job.as_mut().filter(|_| !frame.is_empty()) {
            for event in &frame {
                if let Event::PointerButton { pos, button, pressed: true, .. } = event {
                    // Retain ownership across an unconsumed release; the accepted paint clears it.
                    job.held = Some((*button, *pos));
                }
            }
            job.issued = true;
            self.automation.injected = Some((viewport_id, frame, false));
        }
    }
}

/// Facts about a queued job that its own window's raw input is checked against.
#[derive(Clone, Copy, Debug)]
struct Checks {
    rect: Rect,
    scale: f32,
    held: bool,
    /// A physical button is down in that window.
    pointer_down: bool,
}

/// Raw-input reasons that end a queued job in its own window's pass: lost focus or
/// visibility, a size or scale change, physical input, or a physically held button.
fn interrupted(input: &RawInput, scale: f32, checks: Checks) -> Option<&'static str> {
    if !input.focused || input.viewport().minimized == Some(true) || input.viewport().occluded == Some(true) {
        Some("Editor window lost focus or visibility")
    } else if input.screen_rect.is_some_and(|rect| rect != checks.rect) || scale != checks.scale {
        Some("Window size or scale changed")
    } else if input.events.iter().any(physical_input) {
        Some("Physical input interrupted automation")
    } else if !checks.held && checks.pointer_down {
        Some("A held physical pointer button interrupted automation")
    } else {
        None
    }
}

/// Events produced by a person cancel a queued synthetic gesture instead of mixing owners.
fn physical_input(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerMoved(_)
            | Event::MouseMoved(_)
            | Event::PointerGone
            | Event::PointerButton { .. }
            | Event::Key { .. }
            | Event::ModifiersChanged(_)
            | Event::Text(_)
            | Event::Paste(_)
            | Event::Ime(_)
            | Event::MouseWheel { .. }
            | Event::Touch { .. }
            | Event::Zoom(_)
            | Event::Rotate(_)
    )
}

impl Action {
    /// Whether a click or drag uses coordinates that need to be checked before its press.
    fn coordinate_press(&self) -> bool {
        match self {
            Self::Click { target, .. } => matches!(target, Point::Position(_)),
            Self::Drag { from, to, .. } => matches!(from, Point::Position(_)) || matches!(to, Point::Position(_)),
            _ => false,
        }
    }

    /// Preserve named destinations across hover/layout transitions before a button event.
    fn guards(&self, snapshot: &Snapshot) -> Result<Vec<(String, Pos2)>, String> {
        let points = match self {
            Self::Move { target } | Self::Click { target, .. } | Self::Scroll { target, .. } => vec![target],
            Self::Drag { from, to, .. } => vec![from, to],
            _ => Vec::new(),
        };
        points
            .into_iter()
            .filter_map(|point| match point {
                Point::Named(name) => Some(snapshot.point(Point::Named(Named { handle: name.handle.clone() })).map(|p| (name.handle.clone(), p))),
                _ => None,
            })
            .collect()
    }
    /// Validate the entire action before queuing any events. Drags have at most 120 moves;
    /// text is limited to 4096 bytes and wheel deltas to 10,000 logical points per axis.
    fn frames(self, snapshot: &Snapshot) -> Result<(Vec<Vec<Event>>, Modifiers), String> {
        let mut frames = Vec::new();
        let mut keys = Modifiers::NONE;
        let button = |pos, button, pressed, modifiers| Event::PointerButton { pos, button, pressed, modifiers };
        match self {
            Self::Move { target } => frames.push(vec![Event::PointerMoved(snapshot.point(target)?)]),
            Self::Click { target, button: b, modifiers } => {
                keys = modifiers.egui();
                let pos = snapshot.point(target)?;
                let b = b.egui();
                frames.push(vec![Event::PointerMoved(pos)]);
                frames.push(vec![button(pos, b, true, keys), button(pos, b, false, keys)]);
            }
            Self::Drag { from, to, steps, button: b, modifiers } => {
                if !(2..=120).contains(&steps) {
                    return Err("Drag steps must be between 2 and 120".into());
                }
                keys = modifiers.egui();
                let from = snapshot.point(from)?;
                let to = snapshot.point(to)?;
                let b = b.egui();
                frames.push(vec![Event::PointerMoved(from)]);
                frames.push(vec![button(from, b, true, keys)]);
                for step in 1..=steps {
                    frames.push(vec![Event::PointerMoved(from.lerp(to, f32::from(step) / f32::from(steps)))]);
                }
                frames.push(vec![button(to, b, false, keys)]);
            }
            Self::Key { key, modifiers } => {
                let key = egui::Key::from_name(&key).ok_or("Unknown key; use an egui key name such as W, Escape, Enter or PageDown")?;
                keys = modifiers.egui();
                frames.push(vec![
                    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: keys },
                    Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: keys },
                ]);
            }
            Self::Text { text } => {
                if text.is_empty() || text.len() > 4096 || text.chars().any(|c| c.is_control()) {
                    return Err("Text needs 1–4096 UTF-8 bytes without control characters; use ui.key for Enter or Tab".into());
                }
                frames.push(vec![Event::Text(text)]);
            }
            Self::Scroll { target, delta_x, delta_y, modifiers } => {
                if !delta_x.is_finite() || !delta_y.is_finite() || delta_x.abs() > 10000.0 || delta_y.abs() > 10000.0 {
                    return Err("Wheel deltas must be finite and at most 10000 logical points".into());
                }
                keys = modifiers.egui();
                frames.push(vec![
                    Event::PointerMoved(snapshot.point(target)?),
                    Event::MouseWheel {
                        source: egui::MouseWheelSource::Unknown,
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(delta_x, delta_y),
                        modifiers: keys,
                    },
                ]);
            }
        }
        if keys != snapshot.modifiers {
            frames.push(vec![Event::ModifiersChanged(snapshot.modifiers)]);
        }
        Ok((frames, keys))
    }
}

/// Check UI command ids and parameter names before the input queue sees them. Null uses
/// the existing empty-map convention; required keys still apply, and other non-object forms
/// are refused. Values are checked by the handler.
pub(super) fn check_command(id: &str, params: &Value) -> Result<(), pcbcraft_mcp::Failure> {
    use pcbcraft_engine::error_codes::ErrorCode;
    use pcbcraft_mcp::Failure;
    let catalog = catalog();
    let command = catalog
        .iter()
        .find(|command| command["id"] == id)
        .ok_or_else(|| Failure::new(ErrorCode::UnknownCommand, format!("unknown command `{id}`")))?;
    let empty = serde_json::Map::new();
    let given = match params {
        Value::Null => &empty,
        Value::Object(object) => object,
        _ => return Err(Failure::new(ErrorCode::InvalidParams, format!("`{id}` takes an object of parameters"))),
    };
    let schema = &command["params"];
    if let Some(extra) = given.keys().find(|key| schema["properties"].get(key.as_str()).is_none()) {
        return Err(Failure::new(ErrorCode::InvalidParams, format!("`{id}` has no parameter `{extra}`")));
    }
    if let Some(missing) = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|name| !given.contains_key(*name)) {
        return Err(Failure::new(ErrorCode::InvalidParams, format!("`{id}` needs `{missing}`")));
    }
    Ok(())
}

/// Discover the live automation contract beside the existing project/window command catalog.
pub(super) fn catalog() -> Vec<Value> {
    let point = json!({"oneOf":[{"type":"object","properties":{"handle":{"type":"string"}},"required":["handle"],"additionalProperties":false},
        {"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"],"additionalProperties":false}]});
    let keys = json!({"type":"object","properties":{"ctrl":{"type":"boolean"},"shift":{"type":"boolean"},"alt":{"type":"boolean"},"command":{"type":"boolean"}},"additionalProperties":false});
    let button = json!({"type":"string","enum":["left","right","middle"],"default":"left"});
    let bound = pcbcraft_engine::schematic::MAX_CENTER_NM + pcbcraft_engine::schematic::MAX_PIN_OFFSET_NM;
    let coordinate = json!({"type":"integer","minimum":-bound,"maximum":bound});
    [
        ("inspect","Inspect queued UI input",json!({}),vec![]),
        ("cancel","Cancel queued UI input",json!({}),vec![]),
        ("handles","List visible named controls in one window",json!({"window":{"type":"integer"},"prefix":{"type":"string"}}),vec!["window"]),
        ("at","Locate one unique enabled control",json!({"window":{"type":"integer"},"handle":{"type":"string"}}),vec!["window","handle"]),
        ("window","Measure one native window",json!({"window":{"type":"integer"}}),vec!["window"]),
        ("resize","Request a native window's client size",json!({"window":{"type":"integer"},"width":{"type":"number"},"height":{"type":"number"}}),vec!["window","width","height"]),
        ("project","Project a model point in a view to window coordinates",json!({"view":{"type":"integer"},"at":{"type":"object","properties":{"x_nm":coordinate,"y_nm":coordinate},"required":["x_nm","y_nm"],"additionalProperties":false}}),vec!["view","at"]),
        ("move","Move the pointer through ordinary UI input",json!({"target":point}),vec!["target"]),
        ("click","Click through ordinary UI input",json!({"target":point,"button":button,"modifiers":keys}),vec!["target"]),
        ("drag","Drag over successive UI frames",json!({"from":point,"to":point,"steps":{"type":"integer","minimum":2,"maximum":120},"button":button,"modifiers":keys}),vec!["from","to","steps"]),
        ("key","Press and release a key",json!({"key":{"type":"string"},"modifiers":keys}),vec!["key"]),
        ("text","Type text into the focused field",json!({"text":{"type":"string","minLength":1,"maxLength":4096}}),vec!["text"]),
        ("scroll","Scroll at a named target or logical point",json!({"target":point,"delta_x":{"type":"number"},"delta_y":{"type":"number"},"modifiers":keys}),vec!["target","delta_x","delta_y"]),
    ].into_iter().map(|(name,title,mut properties,mut required)| {
        if matches!(name,"move"|"click"|"drag"|"key"|"text"|"scroll") { properties["window"]=json!({"type":"integer"});properties["layout"]=json!({"type":"integer"});required.push("window"); }
        json!({"id":format!("ui.{name}"),"title":title,"summary":format!("{title}. Native live editor only. Input returns a ticket; poll ui.inspect until completed before sending more input. Coordinates are window-local logical points."),
            "scope":"window","params":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}
