//! Native hosts borrow one application/session synchronously. Each host keeps its own panels.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;

use egui::{Context, ViewportCommand, ViewportId};

use crate::{Dialog, IN_BROWSER, PcbApp, ViewInput, editor_layout::HostId};

/// Window presentation and queued view actions never own project data, files or services.
#[derive(Debug, Default)]
pub(super) struct EditorWindows {
    focused: Option<ViewportId>,
    #[cfg(not(target_arch = "wasm32"))]
    secondary: BTreeMap<HostId, Panels>,
    pending: Vec<(ViewInput, ViewAction)>,
}

/// Presentation actions address an existing view, independently of its current render slot.
#[derive(Clone, Copy, Debug)]
pub(super) enum ViewAction {
    OpenWindow,
    Detach,
    Dock,
    OpenBeside,
    Close,
}

/// Stationary contextual controls belong to one host even as its focused pane changes.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
struct Panels {
    browser: crate::project_browser::ProjectBrowser,
    parts: crate::parts_browser::PartsBrowser,
    nets: crate::net_inspector::NetInspector,
    ribbon: usize,
    pointer: Option<pcbcraft_engine::schematic::Point>,
    pointer_next: Option<pcbcraft_engine::schematic::Point>,
    placed: bool,
}

/// A viewport's identity belongs to its host, independently of the views displayed inside it.
pub(super) fn viewport(host: HostId) -> ViewportId {
    if host == HostId::ROOT { ViewportId::ROOT } else { ViewportId::from_hash_of(("pcbcraft_editor", host)) }
}

impl PcbApp {
    /// Repaint every visible editor after a shared edit or selection intent.
    pub(super) fn repaint_editors(&self, ctx: &Context) {
        for host in self.views.hosts.keys() {
            ctx.request_repaint_of(viewport(*host));
        }
    }

    /// Capture the view's current input address. Modal, pending-project and browser restrictions
    /// apply to every caller; actions execute after all render slots have returned to the root.
    pub(super) fn queue_view_action(&mut self, ctx: &Context, id: crate::ViewId, action: ViewAction) {
        if self.home
            || self.project_action_pending()
            || !matches!(self.dialog, Dialog::None)
            || (IN_BROWSER && matches!(action, ViewAction::OpenWindow | ViewAction::Detach | ViewAction::Dock))
        {
            return;
        }
        if self.views.owner(id).is_some()
            && let Some(source) = self.views.view(id).and_then(|view| view.input(&self.session))
        {
            self.windows.pending.push((source, action));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    /// Detach the primary host's focused view, or explicitly dock the current native pane.
    /// Neither operation targets an arbitrary companion window.
    pub(super) fn toggle_editor_window(&mut self, ctx: &Context) {
        let action = if self.views.host == HostId::ROOT { ViewAction::Detach } else { ViewAction::Dock };
        self.queue_view_action(ctx, self.views.active.id(), action);
    }

    /// Create another native view directly, with no intermediate split or movement of a view.
    pub(super) fn open_editor_window(&mut self, ctx: &Context) {
        self.queue_view_action(ctx, self.views.active.id(), ViewAction::OpenWindow);
    }

    /// Apply queued layout actions after every child has restored the root's slots and panels.
    /// A closed/rebound view or an intervening edit/layout action invalidates its captured input.
    pub(super) fn finish_view_actions(&mut self, ctx: &Context) {
        for (source, action) in std::mem::take(&mut self.windows.pending) {
            if self.home
                || self.project_action_pending()
                || !matches!(self.dialog, Dialog::None)
                || !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session))
            {
                continue;
            }
            match action {
                ViewAction::OpenWindow => {
                    self.views.open_window(source.view());
                }
                ViewAction::Detach => {
                    self.views.detach_view(source.view());
                }
                ViewAction::Dock => {
                    self.views.dock_view(source.view());
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
                }
                ViewAction::OpenBeside => {
                    self.views.open_beside(source.view());
                }
                ViewAction::Close => self.views.close_view(source.view()),
            }
            self.parts.release_focus(ctx);
            egui::DragAndDrop::clear_payload(ctx);
            self.repaint_editors(ctx);
        }
    }

    /// Cancel old host input on actual native focus changes, never on repaint. A modal retains
    /// its captured view address until it is submitted or cancelled in the primary window.
    pub(super) fn editor_window_focus(&mut self, ctx: &Context) {
        if !ctx.input(|input| input.focused) {
            self.views.active.cancel_gesture();
            return;
        }
        let host = ctx.viewport_id();
        if self.windows.focused != Some(host) {
            if self.windows.focused.is_some() && matches!(self.dialog, Dialog::None) {
                self.views.suspend();
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

    /// Route File → Quit and Ctrl+Q from either host to the primary project-close handler.
    /// The child's own window Close command only dismisses its view.
    pub(super) fn request_project_close(&self, ctx: &Context) {
        if !IN_BROWSER {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }
    }

    /// Draw project dialogs once in the primary window. A fields dialog temporarily binds its
    /// originating editor for revision validation, without moving either camera or stealing selection.
    pub(super) fn project_dialogs(&mut self, ctx: &Context) {
        let previous = self.views.active.id();
        if let Dialog::EditFields { source, .. } = &self.dialog {
            self.views.bind(source.view());
        }
        self.dialogs(ctx);
        self.views.bind(previous);
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

    /// Render each native host once, borrowing the same application and restoring its root
    /// render slot afterwards. A child Close discards only that host's views and panel state.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn detached_editor(&mut self, ctx: &Context) {
        self.windows.secondary.retain(|host, _| self.views.hosts.contains_key(host));
        for host in self.views.detached_hosts() {
            let Some(id) = self.views.hosts.get(&host).map(|host| host.focused) else {
                continue;
            };
            let path = self.views.view(id).and_then(|view| view.sheet_path(&self.session)).unwrap_or_else(|| "Schematic".to_owned());
            let integrated_titlebar = self.integrated_titlebar;
            let panels = self.windows.secondary.entry(host).or_default();
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("{path} — PCBCraft"))
                .with_decorations(!integrated_titlebar)
                .with_min_inner_size([480.0, 400.0]);
            if !panels.placed {
                builder = builder.with_inner_size([1100.0, 760.0]);
                panels.placed = true;
            }
            ctx.show_viewport_immediate(viewport(host), builder, |ui, class| {
                if class != egui::ViewportClass::EmbeddedWindow && ui.ctx().input(|input| input.viewport().close_requested()) {
                    self.views.close_host(host);
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    self.repaint_editors(ui.ctx());
                    return;
                }
                self.views.host = host;
                self.views.bind(id);
                self.swap_editor_panels(host);
                ui.push_id(viewport(host), |ui| self.secondary_editor_frame(ui, integrated_titlebar && class != egui::ViewportClass::EmbeddedWindow));
                self.swap_editor_panels(host);
                self.views.bind_root();
            });
        }
        self.windows.secondary.retain(|host, _| self.views.hosts.contains_key(host));
    }

    /// Exchange only this host's panels; project, files, dialogs and services stay shared.
    #[cfg(not(target_arch = "wasm32"))]
    fn swap_editor_panels(&mut self, host: HostId) {
        let panels = self.windows.secondary.entry(host).or_default();
        std::mem::swap(&mut self.browser, &mut panels.browser);
        std::mem::swap(&mut self.parts, &mut panels.parts);
        std::mem::swap(&mut self.net_inspector, &mut panels.nets);
        std::mem::swap(&mut self.ribbon_tab, &mut panels.ribbon);
        std::mem::swap(&mut self.pointer, &mut panels.pointer);
        std::mem::swap(&mut self.pointer_next, &mut panels.pointer_next);
    }

    /// Route child-window input to its bound view, then draw the same editing controls. Global
    /// dialogs live in the primary window and disable both hosts. File/services poll only there.
    /// An undecorated native host receives the shared app-bar controls and edge resize zones.
    #[cfg(not(target_arch = "wasm32"))]
    fn secondary_editor_frame(&mut self, ui: &mut egui::Ui, integrated_titlebar: bool) {
        let ctx = ui.ctx().clone();
        let revision = self.session.revision();
        self.pointer = self.pointer_next.take();
        let had_dialog = !matches!(self.dialog, Dialog::None);
        let was_home = self.home;
        self.editor_window_focus(&ctx);
        self.views.sync(&self.session);
        self.editor_pointer_focus(&ctx);
        let enabled = !self.home && !had_dialog && !self.project_action_pending();
        if enabled && ctx.input(|input| input.focused) {
            self.shortcuts(&ctx, false, self.views.active.id());
        }
        let integrated = std::mem::replace(&mut self.integrated_titlebar, integrated_titlebar);
        if !enabled {
            ui.disable();
        }
        self.app_bar(ui);
        self.ribbon(ui);
        if !matches!(self.dialog, Dialog::None) {
            self.views.cancel_gestures();
        }
        self.status(ui);
        self.editor_workspace(ui, ui.available_rect_before_wrap());
        if self.pointer != self.pointer_next {
            // The status bar precedes the canvas; paint once more to publish this host's readout.
            ctx.request_repaint();
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if integrated_titlebar {
            pcbcraft_chrome::shell::resize_window_edges(ui);
        }
        self.integrated_titlebar = integrated;
        self.views.sync(&self.session);
        if !had_dialog && !matches!(self.dialog, Dialog::None) || !was_home && self.home {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if revision != self.session.revision() {
            self.repaint_editors(&ctx);
        }
    }
}
