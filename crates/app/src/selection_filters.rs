//! Per-view selection policy, shared by canvas picking, Browser actions and live MCP.
use crate::{
    PcbApp,
    editor_view::{ViewId, Workspace},
};
use pcbcraft_engine::selection::Selection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Supported selectable categories. Missing fields in older local arrangements default to on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Filters {
    pub symbols: bool,
    pub wires: bool,
    pub sheets: bool,
    pub labels: bool,
    pub footprints: bool,
    pub graphics: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self { symbols: true, wires: true, sheets: true, labels: true, footprints: true, graphics: true }
    }
}

impl Filters {
    /// Power/ground anchors follow Symbols; local and global names follow Labels.
    pub(super) fn allows_net_label(self, style: pcbcraft_engine::net_labels::Style) -> bool {
        use pcbcraft_engine::net_labels::Style;
        match style {
            Style::Power | Style::Ground => self.symbols,
            Style::Local | Style::Global => self.labels,
        }
    }

    /// Report only selectable categories for the displayed workspace; parked workspace choices
    /// remain in local state without advertising unsupported selection capabilities.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(self, workspace: Workspace) -> Value {
        Value::Object(self.rows(workspace).into_iter().filter(|row| row.enabled).map(|row| (row.key.to_owned(), Value::Bool(row.checked))).collect())
    }

    /// Restrict components, wires and named anchors without changing geometry or connectivity.
    /// Named-anchor styles are resolved on the addressed sheet; absent anchors are dropped.
    pub(super) fn retain(self, selection: &mut Selection, project: Option<&pcbcraft_engine::Project>) {
        selection
            .net_labels
            .retain(|id| project.is_some_and(|p| p.net_labels().iter().any(|label| label.object_id() == *id && self.allows_net_label(label.style))));
        if !self.symbols {
            selection.symbols.clear();
        }
        if !self.wires {
            selection.wires.clear();
        }
    }

    /// Build the workspace's supported rows and the disabled categories shown in FigCraft.
    fn rows(self, workspace: Workspace) -> Vec<pcbcraft_chrome::selection_filters::Filter<'static>> {
        use pcbcraft_chrome::selection_filters::Filter;
        let row = |key, label, checked| Filter { key, label, checked, enabled: true };
        let planned = |key, label| Filter { key, label, checked: false, enabled: false };
        match workspace {
            Workspace::Schematic => vec![
                row("symbols", "Symbols", self.symbols),
                planned("pins", "Pins"),
                row("wires", "Wires", self.wires),
                row("labels", "Labels", self.labels),
                planned("graphics", "Graphics"),
                planned("images", "Images"),
                planned("text", "Text"),
                row("sheets", "Sheets", self.sheets),
                planned("rule_areas", "Rule areas"),
            ],
            Workspace::Board => vec![
                row("footprints", "Footprints", self.footprints),
                planned("pads", "Pads"),
                planned("tracks", "Tracks"),
                planned("vias", "Vias"),
                planned("zones", "Zones"),
                planned("text", "Text"),
                row("graphics", "Graphics", self.graphics),
                planned("dimensions", "Dimensions"),
            ],
        }
    }
}

impl PcbApp {
    /// Atomically apply view-only filter changes, refusing unsupported categories. Dropping a
    /// category clears its edit selection and cancels captured gestures so a later release cannot edit it.
    pub(super) fn set_selection_filters(&mut self, id: ViewId, params: &Value) -> Result<(), String> {
        let view = self.views.view(id).filter(|v| v.definition.is_none()).ok_or("Selection filters require a project view")?;
        let mut next = view.filters;
        for (key, value) in params.as_object().ok_or("Selection filters must be an object")? {
            let field = match (view.workspace, key.as_str()) {
                (Workspace::Schematic, "symbols") => &mut next.symbols,
                (Workspace::Schematic, "wires") => &mut next.wires,
                (Workspace::Schematic, "sheets") => &mut next.sheets,
                (Workspace::Schematic, "labels") => &mut next.labels,
                (Workspace::Board, "footprints") => &mut next.footprints,
                (Workspace::Board, "graphics") => &mut next.graphics,
                (_, "view" | "collapsed" | "open") => continue,
                _ => return Err(format!("Selection filter `{key}` is unavailable in this workspace")),
            };
            *field = value.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?;
        }
        let collapsed = params.get("collapsed").map(|v| v.as_bool().ok_or("`collapsed` must be true or false")).transpose()?;
        let open = params.get("open").map(|v| v.as_bool().ok_or("`open` must be true or false")).transpose()?;
        let project = view.snapshot(&self.session);
        let view = self.views.view_mut(id).ok_or("The addressed view closed")?;
        if next != view.filters {
            view.release_input();
            view.filters = next;
            next.retain(&mut view.schematic.selection, project.as_deref());
            if !next.wires {
                view.schematic.selected_leg = None;
                view.schematic.whole_wire_drag = false;
            }
            if !next.sheets {
                view.schematic.selected_sheet_symbol = None;
            }
            if !next.footprints {
                view.board.selection.clear();
            }
            if !next.graphics {
                view.board.drawing.selected = None;
            }
        }
        if let Some(open) = open {
            if !open {
                view.panels[crate::workspace_panels::FILTERS].cancel_drag();
            }
            view.filters_open = open;
        }
        if let Some(collapsed) = collapsed {
            view.filters_collapsed = collapsed;
        }
        Ok(())
    }

    /// Draw the focused view's filters below Properties and dispatch changes through the same
    /// handler as live MCP. The card keeps unsupported model categories visibly unavailable.
    pub(super) fn selection_filters(&mut self, ui: &egui::Ui, canvas: egui::Rect, area: egui::Rect) -> egui::Rect {
        let view = &self.views.active;
        let id = view.id();
        let mut rows = view.filters.rows(view.workspace);
        let collapsed = view.filters_collapsed;
        let enabled = matches!(self.dialog, crate::Dialog::None) && !self.project_action_pending();
        let mut changed = false;
        let (rect, toggle) = self.views.active.panels[crate::workspace_panels::FILTERS].show(
            ui,
            egui::Id::new(("selection_filters", self.views.host)),
            "Selection filters",
            canvas,
            area,
            collapsed,
            pcbcraft_chrome::workspace::panels::card_width(canvas, false),
            |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    changed = pcbcraft_chrome::selection_filters::controls(ui, &mut rows);
                });
            },
        );
        if changed || toggle {
            let mut params = serde_json::Map::new();
            if changed {
                for row in rows.into_iter().filter(|row| row.enabled) {
                    params.insert(row.key.to_owned(), Value::Bool(row.checked));
                }
            }
            if toggle {
                params.insert("collapsed".into(), Value::Bool(!collapsed));
            }
            if let Err(error) = self.set_selection_filters(id, &Value::Object(params)) {
                self.message = Some(error);
            }
            self.repaint_editors(ui.ctx());
        }
        rect
    }
}
