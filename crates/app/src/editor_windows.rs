//! Native hosts for the existing views. There is still exactly one session, file controller,
//! live connection and assistant, all owned by PcbApp on the UI thread.

use egui::{Context, ViewportCommand, ViewportId};

use crate::{Dialog, IN_BROWSER, PcbApp};

/// Window presentation is separate from electrical data and editor camera/selection state.
#[derive(Debug, Default)]
pub(super) struct EditorWindows {
    pub(super) rendering_secondary: bool,
    focused: Option<ViewportId>,
    #[cfg(not(target_arch = "wasm32"))]
    secondary: Panels,
    #[cfg(not(target_arch = "wasm32"))]
    placed: bool,
    #[cfg(not(target_arch = "wasm32"))]
    placement: Option<(egui::Pos2, egui::Vec2)>,
    pub(super) dock_requested: bool,
}

/// Each native window retains its own stationary contextual panels and ribbon tab.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
struct Panels {
    browser: crate::project_browser::ProjectBrowser,
    parts: crate::parts_browser::PartsBrowser,
    nets: crate::net_inspector::NetInspector,
    ribbon: usize,
}

/// Stable native host identity, independent of which schematic view it currently contains.
pub(super) fn viewport() -> ViewportId {
    ViewportId::from_hash_of("pcbcraft_editor")
}

impl PcbApp {
    /// Wake every editor after a shared edit or selection change, including a covered window.
    pub(super) fn repaint_editors(&self, ctx: &Context) {
        ctx.request_repaint_of(ViewportId::ROOT);
        if self.views.detached.is_some() {
            ctx.request_repaint_of(viewport());
        }
    }

    /// Move the addressed view between native and split hosts without an electrical command.
    /// A modal owns input; browser builds leave native hosting unavailable.
    pub(super) fn toggle_editor_window(&mut self, ctx: &Context) {
        if IN_BROWSER || self.session.project().is_none() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        if self.views.detached.is_some() {
            self.windows.dock_requested = true;
            if !self.windows.rendering_secondary {
                self.finish_redock(ctx);
            }
        } else {
            self.views.detach();
            self.parts.release_focus(ctx);
            egui::DragAndDrop::clear_payload(ctx);
        }
        self.repaint_editors(ctx);
    }

    /// Finish a queued close/redock after the child has returned its render slot and panels.
    /// No save prompt is needed: the view returns to the same still-open project.
    pub(super) fn finish_redock(&mut self, ctx: &Context) {
        if std::mem::take(&mut self.windows.dock_requested) {
            self.views.redock();
            egui::DragAndDrop::clear_payload(ctx);
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
            ctx.request_repaint_of(ViewportId::ROOT);
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
    /// The child's own window Close command instead redocks its view.
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

    /// Render a secondary native viewport by borrowing this same PcbApp. Window-local panels
    /// and the view slot are exchanged only for this synchronous callback, then restored.
    /// Closing the child redocks its view; the browser never invokes this host. Native windows
    /// use the primary host's decoration policy; embedded fallback windows keep their own frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn detached_editor(&mut self, ctx: &Context) {
        let Some(id) = self.views.detached else {
            self.windows.placed = false;
            return;
        };
        let path = self.views.view(id).and_then(|view| view.sheet_path(&self.session)).unwrap_or_else(|| "Schematic".to_owned());
        let integrated_titlebar = self.integrated_titlebar;
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("{path} — PCBCraft"))
            .with_decorations(!integrated_titlebar)
            .with_min_inner_size([480.0, 400.0]);
        if !self.windows.placed {
            builder = match self.windows.placement {
                Some((position, size)) => builder.with_position(position).with_inner_size(size),
                None => builder.with_inner_size([1100.0, 760.0]),
            };
            self.windows.placed = true;
        }
        ctx.show_viewport_immediate(viewport(), builder, |ui, class| {
            if class != egui::ViewportClass::EmbeddedWindow {
                let (outer, inner, close) = ui.ctx().input(|input| {
                    let viewport = input.viewport();
                    (viewport.outer_rect, viewport.inner_rect, viewport.close_requested())
                });
                if let Some((outer, inner)) = outer.zip(inner) {
                    self.windows.placement = Some((outer.min, inner.size()));
                }
                if close {
                    self.windows.dock_requested = true;
                    return;
                }
            }
            let root_view = self.views.active.id();
            self.views.bind(id);
            self.swap_editor_panels();
            self.windows.rendering_secondary = true;
            ui.push_id(viewport(), |ui| self.secondary_editor_frame(ui, integrated_titlebar && class != egui::ViewportClass::EmbeddedWindow));
            self.windows.rendering_secondary = false;
            self.swap_editor_panels();
            self.views.bind(root_view);
        });
        self.finish_redock(ctx);
    }

    /// Exchange only window presentation; session, saving, dialogs and services remain shared.
    #[cfg(not(target_arch = "wasm32"))]
    fn swap_editor_panels(&mut self) {
        std::mem::swap(&mut self.browser, &mut self.windows.secondary.browser);
        std::mem::swap(&mut self.parts, &mut self.windows.secondary.parts);
        std::mem::swap(&mut self.net_inspector, &mut self.windows.secondary.nets);
        std::mem::swap(&mut self.ribbon_tab, &mut self.windows.secondary.ribbon);
    }

    /// Route child-window input to its bound view, then draw the same editing controls. Global
    /// dialogs live in the primary window and disable both hosts. File/services poll only there.
    /// An undecorated native host receives the shared app-bar controls and edge resize zones.
    #[cfg(not(target_arch = "wasm32"))]
    fn secondary_editor_frame(&mut self, ui: &mut egui::Ui, integrated_titlebar: bool) {
        let ctx = ui.ctx().clone();
        let revision = self.session.revision();
        let had_dialog = !matches!(self.dialog, Dialog::None);
        let was_home = self.home;
        self.editor_window_focus(&ctx);
        self.views.sync(&self.session);
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
