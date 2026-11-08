//! Native hosts borrow one application/session synchronously. Each host keeps its own panels.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;

use egui::{Context, ViewportCommand, ViewportId};

use crate::{Dialog, IN_BROWSER, PcbApp, ViewInput, editor_layout::HostId};

mod drag;

/// Window presentation and queued view actions never own project data, files or services.
#[derive(Debug, Default)]
pub(super) struct EditorWindows {
    drag: Option<drag::PaneDrag>,
    drag_geometry: std::collections::BTreeMap<HostId, drag::HostGeometry>,
    spawn_positions: std::collections::BTreeMap<HostId, egui::Pos2>,
    pub(super) bounds: std::collections::BTreeMap<HostId, crate::arrangements::WindowBounds>,
    restore_bounds: std::collections::BTreeSet<HostId>,
    focused: Option<ViewportId>,
    #[cfg(not(target_arch = "wasm32"))]
    panels: BTreeMap<HostId, Panels>,
    pending: Vec<(ViewInput, ViewAction)>,
    pub(super) quit_requested: bool,
}

/// Presentation actions address an existing view, independently of its current render slot.
#[derive(Clone, Debug)]
pub(super) enum ViewAction {
    Browse(crate::project_browser::Action),
    Workspace { workspace: crate::editor_view::Workspace, placement: pcbcraft_chrome::workspace::Placement },
    OpenWindow,
    Detach,
    Dock,
    OpenBeside,
    Close,
    MoveBeside(ViewInput),
    Reorder(ViewInput),
    DetachAt(Option<egui::Pos2>),
}

/// Stationary contextual controls belong to one host even as its focused pane changes.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct Panels {
    browser: crate::project_browser::ProjectBrowser,
    parts: crate::parts_browser::PartsBrowser,
    nets: crate::net_inspector::NetInspector,
    ribbon: usize,
    pointer: Option<pcbcraft_engine::schematic::Point>,
    pointer_next: Option<pcbcraft_engine::schematic::Point>,
    placed: bool,
    properties: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Panels {
    /// New native hosts use the same Properties visibility as a fresh arrangement.
    fn default() -> Self {
        Self {
            browser: Default::default(),
            parts: Default::default(),
            nets: Default::default(),
            ribbon: 0,
            pointer: None,
            pointer_next: None,
            placed: false,
            properties: crate::arrangements::PanelState::default().properties,
        }
    }
}

/// A host's panel visibility and ribbon tab, as window inspection reports them.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HostPanels {
    pub(super) browser: bool,
    pub(super) parts: bool,
    pub(super) catalog_mode: &'static str,
    pub(super) repeat_placement: bool,
    pub(super) nets: bool,
    pub(super) properties: bool,
    pub(super) ribbon: usize,
}

/// A viewport's identity belongs to its host, independently of the views displayed inside it.
pub(super) fn viewport(host: HostId) -> ViewportId {
    if host == HostId::ROOT { ViewportId::ROOT } else { ViewportId::from_hash_of(("pcbcraft_editor", host)) }
}

impl PcbApp {
    /// Read host presentation without exchanging its render slot or cached document objects.
    pub(super) fn saved_editor_panels(&self, host: HostId) -> crate::arrangements::PanelState {
        if host == self.views.host {
            return crate::arrangements::PanelState {
                browser: self.browser.saved_state(),
                parts: self.parts.saved_state(),
                nets: self.net_inspector.open,
                properties: self.properties_open,
                ribbon: self.ribbon_tab,
            };
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(panels) = self.windows.panels.get(&host) {
            return crate::arrangements::PanelState {
                browser: panels.browser.saved_state(),
                parts: panels.parts.saved_state(),
                nets: panels.nets.open,
                properties: panels.properties,
                ribbon: panels.ribbon,
            };
        }
        crate::arrangements::PanelState::default()
    }

    /// Restore contextual controls with empty caches, invalidating queued placement actions.
    pub(super) fn restore_editor_panels(&mut self, state: &crate::arrangements::Snapshot) {
        self.windows = EditorWindows::default();
        for (id, host) in self.views.host_order().into_iter().zip(&state.hosts) {
            if let Some(bounds) = &host.bounds {
                self.windows.bounds.insert(id, bounds.clone());
                self.windows.restore_bounds.insert(id);
            }
            let browser = crate::project_browser::ProjectBrowser::from_state(host.panels.browser);
            let parts = crate::parts_browser::PartsBrowser::from_state(&host.panels.parts);
            let mut nets = crate::net_inspector::NetInspector::default();
            nets.open = host.panels.nets;
            if id == self.views.host {
                self.browser = browser;
                self.parts = parts;
                self.net_inspector = nets;
                self.properties_open = host.panels.properties;
                self.ribbon_tab = host.panels.ribbon;
                self.pointer = None;
                self.pointer_next = None;
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                self.windows
                    .panels
                    .insert(id, Panels { browser, parts, nets, properties: host.panels.properties, ribbon: host.panels.ribbon, ..Panels::default() });
            }
        }
    }

    /// Remember settled native bounds. Restored windows are constrained to the currently
    /// reported monitor; positions that cannot be verified fall back beside the visible host.
    pub(super) fn editor_window_geometry(&mut self, ctx: &Context) {
        if IN_BROWSER {
            return;
        }
        let host = self.views.host;
        let info = ctx.input(|input| input.viewport().clone());
        let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else {
            return;
        };
        if self.windows.restore_bounds.remove(&host) {
            if let Some(saved) = self.windows.bounds.get(&host) {
                let monitor = info.monitor_size.unwrap_or(inner.size());
                let size = egui::vec2(saved.size[0], saved.size[1]).min(monitor).max(egui::vec2(100.0, 100.0));
                let desired = egui::Rect::from_min_size(egui::pos2(saved.position[0], saved.position[1]), size);
                let reachable = egui::Rect::from_min_size(egui::Pos2::ZERO, monitor).contains_rect(desired);
                let position = if reachable { desired.min } else { outer.min };
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(saved.maximized));
                ctx.request_repaint();
            }
            return;
        }
        let bounds = crate::arrangements::WindowBounds {
            position: [outer.min.x, outer.min.y],
            size: [inner.width(), inner.height()],
            maximized: info.maximized.unwrap_or(false),
        };
        if bounds.valid() {
            self.windows.bounds.insert(host, bounds);
        }
    }

    /// Repaint every visible editor after a shared edit or selection intent.
    pub(super) fn repaint_editors(&self, ctx: &Context) {
        for host in self.views.hosts.keys() {
            ctx.request_repaint_of(viewport(*host));
        }
    }

    /// Capture the view's current input address. Modal, pending-project and browser restrictions
    /// apply to every caller; actions execute after all render slots return to the current main host.
    pub(super) fn queue_view_action(&mut self, ctx: &Context, id: crate::ViewId, action: ViewAction) {
        if self.home
            || self.project_action_pending()
            || !matches!(self.dialog, Dialog::None)
            || (IN_BROWSER
                && matches!(
                    action,
                    ViewAction::OpenWindow | ViewAction::Detach | ViewAction::Dock | ViewAction::MoveBeside(_) | ViewAction::DetachAt(_)
                ))
        {
            return;
        }
        if self.views.owner(id).is_some()
            && let Some(source) = self.views.view(id).and_then(|view| view.control_input(&self.session))
        {
            self.windows.pending.push((source, action));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    /// Detach the primary host's focused view, or explicitly dock the current native pane.
    /// Neither operation targets an arbitrary companion window.
    pub(super) fn toggle_editor_window(&mut self, ctx: &Context) {
        let action = if self.views.host == self.views.main { ViewAction::Detach } else { ViewAction::Dock };
        self.queue_view_action(ctx, self.views.active.id(), action);
    }

    /// Create another native view directly, with no intermediate split or movement of a view.
    pub(super) fn open_editor_window(&mut self, ctx: &Context) {
        self.queue_view_action(ctx, self.views.active.id(), ViewAction::OpenWindow);
    }

    /// Apply queued layout actions after every child restores the current main host's slots and panels.
    /// A closed/rebound view or an intervening edit/layout action invalidates its captured input.
    pub(super) fn finish_view_actions(&mut self, ctx: &Context) {
        for (source, action) in std::mem::take(&mut self.windows.pending) {
            if self.home
                || self.project_action_pending()
                || !matches!(self.dialog, Dialog::None)
                || !self.views.view(source.view()).is_some_and(|view| view.control_input(&self.session) == Some(source))
            {
                continue;
            }
            self.apply_view_action(ctx, source.view(), action);
        }
    }

    /// Apply Browser navigation/selection/centering, sheet-tab and ribbon actions, and their
    /// window commands. Call only with the main host bound and no project dialog or action
    /// pending. The deferred queue drops stale captured input before reaching this handler;
    /// invalid Browser targets leave every view unchanged. Unknown views, a sole primary pane's
    /// Close and Dock of an already docked view also leave layouts unchanged.
    pub(super) fn apply_view_action(&mut self, ctx: &Context, id: crate::ViewId, action: ViewAction) {
        match action {
            ViewAction::Browse(action) => {
                self.apply_browser_action(id, action);
                if matches!(self.dialog, Dialog::Sheet(_) | Dialog::Hierarchy(_)) {
                    ctx.send_viewport_cmd_to(viewport(self.views.main), ViewportCommand::Focus);
                }
            }
            ViewAction::Workspace { workspace, placement } => self.apply_workspace(id, workspace, placement),
            ViewAction::OpenWindow => {
                self.views.open_window(id);
            }
            ViewAction::Detach => {
                self.views.detach_view(id);
            }
            ViewAction::Dock => {
                self.views.dock_view(id);
                ctx.send_viewport_cmd_to(viewport(self.views.main), ViewportCommand::Focus);
            }
            ViewAction::OpenBeside => {
                self.views.open_beside(id);
            }
            ViewAction::Close => self.views.close_view(id),
            ViewAction::MoveBeside(target) => {
                if self.views.view(target.view()).is_some_and(|view| view.control_input(&self.session) == Some(target))
                    && self.views.move_beside(id, target.view())
                    && let Some(host) = self.views.owner(id)
                {
                    ctx.send_viewport_cmd_to(viewport(host), ViewportCommand::Focus);
                }
            }
            ViewAction::Reorder(target) => {
                if self.views.view(target.view()).is_some_and(|view| view.control_input(&self.session) == Some(target)) {
                    self.views.reorder_view(id, target.view());
                }
            }
            ViewAction::DetachAt(position) => {
                let source = self.views.owner(id);
                if let Some(host) = self.views.detach_view(id) {
                    if let Some(position) = position {
                        if source != Some(host) {
                            self.windows.spawn_positions.insert(host, position);
                        }
                        ctx.send_viewport_cmd_to(viewport(host), ViewportCommand::OuterPosition(position));
                    }
                    ctx.send_viewport_cmd_to(viewport(host), ViewportCommand::Focus);
                }
            }
        }
        self.sync_library_bindings();
        self.parts.release_focus(ctx);
        egui::DragAndDrop::clear_payload(ctx);
        self.repaint_editors(ctx);
    }

    /// Which contextual panels `host` shows and its ribbon tab, whether or not it is bound.
    /// A host that has not been drawn yet reports the panels it will start with.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn host_panels(&self, host: HostId) -> HostPanels {
        if host == self.views.host {
            return HostPanels {
                browser: self.browser.open,
                parts: self.parts.open,
                catalog_mode: self.parts.presentation(),
                repeat_placement: self.parts.repeats(),
                nets: self.net_inspector.open,
                properties: self.properties_open,
                ribbon: self.ribbon_tab,
            };
        }
        let fresh = Panels::default();
        let panels = self.windows.panels.get(&host).unwrap_or(&fresh);
        HostPanels {
            browser: panels.browser.open,
            parts: panels.parts.open,
            catalog_mode: panels.parts.presentation(),
            repeat_placement: panels.parts.repeats(),
            nets: panels.nets.open,
            properties: panels.properties,
            ribbon: panels.ribbon,
        }
    }

    /// Cancel old host input on actual native focus changes, never on repaint. A modal retains
    /// its captured view address until it is submitted or cancelled in the primary window.
    pub(super) fn editor_window_focus(&mut self, ctx: &Context) {
        if !ctx.input(|input| input.focused) {
            self.views.active.cancel_gesture();
            self.cancel_library_host_gestures();
            return;
        }
        let host = ctx.viewport_id();
        if self.windows.focused != Some(host) {
            if self.windows.focused.is_some() && self.windows.drag.is_none() && matches!(self.dialog, Dialog::None) {
                self.views.suspend();
                self.cancel_library_gestures();
                egui::DragAndDrop::clear_payload(ctx);
            }
            self.windows.focused = Some(host);
        }
    }

    /// Hold editing while a project action awaits a file worker or shutdown is approved.
    /// The merged unsaved-changes controller remains the sole owner of those decisions.
    pub(super) fn project_action_pending(&self) -> bool {
        self.waiting.is_some() || self.close_allowed
    }

    /// Mark explicit Quit separately from window Close, then wake the shared project-close handler.
    pub(super) fn request_project_close(&mut self, ctx: &Context) {
        if !IN_BROWSER {
            self.windows.quit_requested = true;
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }
    }

    /// Draw project dialogs once in their owner window (normally the main host). A fields
    /// or package dialog temporarily binds its originating editor for revision validation. Sheet forms can change
    /// the focused document, so they restore the host's resulting focus rather than its old slot.
    pub(super) fn project_dialogs(&mut self, ctx: &Context) {
        let previous = self.views.active.id();
        let sheet_form = matches!(self.dialog, Dialog::Sheet(_) | Dialog::Hierarchy(_));
        match &self.dialog {
            Dialog::EditFields(edit) => {
                self.views.bind(edit.source.view());
            }
            Dialog::Footprint(edit) => {
                self.views.bind(edit.source.view());
            }
            _ => {}
        }
        self.dialogs(ctx);
        if sheet_form {
            self.views.bind_main();
        } else {
            self.views.bind(previous);
        }
        if self.files.busy()
            && matches!(
                self.waiting,
                Some(
                    crate::unsaved::Waiting::ToDecide(crate::unsaved::Then::Close)
                        | crate::unsaved::Waiting::ForSave(crate::unsaved::Then::Close)
                        | crate::unsaved::Waiting::Discarding(crate::unsaved::Then::Close)
                )
            )
            && matches!(self.dialog, Dialog::None)
        {
            let mut cancel = false;
            let dismissed = pcbcraft_chrome::shell::dialog(ctx, "close_wait", "Close Project", |ui| {
                ui.label("Waiting for the current file operation before closing.");
                cancel = ui.button("Cancel close").clicked();
            });
            if cancel || dismissed {
                // Keep the worker and its immutable snapshot; only withdraw the quit request.
                self.waiting = None;
                self.file_notice = None;
            }
        }
    }

    /// Render every non-root native host once, restoring the main host's controls afterwards.
    /// Close dismisses that host while another survives, or starts the last-window save decision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn detached_editor(&mut self, ctx: &Context) {
        self.windows.panels.retain(|host, _| self.views.hosts.contains_key(host));
        for host in self.views.detached_hosts() {
            let Some(id) = self.views.hosts.get(&host).map(|host| host.focused) else {
                continue;
            };
            let path = self.view_title(id);
            let integrated_titlebar = self.integrated_titlebar;
            let panels = self.windows.panels.entry(host).or_default();
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("{path} — PCBCraft"))
                .with_decorations(!integrated_titlebar)
                .with_min_inner_size([480.0, 400.0]);
            if !panels.placed {
                builder = builder.with_inner_size([1100.0, 760.0]);
                if let Some(position) = self.windows.spawn_positions.remove(&host) {
                    builder = builder.with_position(position);
                }
                panels.placed = true;
            }
            ctx.show_viewport_immediate(viewport(host), builder, |ui, class| {
                if class != egui::ViewportClass::EmbeddedWindow && ui.ctx().input(|input| input.viewport().close_requested()) {
                    self.close_editor_host(host, ui.ctx());
                    if !self.views.hosts.contains_key(&host) {
                        return;
                    }
                    // The last window stays visible while its save decision or file job runs.
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                }
                self.bind_editor_host(host);
                ui.push_id(viewport(host), |ui| self.editor_host_frame(ui, integrated_titlebar && class != egui::ViewportClass::EmbeddedWindow));
                self.bind_editor_host(self.views.main);
            });
        }
        self.windows.panels.retain(|host, _| self.views.hosts.contains_key(host));
    }

    /// Exchange only this host's panels; project, files, dialogs and services stay shared.
    #[cfg(not(target_arch = "wasm32"))]
    fn swap_editor_panels(&mut self, host: HostId) {
        let panels = self.windows.panels.entry(host).or_default();
        std::mem::swap(&mut self.browser, &mut panels.browser);
        std::mem::swap(&mut self.parts, &mut panels.parts);
        std::mem::swap(&mut self.net_inspector, &mut panels.nets);
        std::mem::swap(&mut self.properties_open, &mut panels.properties);
        std::mem::swap(&mut self.ribbon_tab, &mut panels.ribbon);
        std::mem::swap(&mut self.pointer, &mut panels.pointer);
        std::mem::swap(&mut self.pointer_next, &mut panels.pointer_next);
    }

    /// Move the render slot and contextual controls together. The surviving native window keeps
    /// its ID, geometry and UI state when it takes over project-level dialogs and the assistant.
    pub(super) fn bind_editor_host(&mut self, host: HostId) {
        if host != self.views.host {
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.swap_editor_panels(self.views.host);
                self.swap_editor_panels(host);
            }
            self.views.host = host;
        }
        if let Some(layout) = self.views.hosts.get(&host) {
            self.views.bind(layout.focused);
        }
        self.sync_library_focus();
    }

    /// Close just the addressed native window and discard its unfinished arrangement form while
    /// another survives. The final window uses the shared unsaved-changes controller; neither
    /// dismissal nor promotion changes the project.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn close_editor_host(&mut self, host: HostId, ctx: &Context) {
        if !self.views.hosts.contains_key(&host) {
            return;
        }
        if self.views.hosts.len() == 1 {
            self.begin_project_close(ctx);
            return;
        }
        let next = if host == self.views.main { self.views.hosts.keys().copied().find(|id| *id != host) } else { Some(self.views.main) };
        if let Some(next) = next {
            self.bind_editor_host(next);
        }
        self.views.close_host(host);
        if self.arrangement_dialog_host() == Some(host) {
            self.dialog = Dialog::None;
        }
        self.windows.panels.remove(&host);
        self.windows.bounds.remove(&host);
        self.windows.restore_bounds.remove(&host);
        self.windows.spawn_positions.remove(&host);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.send_viewport_cmd_to(viewport(self.views.main), ViewportCommand::Focus);
        ctx.request_repaint_of(ViewportId::ROOT);
        self.repaint_editors(ctx);
    }
}
