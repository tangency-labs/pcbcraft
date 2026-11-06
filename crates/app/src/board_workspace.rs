//! PCB presentation actions share the existing session and editor host lifecycle.
use crate::{
    Dialog, PcbApp, ViewId,
    editor_view::{DocumentTarget, Workspace},
};
use egui::Ui;
use pcbcraft_chrome::{
    shell,
    workspace::{self, Placement, RibbonTool, ToolIcon},
};

impl PcbApp {
    /// Present the requested document here, beside it, or directly in a native window. Target
    /// changes cancel old input and retain each workspace's camera without creating document edits.
    pub(super) fn open_workspace(&mut self, workspace: Workspace, placement: Placement, ctx: &egui::Context) {
        if self.session.project().is_none() || self.home || !matches!(self.dialog, Dialog::None) || self.project_action_pending() {
            return;
        }
        self.queue_view_action(ctx, self.views.active.id(), crate::editor_windows::ViewAction::Workspace { workspace, placement });
    }

    /// Apply a validated placement request only after all hosts finish rendering. Layout creation
    /// and target changes share the same queue and stale-input checks as Detach and Dock.
    pub(super) fn apply_workspace(&mut self, source: ViewId, workspace: Workspace, placement: Placement) {
        if placement == Placement::Here {
            if let Some(project) = self.session.project() {
                let target = match workspace {
                    Workspace::Schematic => DocumentTarget::Sheet(project.sheet_target()),
                    Workspace::Board => DocumentTarget::Board(project.board_target()),
                };
                self.views.open_document(source, target, &self.session);
            }
            self.pointer = None;
            self.pointer_next = None;
            return;
        }
        let id = match placement {
            Placement::Here => Some(source),
            Placement::Beside => self.views.open_beside(source),
            Placement::Window if !crate::IN_BROWSER => {
                self.views.open_window(source).and_then(|host| self.views.hosts.get(&host).map(|host| host.focused))
            }
            Placement::Window => None,
        };
        if let Some(id) = id {
            self.views.switch_workspace(id, workspace, &self.session);
            self.pointer = None;
            self.pointer_next = None;
        }
    }

    /// The first board ribbon exposes navigation and correspondence without advertising editing
    /// commands whose footprint geometry and reviewed-update semantics are not implemented yet.
    pub(super) fn board_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().is_some();
        let tools = [
            RibbonTool { icon: ToolIcon::Fit, label: "Fit", description: "Fit board", shortcut: "", enabled, active: false, toggle: false },
            RibbonTool {
                icon: ToolIcon::Symbol,
                label: "Reveal",
                description: "Reveal highlighted counterpart",
                shortcut: "",
                enabled,
                active: false,
                toggle: false,
            },
        ];
        match workspace::ribbon_group(ui, "Inspect", &tools) {
            Some(0) => self.views.active.camera.request_fit(),
            Some(1) => self.views.reveal_highlight(self.views.active.id(), &self.session),
            _ => {}
        }
    }

    /// Paint any addressed board pane, with input granted only to its focused view. The renderer
    /// returns selection intent; validation and broadcasting happen after its borrow ends.
    pub(super) fn board_pane(&mut self, ui: &mut Ui, id: ViewId) {
        let enabled = ui.is_enabled()
            && (crate::IN_BROWSER || ui.input(|input| input.focused))
            && !self.home
            && self.views.active.id() == id
            && matches!(self.dialog, Dialog::None)
            && !self.project_action_pending()
            && !egui::Popup::is_any_open(ui.ctx());
        let highlighted = self.views.board_highlight_for(id, &self.session);
        let label = if self.views.is_split() { format!("PCB canvas {}", id.0 + 1) } else { "PCB canvas".to_owned() };
        let Some(project) = self.session.project() else {
            return;
        };
        let Some(view) = self.views.view_mut(id) else {
            return;
        };
        let source = view.input(&self.session);
        let Some(project) = view.board.target.and_then(|target| project.board_view(target)) else {
            return;
        };
        let pick = ui
            .push_id(view.widget_id(), |ui| {
                shell::canvas(ui, |ui| {
                    if let Some(point) = super::pointer_on_canvas(ui, &view.camera) {
                        self.pointer_next = Some(point);
                    }
                    crate::board_canvas::show(
                        ui,
                        project,
                        &mut view.camera,
                        &view.board,
                        &highlighted,
                        enabled,
                        self.preferences.navigation,
                        self.preferences.grid_opacity_percent,
                        &label,
                    )
                })
            })
            .inner;
        if let Some(pick) = pick
            && source.is_some_and(|source| self.views.active.accepts(source, &self.session))
            && enabled
        {
            self.views.select_board(pick, &self.session, self.preferences.selection_follow);
            self.repaint_editors(ui.ctx());
        }
    }
}
