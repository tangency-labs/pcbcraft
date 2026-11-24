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

    /// The focused view's selected track segments, without vias or stale ids, in board order.
    /// Lazy, so the ribbon's enable check can stop at the second match.
    fn selected_track_ids(&self) -> impl Iterator<Item = uuid::Uuid> + '_ {
        let board = self.views.active.board.target.and_then(|target| self.session.project()?.board_view(target));
        let tracks = board.map_or(&[][..], |board| board.tracks());
        self.views.active.board.tracks.selected.iter().filter(move |id| tracks.binary_search_by_key(*id, |t| t.id).is_ok()).copied()
    }

    /// Merge the focused view's selected track segments through the shared `pcb.merge_tracks`
    /// command. Like other canvas copper edits, it is validated by background copper preparation
    /// and published as one undo step; refusals appear in the status bar.
    pub(super) fn merge_selected_tracks(&mut self) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(target) = self.views.active.board.target else { return };
        let tracks: Vec<_> = self.selected_track_ids().collect();
        if tracks.is_empty() {
            return;
        }
        let params = serde_json::json!({"board": target.board, "tracks": tracks, "expected_revision": source.revision});
        self.prepare_canvas_copper(crate::board_drawing::Command::new(source, pcbcraft_engine::MERGE_TRACKS, params));
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
                view.library = None;
                view.manufacturing = None;
                view.render = None;
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
        let place = [
            RibbonTool {
                icon: ToolIcon::Footprint,
                label: "Footprint",
                description: "Assign and place component footprints",
                shortcut: "",
                enabled: enabled && self.session.project().is_some_and(|p| p.components().next().is_some()),
                active: false,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Zone,
                label: "Zone",
                description: "Draw zone outlines",
                shortcut: "",
                enabled: enabled && self.can_route() && !editing,
                active: self.views.active.board.zones.tool.is_some(),
                menu_check: None,
            },
        ];
        match workspace::ribbon_group(ui, "Place", &place) {
            Some(0) => {
                let component = self
                    .session
                    .project()
                    .and_then(|p| p.board_target().and_then(|target| p.board_view(target)))
                    .and_then(|board| board.footprints().find(|f| self.views.active.board.selection.contains(&f.id)).map(|f| f.component));
                self.begin_footprint_assignment(component);
            }
            Some(1) => self.begin_zone(),
            _ => {}
        }
        workspace::ribbon_separator(ui);
        self.board_route_tools(ui);
        workspace::ribbon_separator(ui);
        self.board_layout_actions(ui);
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

        let group_moving = self.views.active.board.movement.active();
        let selected = enabled && self.views.active.board.selection.len() == 1;
        let reference = enabled && self.views.active.board.reference.selected.is_some();
        // Move also starts a copper move when tracks or vias are selected and no footprint is.
        let copper = enabled && self.views.active.board.selection.is_empty() && !self.views.active.board.tracks.selected.is_empty();
        let modify = [
            RibbonTool {
                icon: ToolIcon::Move,
                label: "Move",
                description: "Move selected PCB items or text",
                shortcut: "M",
                enabled: enabled && (selected_items || reference || copper) && !editing,
                active: editing,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Rotate,
                label: "Rotate",
                description: "Rotate selected footprint or text counterclockwise",
                shortcut: "R",
                enabled: !group_moving && (selected || reference || editing),
                active: false,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Mirror,
                label: "Flip",
                description: "Flip footprint to the other side",
                shortcut: "F",
                enabled: !group_moving && (selected || self.views.active.board.draft.is_some()),
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
        let merge = RibbonTool {
            icon: ToolIcon::Track,
            label: "Merge Tracks",
            description: "Merge selected straight track segments",
            shortcut: "",
            enabled: idle && !editing && !self.views.active.board.tracks.committing() && self.selected_track_ids().nth(1).is_some(),
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
        let menu: Vec<_> = [delete, merge].into_iter().chain(aligns).collect();
        match workspace::ribbon_group_with_menu(ui, "Modify", &modify, &menu) {
            Some(0) => self.views.active.board.move_requested = true,
            Some(1) => self.views.active.board.rotate_requested = 90,
            Some(2) => self.views.active.board.flip_requested = true,
            Some(3) => self.delete_board_items(),
            Some(4) => self.merge_selected_tracks(),
            Some(index) if (5..5 + ALIGNMENTS.len()).contains(&index) => self.align_footprints(ALIGNMENTS[index - 5].1),
            _ => {}
        }
    }

    /// Board-wide actions operate on the focused board. Removing pours keeps the authored zones.
    fn board_layout_actions(&mut self, ui: &mut Ui) {
        let project = self.session.project();
        let enabled = project.and_then(|project| project.board_target()).is_some();
        let board = project.and_then(|project| self.views.active.board.target.and_then(|target| project.board_view(target)));
        let can_pour = !self.views.active.board.editing_footprint() && board.is_some_and(|board| !board.zones().is_empty());
        let fill_shortcut = ui.ctx().format_shortcut(&crate::POUR_COPPER);
        let unfill_shortcut = ui.ctx().format_shortcut(&crate::REMOVE_POURS);
        let tools = [
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
            RibbonTool {
                icon: ToolIcon::Refill,
                label: "Pour copper",
                description: "Fill zone outlines with copper",
                shortcut: &fill_shortcut,
                enabled: can_pour,
                active: false,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Delete,
                label: "Remove pours",
                description: "Remove poured copper and keep zone outlines",
                shortcut: &unfill_shortcut,
                enabled: can_pour && board.is_some_and(|board| board.zone_fill_enabled()),
                active: false,
                menu_check: None,
            },
        ];
        match workspace::ribbon_group(ui, "Board", &tools) {
            Some(0) => self.message = self.begin_board_update(self.views.active.id()).err(),
            Some(1) => self.begin_board_setup(self.views.active.id()),
            Some(2) => self.refill_zones(),
            Some(3) => self.unfill_zones(),
            _ => {}
        }
    }

    /// Tracks and vias share layer and dimension controls. Unfinished differential-pair and
    /// length-tuning tools stay disabled in the shared Route menu.
    fn board_route_tools(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let editing = self.views.active.board.editing_footprint();
        let planned =
            |icon, label, description| RibbonTool { icon, label, description, shortcut: "", enabled: false, active: false, menu_check: None };
        let via_shortcut = ui.ctx().format_shortcut(&crate::PLACE_VIA);
        let tools = [
            RibbonTool {
                icon: ToolIcon::Track,
                label: "Track",
                description: "Route tracks on the active copper layer",
                shortcut: "X",
                enabled: enabled && self.can_route() && !editing,
                active: self.views.active.board.tracks.tool,
                menu_check: None,
            },
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
        let menu = [
            RibbonTool {
                icon: ToolIcon::Settings,
                label: "Router Settings",
                description: "Interactive Router Settings",
                shortcut: "",
                enabled,
                active: self.views.active.board.router_settings_open,
                menu_check: Some(pcbcraft_chrome::menu::CheckKind::Toggle),
            },
            planned(ToolIcon::Pair, "Pair", "Route differential pairs (planned)"),
            planned(ToolIcon::Tune, "Tune", "Tune track length (planned)"),
        ];
        match workspace::ribbon_group_with_menu(ui, "Route", &tools, &menu) {
            Some(0) => self.start_route(),
            Some(1) => self.start_via(),
            Some(2) => {
                let view = &mut self.views.active;
                view.board.router_settings_open = !view.board.router_settings_open;
                if !view.board.router_settings_open {
                    view.panels[crate::workspace_panels::ROUTER].cancel_drag();
                }
            }
            _ => {}
        }
    }

    /// Draw live, view-local controls in the same movable shell as Layers and Properties.
    /// An in-progress edit keeps its captured policy and may continue outside the card.
    pub(super) fn router_settings_panel(&mut self, ui: &Ui, area: egui::Rect, dock: egui::Rect) -> egui::Rect {
        use pcbcraft_chrome::{controls, settings, workspace::panels::Position};
        use pcbcraft_engine::board::route::Mode;

        let enabled = matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
        let view = &mut self.views.active;
        if !view.board.router_settings_open {
            return egui::Rect::NOTHING;
        }
        let id = egui::Id::unique(("router_settings", self.views.host, view.id()));
        let state = &mut view.board;
        let panel = &mut view.panels[crate::workspace_panels::ROUTER];
        let width = workspace::panels::card_width(area, false);
        if panel.position() == Position::Floating && panel.saved_state().at.is_none() {
            panel.set_position(Position::Floating, Some([(area.width() - width) / 2.0, 20.0]));
        }
        let available = enabled && !state.editing_footprint() && state.tracks.can_change_policy();
        let (original, original_merge) = state.tracks.policy();
        let (mut mode, mut merge) = (original, original_merge);
        let mut close = false;
        let (rect, toggle) = panel.show(ui, id, "Router Settings", area, dock, state.router_settings_collapsed, width, |ui| {
            ui.add_enabled_ui(available, |ui| {
                let mut policy = if mode == Mode::ShoveVias { Mode::Shove } else { mode };
                ui.label("Policy");
                if settings::choice(
                    ui,
                    "Router policy",
                    &mut policy,
                    &[(Mode::Walkaround, "Walkaround"), (Mode::Shove, "Shove"), (Mode::Highlight, "Highlight collisions")],
                ) {
                    mode = policy;
                }
                if policy == Mode::Shove {
                    let mut vias = mode == Mode::ShoveVias;
                    if settings::checkbox(ui, "Shove through vias", "Shove through vias", &mut vias).changed() {
                        mode = if vias { Mode::ShoveVias } else { Mode::Shove };
                    }
                    settings::checkbox(ui, "Merge same-net copper", "Merge same-net copper", &mut merge);
                }
            })
            .response
            .on_disabled_hover_text("Finish or cancel the current edit to change router policy.");
            let dismiss = controls::button(ui, "Close");
            dismiss.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Close Router Settings"));
            close = dismiss.clicked();
        });
        if available {
            if mode != original {
                state.tracks.select_mode(mode);
            }
            if merge != original_merge {
                state.tracks.select_same_net_merge(merge);
            }
        }
        state.router_settings_collapsed ^= toggle;
        if close {
            state.router_settings_open = false;
            panel.cancel_drag();
        }
        rect
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
        if self.views.active.board.tracks.via_tool {
            let revision = self.session.revision();
            if let Some(project) = self.session.project() {
                let state = &mut self.views.active.board.tracks;
                if let Some(board) = project.board_target() {
                    if let Some(view) = project.board_view(board) {
                        state.refresh_net_choices(view, revision);
                    }
                    let choices = &state.via_choices.as_ref().map(|(_, _, c)| c.as_slice()).unwrap_or_default();
                    let current = state.via_target.as_ref().map_or_else(
                        || "Automatic".to_owned(),
                        |target| choices.iter().find(|c| &c.target == target).map_or_else(|| "Selected net".into(), |c| c.label.clone()),
                    );
                    let response =
                        workspace::ribbon_field(ui, "Via net", compact, |ui| crate::board_net_choice::trigger(ui, "Via placement net", &current));
                    if response.clicked()
                        && let Some(source) = self.views.active.input(&self.session)
                    {
                        self.begin_via_net_choice(source, crate::board_net_choice::Destination::Placement);
                    }
                }
            }
        }
    }

    /// Analysis keeps physical checks board-gated while shared class authoring needs only a project.
    fn board_analyze_tab(&mut self, ui: &mut Ui) {
        let enabled = self.session.project().and_then(|project| project.board_target()).is_some();
        let checks = [
            RibbonTool {
                icon: ToolIcon::Drc,
                label: "DRC",
                description: "Run design rules check",
                shortcut: "",
                enabled,
                active: self.views.active.board.drc.open,
                menu_check: None,
            },
            RibbonTool {
                icon: ToolIcon::Rules,
                label: "Rules",
                description: "Edit net classes, assignments and board defaults.",
                shortcut: "",
                enabled: self.session.project().is_some(),
                active: false,
                menu_check: None,
            },
        ];
        match workspace::ribbon_group(ui, "Checks", &checks) {
            Some(0) => self.run_drc(),
            Some(1) => self.begin_rules(),
            _ => {}
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
            self.views.reveal_highlight(self.views.active.id(), &self.session, self.preferences.selection_follow);
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
        let review = RibbonTool {
            icon: ToolIcon::Factory,
            label: "Manufacturing",
            description: "Open the Manufacturing workspace",
            shortcut: "",
            enabled: true,
            active: false,
            menu_check: None,
        };
        match workspace::ribbon_group(ui, "Fabrication", &[outputs[0], review]) {
            Some(0) => self.export_fabrication(ui.ctx()),
            Some(1) => self.message = self.enter_workspace(ui.ctx(), "manufacturing", Placement::Here).err(),
            _ => {}
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

    /// Capture the focused board's complete ZIP on the editor loop. The native worker saves it
    /// at the chosen path; the browser path requests one download.
    /// Missing setup or outline and a busy export slot are reported without editing the project.
    pub(super) fn export_fabrication(&mut self, ctx: &egui::Context) {
        let Some(target) = self.views.active.board.target else { return };
        self.file_notice = None;
        let result = self
            .session
            .export_fabrication_archive(&serde_json::json!({"board": target.board}))
            .map_err(|error| error.to_string())
            .and_then(|archive| {
                let name = pcbcraft_engine::board::fabrication::archive_name(self.session.project().map_or("board", |project| project.name.as_str()));
                let source = crate::board_outputs::Source { board: target, opening: self.session.history_root_revision() };
                let directory = self.files.project_directory(&self.session).map(std::path::Path::to_owned);
                self.fabrication_exports.start(ctx, source, name, archive, directory)
            });
        self.message = result.err();
    }

    /// Run the design rules check for the focused board view with the default rules, opening its
    /// results card. The check reads the current board and never edits it.
    pub(super) fn run_drc(&mut self) {
        if let Some(target) = self.views.active.board.target {
            self.message = self.views.active.board.drc.run(&self.session, target).err();
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
        self.views.active.board.zones.cancel();
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
        self.views.active.board.zones.cancel();
        self.views.active.board.measure.cancel();
        self.views.active.board.tracks.start();
    }

    /// Paint any addressed board pane, with input granted only to its focused view. The renderer
    /// returns selection and transform intent; validation and dispatch follow after its borrow ends.
    /// All board panes use the shared grid display preference without changing their cameras.
    /// Rejected drawing creation restores its captured gesture so the user can correct it.
    /// An accepted zone creation renews its armed net choice for the next outline only.
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
        if self.board_move_ready(ui.ctx(), id, enabled)
            && let Some(widget) = self.views.view(id).map(|v| v.widget_id())
        {
            ui.push_id(widget, |ui| shell::canvas(ui, |ui| self.board_move_canvas(ui, id, &label)));
            return;
        }
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
                        &view.inspection,
                        &markers,
                        self.preferences.units,
                    )
                })
            })
            .inner;
        if (pick.inspection.is_some()
            || pick.clipboard.is_some()
            || pick.error.is_some()
            || pick.items.is_some()
            || pick.selection.is_some()
            || pick.edit.is_some()
            || pick.reference_edit.is_some()
            || pick.reference_pick.is_some()
            || pick.zone_edit.is_some()
            || pick.drawing_edit.is_some()
            || pick.track_edit.is_some()
            || pick.placement.is_some())
            && (enabled || owner_enabled && (pick.clipboard.is_some() || pick.track_edit.is_some() || pick.vertex_menu_edit))
            && source.is_some_and(|source| self.views.active.accepts(source, &self.session))
        {
            if let Some((source, error)) = pick.error
                && self.views.active.accepts(source, &self.session)
            {
                self.message = Some(error);
            }
            if let Some((pad, modifiers)) = pick.inspection {
                self.inspect_pad(pad, modifiers);
            }
            if let Some(action) = pick.clipboard {
                self.board_clipboard_action(ui.ctx(), action);
            }
            let previous_inspection =
                (pick.extend_inspection && (pick.items.is_some() || pick.selection.is_some())).then(|| self.views.active.inspection.clone());
            if let Some(items) = pick.items {
                self.select_board_items(items);
            }
            if let Some(selection) = pick.selection {
                self.views.select_board(selection, &self.session, self.preferences.selection_follow);
            }
            if let Some(inspection) = previous_inspection {
                self.views.active.inspection = inspection;
            }
            if let Some(footprint) = pick.reference_pick {
                self.views.select_board_reference(footprint, &self.session, self.preferences.selection_follow);
            }
            if let Some(mut edit) = pick.zone_edit
                && self.views.active.accepts(edit.source, &self.session)
                && let Some(target) = self.views.active.board.target
            {
                edit.params["board"] = serde_json::json!(target.board);
                self.run(edit.id, &edit.params);
                if self.message.is_some() {
                    edit.restore_rejected(&mut self.views.active.board.zones);
                } else if edit.id == pcbcraft_engine::board::zones::ADD_ZONE {
                    // Only this tool's accepted creation renews its chosen net; unrelated edits
                    // leave the captured authority stale and cancel before another point starts.
                    self.views.active.board.zones.zone_source = self.views.active.input(&self.session);
                }
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
                self.prepare_canvas_copper(edit);
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

    /// Explain the board workflow without creating a physical owner. The schematic shortcut
    /// queues ordinary pane navigation; Create board opens a disposable setup draft. Applying
    /// that draft creates the board atomically, while Cancel leaves the project untouched.
    /// Inactive panes cannot dispatch either action.
    fn empty_board_pane(&mut self, ui: &mut Ui, id: ViewId, enabled: bool) {
        let Some(widget_id) = self.views.view(id).map(|view| view.widget_id()) else { return };
        let action = ui
            .push_id(widget_id, |ui| {
                shell::canvas(ui, |ui| {
                    pcbcraft_chrome::empty_state::show(
                        ui,
                        "Start your PCB layout",
                        "This project doesn’t have a board yet. Create one to open Board setup, then add footprints and route your circuit.",
                        ["Create board", "Open schematic"],
                        &[
                            "Set up layers and design rules in Board setup",
                            "Use Update PCB to review and add schematic footprints",
                            "Draw the board outline and route connections",
                        ],
                        enabled,
                    )
                })
            })
            .inner;
        if action == pcbcraft_chrome::empty_state::Action::Secondary && enabled {
            self.queue_view_action(
                ui.ctx(),
                id,
                crate::editor_windows::ViewAction::Workspace { workspace: Workspace::Schematic, placement: Placement::Here },
            );
        }
        if action == pcbcraft_chrome::empty_state::Action::Primary && enabled && self.session.available("pcb.create_board") {
            self.begin_board_setup(id);
            self.repaint_editors(ui.ctx());
        }
    }
}

#[cfg(test)]
mod pour_button_tests {
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    /// The real Board controls address the focused view; an unavailable target cannot inherit
    /// the default board's pours. Render the ribbon directly so frame binding cannot repair it.
    #[test]
    fn pour_actions_follow_the_focused_board_target() {
        let mut fixture = crate::tests::fill_harness();
        let app = std::mem::take(fixture.state_mut());
        let mut h = Harness::builder()
            .with_context_setup(pcbcraft_chrome::install)
            .with_size(egui::vec2(1200., 700.))
            .build_ui_state(|ui, app: &mut crate::PcbApp| app.board_layout_actions(ui), app);
        h.run();
        assert!(!h.get_by_label("Fill zone outlines with copper").accesskit_node().is_disabled());
        assert!(!h.get_by_label("Remove poured copper and keep zone outlines").accesskit_node().is_disabled());
        h.state_mut().views.active.board.target = None;
        h.run();
        assert!(h.get_by_label("Fill zone outlines with copper").accesskit_node().is_disabled());
        assert!(h.get_by_label("Remove poured copper and keep zone outlines").accesskit_node().is_disabled());
    }
}
