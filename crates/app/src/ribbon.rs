//! PCBcraft Next's grouped ribbon adapted to the upstream command and view owners.
//! Presentation tables are qualified in ATTRIBUTION.md; no prototype engine is imported.

use crate::{Dialog, PcbApp, Selection, SymbolAction, Tool, Workspace, editor_windows};
use pcbcraft_chrome::workspace::{self, RibbonTool, ToolIcon as Icon};
use serde_json::json;

/// Stable saved indices retain legacy View=1 while visual tab order follows PCBcraft Next.
pub(super) const TAB_ORDER: [usize; 5] = [0, 2, 3, 4, 1];
/// PCB shares the saved Libraries and View slots with schematic hosts.
pub(super) const BOARD_TAB_ORDER: [usize; 3] = [0, 3, 1];

/// Names shared by live control inspection and its explicit tab-selection command.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn tab_name(slot: usize) -> &'static str {
    match slot {
        1 => "view",
        2 => "analyze",
        3 => "libraries",
        4 => "manage",
        _ => "capture",
    }
}

/// Unknown names are already rejected by command validation; legacy Layout selects slot zero.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn tab_slot(name: &str) -> usize {
    match name {
        "view" => 1,
        "analyze" => 2,
        "libraries" => 3,
        "manage" => 4,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Symbol,
    Power,
    HierarchicalLabel,
    Text,
    NewSheet,
    Subsheet,
    Wire,
    Bus,
    Label,
    Move,
    Rotate,
    Mirror,
    Rules,
    Erc,
    Annotate,
    Bom,
    Sync,
    Delete,
    Slice,
    Heal,
    AutoHeal,
    OpenLibraryItem,
    ResumeLibraryEditors,
    EditDefinitionCopy,
    Sources,
    Updates,
    Variants,
}

/// A single metadata table drives both promoted controls and caption-menu actions.
#[derive(Clone, Copy)]
struct ToolSpec {
    action: Action,
    label: &'static str,
    icon: Icon,
    hint: &'static str,
    key: &'static str,
}
struct Group {
    title: &'static str,
    tools: &'static [ToolSpec],
    extras: &'static [ToolSpec],
}

const fn tool(action: Action, label: &'static str, icon: Icon, hint: &'static str, key: &'static str) -> ToolSpec {
    ToolSpec { action, label, icon, hint, key }
}
const SHEETS: &[ToolSpec] = &[
    tool(Action::Subsheet, "Subsheet", Icon::Subsheet, "Place hierarchical sheet (S)", "S"),
    tool(Action::HierarchicalLabel, "Hierarchical label", Icon::HierarchicalLabel, "Place hierarchical label (H)", "H"),
];
const CAPTURE: &[Group] = &[
    Group {
        title: "Place",
        tools: &[
            tool(Action::Symbol, "Symbol", Icon::Symbol, "Add symbol (A)", "A"),
            tool(Action::Power, "Power", Icon::Power, "Place power or ground (P)", "P"),
            tool(Action::Text, "Text", Icon::Text, "Text placement is not available yet.", "T"),
        ],
        extras: &[],
    },
    Group {
        title: "Connect",
        tools: &[
            tool(Action::Wire, "Wire", Icon::Wire, "Draw wire (W)", "W"),
            tool(Action::Bus, "Bus", Icon::Bus, "Buses are not available yet.", ""),
            tool(Action::Label, "Net label", Icon::NetLabel, "Place local label (L); choose Global label in Type for project scope", "L"),
        ],
        extras: &[],
    },
    Group { title: "Sheets", tools: SHEETS, extras: &[] },
    Group {
        title: "Modify",
        tools: &[
            tool(Action::Move, "Move", Icon::Move, "Move selected object (M)", "M"),
            tool(Action::Rotate, "Rotate", Icon::Rotate, "Rotate selected object", "R"),
            tool(Action::Mirror, "Mirror", Icon::Mirror, "Mirroring is not available yet.", ""),
        ],
        extras: &[
            tool(Action::Delete, "Delete", Icon::Delete, "Delete selected items", "Del"),
            tool(Action::Slice, "Slice", Icon::Slice, "Slice selected wire", ""),
            tool(Action::Heal, "Heal", Icon::Heal, "Heal selected wires", ""),
            tool(Action::AutoHeal, "Auto-heal wires", Icon::Heal, "Auto-heal wires", ""),
        ],
    },
];
const ANALYZE: &[Group] = &[
    Group {
        title: "Electrical",
        tools: &[
            tool(Action::Erc, "ERC", Icon::Erc, "Electrical rule checking is not available yet.", ""),
            tool(Action::Rules, "Rules", Icon::Rules, "Electrical rules are not available yet.", ""),
        ],
        extras: &[],
    },
    Group { title: "Reports", tools: &[tool(Action::Bom, "BOM", Icon::Bom, "Bill of materials generation is not available yet.", "")], extras: &[] },
];
const LIBRARIES: &[Group] = &[
    Group {
        title: "Editors",
        tools: &[
            tool(Action::OpenLibraryItem, "Open", Icon::Browser, "Open a saved library item", ""),
            tool(Action::ResumeLibraryEditors, "Resume", Icon::EditorWindow, "Return to open library editors", ""),
        ],
        extras: &[],
    },
    Group {
        title: "Selected definition",
        tools: &[tool(Action::EditDefinitionCopy, "Edit copy", Icon::Symbol, "Edit the selected definition as an independent copy", "")],
        extras: &[],
    },
    Group {
        title: "Library sources",
        tools: &[
            tool(Action::Sources, "Sources", Icon::Browser, "Manage native library sources", ""),
            tool(Action::Updates, "Updates", Icon::Sync, "Library update review is not available yet.", ""),
        ],
        extras: &[],
    },
];
const MANAGE: &[Group] = &[
    Group { title: "Sheets", tools: &[tool(Action::NewSheet, "New sheet", Icon::Sheet, "Create an independent sheet", "")], extras: &[] },
    Group {
        title: "Project",
        tools: &[
            tool(Action::Annotate, "Annotate", Icon::Annotate, "Automatic annotation is not available yet.", ""),
            tool(Action::Variants, "Variants", Icon::Bom, "Assembly variants are not available yet.", ""),
            tool(Action::Sync, "Update PCB", Icon::Sync, "Schematic-to-board synchronization is not available yet.", ""),
        ],
        extras: &[],
    },
];

impl Action {
    /// Keep future controls visible on wide ribbons but out of compact promoted rows.
    fn implemented(self) -> bool {
        matches!(
            self,
            Self::Power
                | Self::Label
                | Self::Symbol
                | Self::Sources
                | Self::OpenLibraryItem
                | Self::ResumeLibraryEditors
                | Self::EditDefinitionCopy
                | Self::HierarchicalLabel
                | Self::NewSheet
                | Self::Subsheet
                | Self::Wire
                | Self::Move
                | Self::Rotate
                | Self::Delete
                | Self::Slice
                | Self::Heal
                | Self::AutoHeal
        )
    }

    /// Availability uses the same current view/session state as the existing command handlers.
    fn enabled(self, app: &PcbApp) -> bool {
        if !self.implemented() {
            return false;
        }
        let project = app.session.project().is_some();
        let schematic = app.views.active.workspace == Workspace::Schematic && !app.views.active.unavailable();
        match self {
            Self::Sources => true,
            Self::OpenLibraryItem => !app.library_editors.busy(),
            Self::ResumeLibraryEditors => app.library_editors.has_documents(),
            Self::EditDefinitionCopy => {
                project
                    && !app.views.active.unavailable()
                    && match app.views.active.workspace {
                        Workspace::Schematic => app.views.active.schematic.selection.single_symbol().is_some(),
                        Workspace::Board => {
                            app.views.active.board.selection.len() == 1
                                && app.views.active.snapshot(&app.session).is_some_and(|project| {
                                    project.board_view(project.board_target()).is_some_and(|board| {
                                        board
                                            .footprints()
                                            .any(|footprint| footprint.physical_reference && app.views.active.board.selection.contains(&footprint.id))
                                    })
                                })
                        }
                    }
            }
            Self::NewSheet => app.session.available(pcbcraft_engine::sheets::CREATE_SHEET),
            Self::Move | Self::Rotate => {
                schematic
                    && app.views.active.schematic.tool.is_none()
                    && (app.views.active.schematic.selection.single_symbol().is_some() || app.views.active.schematic.selected_net_label.is_some())
            }
            Self::Delete => {
                schematic
                    && (!app.views.active.schematic.selection.is_empty()
                        || app.views.active.schematic.selected_sheet_symbol.is_some()
                        || app.views.active.schematic.selected_net_label.is_some())
            }
            Self::Slice => schematic && app.views.active.schematic.selected_leg.as_ref().is_some_and(|leg| leg.split.is_some()),
            Self::Heal => {
                schematic && !app.views.active.schematic.selection.wires.is_empty() && app.views.active.schematic.selection.symbols.is_empty()
            }
            _ => project && schematic,
        }
    }

    /// Tool/toggle ink is local presentation state and never changes authored data.
    fn active(self, app: &PcbApp) -> bool {
        match self {
            Self::Symbol => matches!(app.views.active.schematic.tool, Some(Tool::PlaceSymbol { .. })),
            Self::HierarchicalLabel => matches!(app.views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Label, .. })),
            Self::Subsheet => matches!(app.views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Sheet, .. })),
            Self::Wire => matches!(app.views.active.schematic.tool, Some(Tool::Wire { .. })),
            Self::AutoHeal => app.preferences.auto_heal_wires,
            _ => false,
        }
    }
}

impl PcbApp {
    /// Render the ported group definitions with the upstream chrome and command authority.
    /// Compact ribbons retain working controls; planned tools remain in caption menus.
    pub(super) fn ported_ribbon_tab(&mut self, ui: &mut egui::Ui, slot: usize) {
        let groups = match slot {
            2 => ANALYZE,
            3 => LIBRARIES,
            4 => MANAGE,
            _ => CAPTURE,
        };
        let compact = workspace::ribbon_is_compact(ui);
        for (index, group) in groups.iter().enumerate() {
            if index > 0 {
                workspace::ribbon_separator(ui);
            }
            let promoted: Vec<_> = group.tools.iter().copied().filter(|t| !compact || t.action.implemented()).collect();
            let extra: Vec<_> =
                group.tools.iter().copied().filter(|t| compact && !t.action.implemented()).chain(group.extras.iter().copied()).collect();
            let render = |t: &ToolSpec| RibbonTool {
                icon: t.icon,
                label: t.label,
                description: t.hint,
                shortcut: t.key,
                enabled: t.action.enabled(self),
                active: t.action.active(self),
                toggle: t.action == Action::AutoHeal,
            };
            let tools: Vec<_> = promoted.iter().map(render).collect();
            let menu: Vec<_> = extra.iter().map(render).collect();
            if let Some(picked) = workspace::ribbon_group_with_menu(ui, group.title, &tools, &menu)
                && let Some(tool) = promoted.iter().chain(&extra).nth(picked)
            {
                self.ribbon_action(ui.ctx(), tool.action);
            }
        }
    }

    /// Ported toolbar actions dispatch through the existing revision-checked engine/view paths.
    fn ribbon_action(&mut self, ctx: &egui::Context, action: Action) {
        if !action.enabled(self) {
            return;
        }
        match action {
            Action::Sources => self.dialog = Dialog::LibrarySources,
            Action::OpenLibraryItem => self.message = self.open_library_picker(ctx).err(),
            Action::ResumeLibraryEditors => self.resume_library_workspace(ctx),
            Action::EditDefinitionCopy => {
                let kind = match self.views.active.workspace {
                    Workspace::Schematic => pcbcraft_engine::authoring::Kind::Symbol,
                    Workspace::Board => pcbcraft_engine::authoring::Kind::Footprint,
                };
                self.message = self.fork_library_selection(kind).err();
            }
            Action::Symbol => self.parts.focus_search(),
            Action::Power => self.parts.focus_power(),
            Action::Label => self.begin_net_label(pcbcraft_engine::net_labels::Style::Local),
            Action::HierarchicalLabel => self.begin_hierarchy(crate::hierarchy::Kind::Label),
            Action::Subsheet => self.begin_hierarchy(crate::hierarchy::Kind::Sheet),
            Action::NewSheet => {
                self.begin_sheet_edit(self.views.active.id(), pcbcraft_engine::sheets::CREATE_SHEET, None);
                if matches!(self.dialog, Dialog::Sheet(_)) {
                    ctx.send_viewport_cmd_to(editor_windows::viewport(self.views.main), egui::ViewportCommand::Focus);
                }
            }
            Action::Wire => self.begin_wire(),
            Action::Move | Action::Rotate => {
                if let Some(label) = self
                    .views
                    .active
                    .snapshot(&self.session)
                    .and_then(|p| p.net_labels().iter().find(|l| Some(l.id) == self.views.active.schematic.selected_net_label).cloned())
                {
                    if let Some(source) = self.views.active.input(&self.session) {
                        self.net_label_action(
                            source,
                            label,
                            if action == Action::Move { crate::net_labels::Action::Move } else { crate::net_labels::Action::Rotate },
                        );
                    }
                } else {
                    self.edit_selected(if action == Action::Move { SymbolAction::DisconnectMove } else { SymbolAction::Rotate });
                }
            }
            Action::Delete => {
                if let Some(label) = self
                    .views
                    .active
                    .snapshot(&self.session)
                    .and_then(|p| p.net_labels().iter().find(|l| Some(l.id) == self.views.active.schematic.selected_net_label).cloned())
                {
                    if let Some(source) = self.views.active.input(&self.session) {
                        self.net_label_action(source, label, crate::net_labels::Action::Delete);
                    }
                } else if let Some(id) = self.views.active.schematic.selected_sheet_symbol {
                    if let Some(source) = self.views.active.input(&self.session) {
                        self.hierarchy_action(source, crate::hierarchy::Kind::Sheet, id, crate::hierarchy::Action::Delete);
                    }
                } else {
                    self.delete_selection();
                }
            }
            Action::Slice => {
                if let Some(leg) = self.views.active.schematic.selected_leg.clone() {
                    self.edit_wire(crate::schematic_canvas::WireAction::Slice, &leg);
                }
            }
            Action::Heal => {
                self.run(
                    pcbcraft_engine::HEAL_WIRES,
                    &json!({"items":self.views.active.schematic.selection,"expected_revision":self.session.revision()}),
                );
                if self.message.is_none() {
                    self.select(Selection::default());
                }
            }
            Action::AutoHeal => self.preferences.auto_heal_wires = !self.preferences.auto_heal_wires,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Editing and project actions have one tab owner, even when planned controls are disabled.
    #[test]
    fn actions_are_not_repeated_across_tabs() {
        let mut owners = Vec::new();
        for (tab, groups) in [("Capture", CAPTURE), ("Analyze", ANALYZE), ("Libraries", LIBRARIES), ("Manage", MANAGE)] {
            for tool in groups.iter().flat_map(|group| group.tools.iter().chain(group.extras)) {
                assert!(!owners.iter().any(|(action, _)| *action == tool.action), "{:?} repeated on {tab}: {owners:?}", tool.action);
                owners.push((tool.action, tab));
            }
        }
    }
}
