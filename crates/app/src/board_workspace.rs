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
    /// Delete only explicitly selected placements in the addressed focused view. Active tools
    /// and previews keep ownership of input; schematic components and package choices remain.
    pub(super) fn delete_board_footprints(&mut self, ctx: &egui::Context, view: ViewId) {
        let Some(editor) = self.views.view(view).filter(|v| v.workspace == Workspace::Board && v.definition.is_none()) else { return };
        if self.views.active.id() != view
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || self.files.busy()
            || self.project_action_pending()
            || editor.board.draft.is_some()
            || editor.board.box_drag.is_some()
            || editor.board.drawing.tool.is_some()
            || editor.board.drawing.source().is_some()
            || editor.board.selection.is_empty()
        {
            return;
        }
        let Some(source) = editor.input(&self.session) else { return };
        let Some(target) = editor.board.target else { return };
        let params = serde_json::json!({"board":target.board,"footprints":editor.board.selection,"expected_revision":source.revision});
        self.run(pcbcraft_engine::DELETE_FOOTPRINTS, &params);
        self.repaint_editors(ctx);
    }

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
            if let Some(view) = self.views.view_mut(source) {
                view.definition = None;
                view.release_input();
            }
            if let Some(project) = self.session.project() {
                let target = match workspace {
                    Workspace::Schematic => DocumentTarget::Sheet(
                        self.views
                            .view(source)
                            .and_then(|view| view.snapshot(&self.session))
                            .map_or_else(|| project.sheet_target(), |p| p.sheet_target()),
                    ),
                    Workspace::Board => project.board_target().map_or_else(|| DocumentTarget::BoardWorkspace(project.id()), DocumentTarget::Board),
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

    /// Offer footprint transforms and placement-only deletion alongside navigation, keeping board
    /// tools separate from schematic tools. Delete stays in the group menu, as in Capture;
    /// ordinary canvas selection followed by Delete uses the same command. Completed edits
    /// use the same atomic commands as MCP.
    /// The active-layer field uses the Layers card's view-local handler, without editing the project.
    /// Components › Footprints opens the package chooser for the selected footprint's component,
    /// or leaves the component unselected when no footprint is selected.
    pub(super) fn board_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let setup =
            RibbonTool { icon: ToolIcon::Settings, label: "Setup", description: "Board setup", shortcut: "", enabled, active: false, toggle: false };
        if workspace::ribbon_group(ui, "Board", &[setup]).is_some() {
            self.begin_board_setup(self.views.active.id());
        }
        workspace::ribbon_separator(ui);
        let update = RibbonTool {
            icon: ToolIcon::Sync,
            label: "Update PCB",
            description: "Update PCB from schematic",
            shortcut: "",
            enabled: self.session.project().is_some() && self.views.active.input(&self.session).is_some(),
            active: false,
            toggle: false,
        };
        if workspace::ribbon_group(ui, "Schematic", &[update]).is_some() {
            self.message = self.begin_board_update(self.views.active.id()).err();
        }
        workspace::ribbon_separator(ui);
        let selected = enabled && self.views.active.board.selection.len() == 1;
        let editing = self.views.active.board.draft.is_some();
        let edit_tools = [
            RibbonTool {
                icon: ToolIcon::Move,
                label: "Move",
                description: "Move footprint",
                shortcut: "M",
                enabled: selected && !editing,
                active: editing,
                toggle: false,
            },
            RibbonTool {
                icon: ToolIcon::Rotate,
                label: "Rotate",
                description: "Rotate footprint counterclockwise",
                shortcut: "R",
                enabled: selected || editing,
                active: false,
                toggle: false,
            },
        ];
        let delete = RibbonTool {
            icon: ToolIcon::Delete,
            label: "Delete",
            description: "Delete selected footprints",
            shortcut: "Delete",
            enabled: enabled
                && !self.views.active.board.selection.is_empty()
                && !editing
                && self.views.active.board.drawing.tool.is_none()
                && self.views.active.board.drawing.source().is_none(),
            active: false,
            toggle: false,
        };
        match workspace::ribbon_group_with_menu(ui, "Placement", &edit_tools, &[delete]) {
            Some(0) => self.views.active.board.move_requested = true,
            Some(1) => self.views.active.board.rotate_requested = 90,
            Some(2) => self.delete_board_footprints(ui.ctx(), self.views.active.id()),
            _ => {}
        }
        workspace::ribbon_separator(ui);
        if workspace::ribbon_group(
            ui,
            "Components",
            &[RibbonTool {
                icon: ToolIcon::Symbol,
                label: "Footprints",
                description: "Assign and place component footprints",
                shortcut: "",
                enabled: enabled && self.session.project().is_some_and(|p| p.components().next().is_some()),
                active: false,
                toggle: false,
            }],
        )
        .is_some()
        {
            let component = self
                .session
                .project()
                .and_then(|p| p.board_target().and_then(|target| p.board_view(target)))
                .and_then(|board| board.footprints().find(|f| self.views.active.board.selection.contains(&f.id)).map(|f| f.component));
            self.begin_footprint_assignment(component);
        }
        workspace::ribbon_separator(ui);
        if let Some(board) = self.session.project().and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target))) {
            let layers = crate::board_panels::ordered_layers(board);
            let labels: Vec<_> = layers
                .iter()
                .map(|layer| {
                    let label = crate::board_panels::layer_name(layer);
                    if self.views.active.board.hidden_layers.contains(&layer.id.to_string()) { format!("{label} (hidden)") } else { label.to_owned() }
                })
                .collect();
            let options: Vec<_> = labels
                .iter()
                .zip(&layers)
                .enumerate()
                .map(|(i, (label, layer))| (i, label.as_str(), self.board_appearance.color(board, layer)))
                .collect();
            let mut selected =
                layers.iter().position(|layer| self.views.active.board.active_layer.as_deref() == Some(layer.id.to_string().as_str())).unwrap_or(0);
            let changed = workspace::ribbon_field(ui, "Active layer", |ui| workspace::layer_choice(ui, "Active layer", &mut selected, &options));
            if changed && let Some(layer) = layers.get(selected) {
                self.views.active.board.set_layer(&layer.id.to_string(), true, None);
            }
            workspace::ribbon_separator(ui);
        }
        let can_draw =
            self.session.project().and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target))).is_some_and(
                |board| {
                    crate::board_drawing::can_draw(board, self.views.active.board.active_layer.as_deref(), &self.views.active.board.hidden_layers)
                },
            );
        let drawing_tools = [
            RibbonTool {
                icon: ToolIcon::Line,
                label: "Line",
                description: "Draw lines on Board Edge",
                shortcut: "",
                enabled: enabled && can_draw && !editing,
                active: self.views.active.board.drawing.tool == Some(crate::board_drawing::Tool::Lines),
                toggle: false,
            },
            RibbonTool {
                icon: ToolIcon::Rectangle,
                label: "Rectangle",
                description: "Draw rectangle on Board Edge",
                shortcut: "",
                enabled: enabled && can_draw && !editing,
                active: self.views.active.board.drawing.tool == Some(crate::board_drawing::Tool::Rectangle),
                toggle: false,
            },
        ];
        if let Some(index) = workspace::ribbon_group(ui, "Draw", &drawing_tools) {
            let tool = if index == 0 { crate::board_drawing::Tool::Lines } else { crate::board_drawing::Tool::Rectangle };
            let layer = self.views.active.board.active_layer.clone();
            self.views.active.board.drawing.start(tool, layer);
        }
        workspace::ribbon_separator(ui);
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
    /// returns selection and transform intent; validation and dispatch follow after its borrow ends.
    /// All board panes use the shared grid display preference without changing their cameras.
    pub(super) fn board_pane(&mut self, ui: &mut Ui, id: ViewId) {
        let enabled = ui.is_enabled()
            && (crate::IN_BROWSER || ui.input(|input| input.focused))
            && !self.home
            && self.views.active.id() == id
            && matches!(self.dialog, Dialog::None)
            && !self.project_action_pending()
            && !egui::Popup::is_any_open(ui.ctx());
        if self.session.project().is_some_and(|project| project.board_target().is_none())
            && self.views.view(id).is_some_and(|view| !view.unavailable())
        {
            self.empty_board_pane(ui, id, enabled);
            return;
        }
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
                        &mut view.board,
                        source,
                        &highlighted,
                        view.filters,
                        &self.board_appearance,
                        enabled,
                        self.preferences.navigation,
                        self.preferences.grid_opacity_percent,
                        self.preferences.board_grid_style,
                        &label,
                    )
                })
            })
            .inner;
        if (pick.selection.is_some() || pick.edit.is_some() || pick.drawing_edit.is_some() || pick.placement.is_some())
            && enabled
            && source.is_some_and(|source| self.views.active.accepts(source, &self.session))
        {
            if let Some(selection) = pick.selection {
                self.views.select_board(selection, &self.session, self.preferences.selection_follow);
            }
            if let Some(mut edit) = pick.drawing_edit
                && self.views.active.accepts(edit.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                edit.params["board"] = serde_json::json!(target.board);
                self.run(edit.id, &edit.params);
            }
            if let Some(placement) = pick.placement
                && self.views.active.accepts(placement.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                self.run(
                    pcbcraft_engine::PLACE_ASSIGNED_FOOTPRINTS,
                    &serde_json::json!({
                        "board": target.board, "components": placement.components,
                        "x_nm": placement.position.x_nm, "y_nm": placement.position.y_nm,
                        "rotation": placement.rotation, "expected_revision": placement.source.revision,
                    }),
                );
            }
            if let Some(draft) = pick.edit
                && self.views.active.accepts(draft.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                self.run(
                    pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
                    &serde_json::json!({
                        "board": target.board, "footprint": draft.footprint,
                        "x_nm": draft.position.x_nm, "y_nm": draft.position.y_nm,
                        "rotation": draft.rotation, "expected_revision": draft.source.revision,
                    }),
                );
            }
            self.repaint_editors(ui.ctx());
        }
    }

    /// Present the empty PCB workspace without creating a physical owner. Explicit creation
    /// uses the shared revision-checked command and binds the resulting board after success.
    fn empty_board_pane(&mut self, ui: &mut Ui, id: ViewId, enabled: bool) {
        let revision = self.session.revision();
        let Some(widget_id) = self.views.view(id).map(|view| view.widget_id()) else { return };
        let create = ui
            .push_id(widget_id, |ui| {
                shell::canvas(ui, |ui| {
                    ui.add_space((ui.available_height() * 0.5 - 16.0).max(0.0));
                    ui.vertical_centered(|ui| ui.add_enabled(enabled, pcbcraft_chrome::catalog::button("Create board", true)).clicked()).inner
                })
            })
            .inner;
        if create && self.session.available("pcb.create_board") {
            self.run("pcb.create_board", &serde_json::json!({"expected_revision":revision}));
            if self.message.is_none()
                && let Some(target) = self.session.project().and_then(|project| project.board_target())
                && let Some(view) = self.views.view_mut(id)
            {
                view.open_document(DocumentTarget::Board(target), &self.session);
            }
            self.repaint_editors(ui.ctx());
        }
    }
}
