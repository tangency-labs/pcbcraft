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
use pcbcraft_engine::AlignMode;

/// Modify menu alignments: the label and the engine mode each runs.
const ALIGNMENTS: [(&str, AlignMode); 8] = [
    ("Align to Left", AlignMode::Left),
    ("Align to Right", AlignMode::Right),
    ("Align to Top", AlignMode::Top),
    ("Align to Bottom", AlignMode::Bottom),
    ("Align to Vertical Center", AlignMode::CenterX),
    ("Align to Horizontal Center", AlignMode::CenterY),
    ("Distribute Horizontally", AlignMode::DistributeX),
    ("Distribute Vertically", AlignMode::DistributeY),
];

impl PcbApp {
    /// Align or distribute the focused view's selected footprints through the shared engine
    /// command, as one undo step. Refusals, such as too few footprints, appear in the status bar.
    pub(super) fn align_footprints(&mut self, mode: AlignMode) {
        let Some(target) = self.views.active.board.target else { return };
        let params = serde_json::json!({
            "board": target.board, "footprints": self.views.active.board.selection,
            "mode": mode.name(), "expected_revision": self.session.revision(),
        });
        self.run(pcbcraft_engine::ALIGN_FOOTPRINTS, &params);
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

    /// Draw PCB commands by task while preserving the focused board's command handlers.
    /// Layout is the fallback for slots not handled by these PCB task groups.
    pub(super) fn board_tab(&mut self, ui: &mut Ui, slot: usize) {
        match slot {
            crate::ribbon::DRAW_TAB => self.board_draw_tab(ui),
            2 => self.board_analyze_tab(ui),
            crate::ribbon::MANUFACTURE_TAB => self.board_manufacture_tab(ui),
            _ => self.board_layout_tab(ui),
        }
    }

    /// Placement, routing and selection edits, with board setup and synchronization.
    fn board_layout_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let editing = self.views.active.board.editing_footprint();
        let place = [RibbonTool {
            icon: ToolIcon::Footprint,
            label: "Footprint",
            description: "Assign and place component footprints",
            shortcut: "",
            enabled: enabled && self.session.project().is_some_and(|p| p.components().next().is_some()),
            active: false,
            menu_check: None,
        }];
        let picked = workspace::ribbon_group(ui, "Place", &place);
        if picked == Some(0) {
            let component = self
                .session
                .project()
                .and_then(|p| p.board_target().and_then(|target| p.board_view(target)))
                .and_then(|board| board.footprints().find(|f| self.views.active.board.selection.contains(&f.id)).map(|f| f.component));
            self.begin_footprint_assignment(component);
        }
        workspace::ribbon_separator(ui);
        self.board_route_tools(ui);
        workspace::ribbon_separator(ui);
        let idle = enabled && !crate::board_clipboard::busy(&self.views.active.board);
        let selected_items = self.session.project().is_some_and(|p| !crate::board_clipboard::selection(&self.views.active.board, p).is_empty());
        let clipboard_tools: Vec<_> = [
            ("Copy", "Copy selected PCB items", "Ctrl+C", idle && selected_items),
            ("Paste", "Paste from system clipboard", "Ctrl+V", enabled),
            ("Cut", "Cut selected PCB items", "Ctrl+X", idle && selected_items),
            ("Paste Options…", "Choose PCB paste options", "Ctrl+Shift+V", enabled),
            ("Duplicate", "Duplicate selected PCB items", "Ctrl+D", idle && selected_items),
        ]
        .into_iter()
        .map(|(label, description, shortcut, enabled)| RibbonTool {
            icon: ToolIcon::Sheet,
            label,
            description,
            shortcut,
            enabled,
            active: false,
            menu_check: None,
        })
        .collect();
        match workspace::ribbon_group_with_menu(ui, "Clipboard", &clipboard_tools[..2], &clipboard_tools[2..]) {
            Some(0) => self.copy_board_clipboard(ui.ctx(), false),
            Some(1) => self.request_clipboard(ui.ctx(), false),
            Some(2) => self.copy_board_clipboard(ui.ctx(), true),
            Some(3) => self.request_clipboard(ui.ctx(), true),
            Some(4) => self.duplicate_board_clipboard(),
            _ => {}
        }
        workspace::ribbon_separator(ui);

        let selected = enabled && self.views.active.board.selection.len() == 1;
        let reference = enabled && self.views.active.board.reference.selected.is_some();
        // Move also starts a copper move when tracks or vias are selected and no footprint is.
        let copper = enabled && self.views.active.board.selection.is_empty() && !self.views.active.board.tracks.selected.is_empty();
        let modify = [
            RibbonTool {
                icon: ToolIcon::Move,
                label: "Move",
                description: "Move selected footprint, text or copper",
                shortcut: "M",
                enabled: (selected || reference || copper) && !editing,
                active: editing,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Rotate,
                label: "Rotate",
                description: "Rotate selected footprint or text counterclockwise",
                shortcut: "R",
                enabled: selected || reference || editing,
                active: false,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Mirror,
                label: "Flip",
                description: "Flip footprint to the other side",
                shortcut: "F",
                enabled: selected || self.views.active.board.draft.is_some(),
                active: false,
                menu_check: None,
            },
        ];
        let delete = RibbonTool {
            icon: ToolIcon::Delete,
            label: "Delete",
            description: "Delete selected PCB items",
            shortcut: "Delete",
            enabled: idle && (selected_items || reference),
            active: false,
            menu_check: None,
        };
        let count = self.views.active.board.selection.len();
        let aligns: Vec<RibbonTool> = ALIGNMENTS
            .iter()
            .map(|(label, mode)| RibbonTool {
                icon: ToolIcon::Align,
                label,
                description: label,
                shortcut: "",
                enabled: enabled && !editing && count >= mode.minimum(),
                active: false,
                menu_check: None,
            })
            .collect();
        let menu: Vec<_> = std::iter::once(delete).chain(aligns).collect();
        match workspace::ribbon_group_with_menu(ui, "Modify", &modify, &menu) {
            Some(0) => self.views.active.board.move_requested = true,
            Some(1) => self.views.active.board.rotate_requested = 90,
            Some(2) => self.views.active.board.flip_requested = true,
            Some(3) => self.delete_board_items(),
            Some(index) if (4..4 + ALIGNMENTS.len()).contains(&index) => self.align_footprints(ALIGNMENTS[index - 4].1),
            _ => {}
        }
        workspace::ribbon_separator(ui);
        let design = [
            RibbonTool {
                icon: ToolIcon::Sync,
                label: "Update PCB",
                description: "Update PCB from schematic",
                shortcut: "",
                enabled: self.session.project().is_some() && self.views.active.input(&self.session).is_some(),
                active: false,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Settings,
                label: "Setup",
                description: "Board setup",
                shortcut: "",
                enabled,
                active: false,
                menu_check: None,
            },
        ];
        match workspace::ribbon_group(ui, "Board", &design) {
            Some(0) => self.message = self.begin_board_update(self.views.active.id()).err(),
            Some(1) => self.begin_board_setup(self.views.active.id()),
            _ => {}
        }
    }

    /// Tracks and vias share their layer and dimension controls; planned copper tools
    /// stay in the Route menu and cannot submit commands.
    fn board_route_tools(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let editing = self.views.active.board.editing_footprint();
        let planned =
            |icon, label, description| RibbonTool { icon, label, description, shortcut: "", enabled: false, active: false, menu_check: None };
        let route = [RibbonTool {
            icon: ToolIcon::Track,
            label: "Track",
            description: "Route tracks on the active copper layer",
            shortcut: "X",
            enabled: enabled && self.can_route() && !editing,
            active: self.views.active.board.tracks.tool,
            menu_check: None,
        }];
        let via_shortcut = ui.ctx().format_shortcut(&crate::PLACE_VIA);
        let tools = [
            route[0],
            RibbonTool {
                icon: ToolIcon::Via,
                label: "Via",
                description: "Place vias",
                shortcut: &via_shortcut,
                enabled: enabled && self.can_place_via() && !editing,
                active: self.views.active.board.tracks.via_tool,
                menu_check: None,
            },
        ];
        let planned = [
            planned(ToolIcon::Zone, "Zone", "Draw copper zones (planned)"),
            planned(ToolIcon::Pair, "Pair", "Route differential pairs (planned)"),
            planned(ToolIcon::Tune, "Tune", "Tune track length (planned)"),
            planned(ToolIcon::Refill, "Refill zones", "Refill copper zones (planned)"),
        ];
        match workspace::ribbon_group_with_menu(ui, "Route", &tools, &planned) {
            Some(0) => self.start_route(),
            Some(1) => self.start_via(),
            _ => {}
        }
    }

    /// Draw graphic paths on the active layer; routing and editing remain in Layout.
    fn board_draw_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let editing = self.views.active.board.editing_footprint();
        let can_draw =
            self.session.project().and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target))).is_some_and(
                |board| {
                    crate::board_drawing::can_draw(board, self.views.active.board.active_layer.as_deref(), &self.views.active.board.hidden_layers)
                },
            );
        let outline = [
            RibbonTool {
                icon: ToolIcon::Line,
                label: "Line",
                description: "Draw lines on active layer",
                shortcut: "",
                enabled: enabled && can_draw && !editing,
                active: self.views.active.board.drawing.tool == Some(crate::board_drawing::Tool::Lines),
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Rectangle,
                label: "Rectangle",
                description: "Draw rectangle on active layer",
                shortcut: "",
                enabled: enabled && can_draw && !editing,
                active: self.views.active.board.drawing.tool == Some(crate::board_drawing::Tool::Rectangle),
                menu_check: None,
            },
        ];
        if let Some(index) = workspace::ribbon_group(ui, "Draw", &outline) {
            let tool = if index == 0 { crate::board_drawing::Tool::Lines } else { crate::board_drawing::Tool::Rectangle };
            let layer = self.views.active.board.active_layer.clone();
            self.views.active.board.tracks.cancel();
            self.views.active.board.measure.cancel();
            self.views.active.board.drawing.start(tool, layer);
        }
    }

    /// Persistent layer and dimension fields follow the focused board across every PCB tab.
    pub(super) fn board_ribbon_controls(&mut self, ui: &mut Ui, compact: bool) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        self.active_layer_field(ui, compact);
        self.board_size_controls(ui, enabled, compact);
    }

    /// Analysis opens existing DRC, measurement and counterpart tools without editing the board.
    fn board_analyze_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let checks = [RibbonTool {
            icon: ToolIcon::Drc,
            label: "DRC",
            description: "Run design rules check",
            shortcut: "",
            enabled,
            active: self.views.active.board.drc.open,
            menu_check: None,
        }];
        let planned = [RibbonTool {
            icon: ToolIcon::Rules,
            label: "Rules",
            description: "Edit design rules (planned)",
            shortcut: "",
            enabled: false,
            active: false,
            menu_check: None,
        }];
        if workspace::ribbon_group_with_menu(ui, "Checks", &checks, &planned) == Some(0) {
            self.run_drc();
        }
        workspace::ribbon_separator(ui);
        let measure = [RibbonTool {
            icon: ToolIcon::Measure,
            label: "Measure",
            description: "Measure distance",
            shortcut: "Ctrl+Shift+M",
            enabled,
            active: self.views.active.board.measure.tool,
            menu_check: None,
        }];
        if workspace::ribbon_group(ui, "Measure", &measure).is_some() {
            self.start_measure();
        }
        workspace::ribbon_separator(ui);
        let reveal = [RibbonTool {
            icon: ToolIcon::Symbol,
            label: "Reveal schematic",
            description: "Reveal highlighted counterpart",
            shortcut: "",
            enabled,
            active: false,
            menu_check: None,
        }];
        if workspace::ribbon_group(ui, "Cross-probe", &reveal).is_some() {
            self.views.reveal_highlight(self.views.active.id(), &self.session);
        }
    }

    /// Generate existing fabrication outputs from the focused physical board.
    fn board_manufacture_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let outputs = [RibbonTool {
            icon: ToolIcon::Outputs,
            label: "Outputs",
            description: "Generate fabrication outputs",
            shortcut: "",
            enabled: enabled && !self.fabrication_exports.busy(),
            active: self.fabrication_exports.busy(),
            menu_check: None,
        }];
        if workspace::ribbon_group(ui, "Fabrication", &outputs).is_some() {
            self.export_fabrication(ui.ctx());
        }
    }

    /// The ribbon's active-layer chooser, sharing the Layers card's view-local handler. Hidden
    /// layers stay choosable and say so; choosing one changes only this view's active layer.
    fn active_layer_field(&mut self, ui: &mut Ui, compact: bool) {
        let Some(board) = self.session.project().and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target)))
        else {
            workspace::ribbon_field(ui, "Active layer", compact, |ui| {
                let mut empty = 0usize;
                ui.add_enabled_ui(false, |ui| workspace::layer_choice(ui, "Active layer", &mut empty, &[(0, "No board", egui::Color32::TRANSPARENT)]))
                    .inner
            });
            return;
        };
        let layers = crate::board_panels::ordered_layers(board);
        let labels: Vec<_> = layers
            .iter()
            .map(|layer| {
                let label = crate::board_panels::layer_name(layer, &layers);
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
        let changed = workspace::ribbon_field(ui, "Active layer", compact, |ui| workspace::layer_choice(ui, "Active layer", &mut selected, &options));
        if changed.changed()
            && let Some(layer) = layers.get(selected)
        {
            self.views.active.board.set_layer(&layer.id.to_string(), true, None);
        }
    }

    /// Whether the focused board view's active layer is a visible copper layer that can take tracks.
    fn can_route(&self) -> bool {
        let board = &self.views.active.board;
        self.session
            .project()
            .and_then(|project| board.target.and_then(|target| project.board_view(target)))
            .is_some_and(|view| crate::board_tracks::can_route(view, board.active_layer.as_deref(), &board.hidden_layers))
    }

    /// Generate the focused board's Gerber and drill files as one archive and save it off the
    /// editor loop. A missing setup or outline, or an export already running, is reported in the
    /// status bar. A targeted attempt clears the previous file notice; nothing in the project changes.
    pub(super) fn export_fabrication(&mut self, ctx: &egui::Context) {
        let Some(target) = self.views.active.board.target else { return };
        self.file_notice = None;
        let result = self
            .session
            .export_fabrication(&serde_json::json!({"board": target.board}))
            .map_err(|error| error.to_string())
            .and_then(|outputs| pcbcraft_engine::board::fabrication::zip(&outputs))
            .and_then(|bytes| {
                let name = crate::board_outputs::archive_name(self.session.project().map_or("board", |project| project.name.as_str()));
                self.fabrication_exports.start(ctx, name, bytes)
            });
        self.message = result.err();
    }

    /// Run the design rules check for the focused board view with the default rules, opening its
    /// results card. The check reads the current board and never edits it.
    pub(super) fn run_drc(&mut self) {
        let revision = self.session.revision();
        if let Some(project) = self.session.project()
            && let Some(view) = self.views.active.board.target.and_then(|target| project.board_view(target))
        {
            self.views.active.board.drc.run(view, revision);
        }
    }

    /// Whether the focused board view's board has the two copper layers a through via needs.
    fn can_place_via(&self) -> bool {
        self.session
            .project()
            .and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target)))
            .is_some_and(crate::board_tracks::can_place_via)
    }

    /// Arm Place › Via (Ctrl+Shift+X) in the focused board view when the board has two copper
    /// layers and no footprint transform is in progress. Drawing and route tools are dropped first.
    pub(super) fn start_via(&mut self) {
        if self.views.active.workspace != Workspace::Board || self.views.active.board.editing_footprint() || !self.can_place_via() {
            return;
        }
        self.views.active.board.drawing.cancel();
        self.views.active.board.measure.cancel();
        self.views.active.board.tracks.start_via();
    }

    /// Arm Inspect › Measure (Ctrl+Shift+M) in the focused board view, dropping other tools.
    /// Drops clipboard and placement previews without editing the project or selection.
    pub(super) fn start_measure(&mut self) {
        if self.views.active.workspace != Workspace::Board || self.views.active.board.editing_footprint() {
            return;
        }
        self.views.active.cancel_gesture();
        self.views.active.board.measure.start();
    }

    /// Arm the route tool (KiCad: X) in the focused board view when its active layer is visible
    /// copper and no footprint transform is in progress. Drawing tools are dropped first.
    pub(super) fn start_route(&mut self) {
        if self.views.active.workspace != Workspace::Board || self.views.active.board.editing_footprint() || !self.can_route() {
            return;
        }
        self.views.active.board.drawing.cancel();
        self.views.active.board.measure.cancel();
        self.views.active.board.tracks.start();
    }

    /// Paint any addressed board pane, with input granted only to its focused view. The renderer
    /// returns selection and transform intent; validation and dispatch follow after its borrow ends.
    /// All board panes use the shared grid display preference without changing their cameras.
    /// Rejected drawing creation restores its captured gesture so the user can correct it.
    pub(super) fn board_pane(&mut self, ui: &mut Ui, id: ViewId) {
        let enabled = ui.is_enabled()
            && (crate::IN_BROWSER || ui.input(|input| input.focused))
            && !self.home
            && self.views.active.id() == id
            && matches!(self.dialog, Dialog::None)
            && !self.project_action_pending();
        let owner_enabled = enabled;
        let enabled = enabled && !egui::Popup::is_any_open(ui.ctx());
        if self.session.project().is_some_and(|project| project.board_target().is_none())
            && self.views.view(id).is_some_and(|view| !view.unavailable())
        {
            self.empty_board_pane(ui, id, enabled);
            return;
        }
        let highlighted = self.views.board_highlight_for(id, &self.session);
        let label = if self.views.is_split() { format!("PCB canvas {}", id.0 + 1) } else { "PCB canvas".to_owned() };
        if self.board_clipboard_ready(ui.ctx(), id, enabled)
            && let Some(widget) = self.views.view(id).map(|v| v.widget_id())
        {
            ui.push_id(widget, |ui| shell::canvas(ui, |ui| self.board_clipboard_canvas(ui, id, &label)));
            return;
        }
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
        let markers = view.board.drc.markers(view.board.target);
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
                        owner_enabled,
                        self.preferences.navigation,
                        self.preferences.grid_opacity(true),
                        self.preferences.board_grid_style,
                        &label,
                        &markers,
                        self.preferences.units,
                    )
                })
            })
            .inner;
        if (pick.clipboard.is_some()
            || pick.items.is_some()
            || pick.selection.is_some()
            || pick.edit.is_some()
            || pick.reference_edit.is_some()
            || pick.reference_pick.is_some()
            || pick.drawing_edit.is_some()
            || pick.track_edit.is_some()
            || pick.placement.is_some())
            && (enabled || owner_enabled && (pick.clipboard.is_some() || pick.track_edit.is_some()))
            && source.is_some_and(|source| self.views.active.accepts(source, &self.session))
        {
            if let Some(action) = pick.clipboard {
                self.board_clipboard_action(ui.ctx(), action);
            }
            if let Some(items) = pick.items {
                self.select_board_items(items);
            }
            if let Some(selection) = pick.selection {
                self.views.select_board(selection, &self.session, self.preferences.selection_follow);
            }
            if let Some(footprint) = pick.reference_pick {
                self.views.select_board_reference(footprint, &self.session, self.preferences.selection_follow);
            }
            if let Some(mut edit) = pick.drawing_edit
                && self.views.active.accepts(edit.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                edit.params["board"] = serde_json::json!(target.board);
                self.run(edit.id, &edit.params);
                if self.message.is_some() {
                    edit.restore_rejected(&mut self.views.active.board.drawing);
                }
            }
            if let Some(mut edit) = pick.track_edit
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
                        "rotation": draft.rotation, "side": if draft.bottom { "back" } else { "front" },
                        "expected_revision": draft.source.revision,
                    }),
                );
            }
            if let Some(draft) = pick.reference_edit
                && self.views.active.accepts(draft.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                self.run(
                    pcbcraft_engine::SET_FOOTPRINT_REFERENCE_TEXT,
                    &serde_json::json!({"board":target.board,"footprint":draft.footprint,"appearance":draft.appearance,"offset":draft.offset,"expected_revision":draft.source.revision}),
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
