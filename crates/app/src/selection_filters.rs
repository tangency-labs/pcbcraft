//! Per-view selection policy, shared by canvas picking, Browser actions and live MCP.
use crate::{
    PcbApp,
    editor_view::{BoardState, ViewId, Workspace},
};
use pcbcraft_engine::{board::view::BoardView, selection::Selection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Supported selectable categories. Missing fields in older local arrangements default to on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Filters {
    pub symbols: bool,
    pub pins: bool,
    pub pads: bool,
    pub wires: bool,
    pub sheets: bool,
    pub labels: bool,
    pub no_connects: bool,
    pub footprints: bool,
    pub tracks: bool,
    pub vias: bool,
    pub graphics: bool,
    pub zones: bool,
    pub include_locked: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            symbols: true,
            pins: false,
            pads: false,
            wires: true,
            sheets: true,
            labels: true,
            no_connects: true,
            footprints: true,
            tracks: true,
            vias: true,
            zones: true,
            graphics: true,
            include_locked: true,
        }
    }
}

impl Filters {
    /// Local/global annotations follow Labels; native power instances follow Symbols normally.
    pub(super) fn allows_net_label(self, _style: pcbcraft_engine::net_labels::Style) -> bool {
        self.labels
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
        if !self.no_connects {
            selection.no_connects.clear();
        }
        if !self.symbols {
            selection.symbols.clear();
        }
        if !self.wires {
            selection.wires.clear();
        }
    }

    /// Reject locked board objects only when the view excludes them from selection.
    pub(super) fn permits_lock(self, locked: bool) -> bool {
        self.include_locked || !locked
    }

    /// Drop locked placements from a footprint selection without touching inspection.
    pub(super) fn retain_unlocked_footprints(self, board: BoardView<'_>, selected: &mut BTreeSet<String>) {
        if !self.include_locked {
            selected.retain(|id| board.footprint_locked(id) != Some(true));
        }
    }

    /// Prune edit targets after a filter change, a board update or a mixed selection.
    pub(super) fn prune_locked_board_selection(self, board: BoardView<'_>, state: &mut BoardState) {
        if self.include_locked {
            return;
        }
        self.retain_unlocked_footprints(board, &mut state.selection);
        if state.reference.selected.as_ref().and_then(|id| board.footprint_locked(id)) == Some(true) {
            state.reference = Default::default();
        }
        self.retain_unlocked_copper(board, &mut state.tracks.selected);
    }

    /// Drop locked tracks and vias from a mixed copper selection.
    pub(super) fn retain_unlocked_copper(self, board: BoardView<'_>, selected: &mut BTreeSet<uuid::Uuid>) {
        if !self.include_locked {
            retain_unlocked_ids(selected, board.tracks(), |track| (track.id, track.locked));
            retain_unlocked_ids(selected, board.vias(), |via| (via.id, via.locked));
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
                row("pins", "Pins", self.pins),
                row("wires", "Wires", self.wires),
                row("labels", "Labels", self.labels),
                row("no_connects", "No connects", self.no_connects),
                planned("graphics", "Graphics"),
                planned("images", "Images"),
                planned("text", "Text"),
                row("sheets", "Sheets", self.sheets),
                planned("rule_areas", "Rule areas"),
            ],
            Workspace::Board => vec![
                row("footprints", "Footprints", self.footprints),
                row("pads", "Pads", self.pads),
                row("tracks", "Tracks", self.tracks),
                row("vias", "Vias", self.vias),
                row("zones", "Zones", self.zones),
                planned("text", "Text"),
                row("graphics", "Graphics", self.graphics),
                planned("dimensions", "Dimensions"),
                row("include_locked", "Locked objects", self.include_locked),
            ],
        }
    }
}

/// Check selected IDs against one ID-sorted copper slice without walking the board.
/// IDs absent from this slice may belong to the other copper kind, so leave them alone.
fn retain_unlocked_ids<T>(selected: &mut BTreeSet<uuid::Uuid>, ordered: &[T], mut fields: impl FnMut(&T) -> (uuid::Uuid, bool)) {
    selected.retain(|id| !ordered.binary_search_by_key(id, |item| fields(item).0).is_ok_and(|index| fields(&ordered[index]).1));
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
                (Workspace::Schematic, "pins") => &mut next.pins,
                (Workspace::Board, "pads") => &mut next.pads,
                (Workspace::Schematic, "wires") => &mut next.wires,
                (Workspace::Schematic, "sheets") => &mut next.sheets,
                (Workspace::Schematic, "labels") => &mut next.labels,
                (Workspace::Schematic, "no_connects") => &mut next.no_connects,
                (Workspace::Board, "footprints") => &mut next.footprints,
                (Workspace::Board, "tracks") => &mut next.tracks,
                (Workspace::Board, "vias") => &mut next.vias,
                (Workspace::Board, "zones") => &mut next.zones,
                (Workspace::Board, "graphics") => &mut next.graphics,
                (Workspace::Board, "include_locked") => &mut next.include_locked,
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
            if !next.pins || !next.pads {
                view.inspection.retain(|target| match target {
                    pcbcraft_engine::board::correspondence::Source::Terminal(_) => next.pins,
                    pcbcraft_engine::board::correspondence::Source::Pad(_) => next.pads,
                });
            }
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
                view.board.reference = Default::default();
            }
            let board = self.session.project().and_then(|project| view.board.target.and_then(|target| project.board_view(target)));
            if !next.tracks || !next.vias {
                // Drop only the copper kinds that can no longer be picked.
                let vias: BTreeSet<uuid::Uuid> = board.map(|board| board.vias().iter().map(|via| via.id).collect()).unwrap_or_default();
                view.board.tracks.selected.retain(|id| if vias.contains(id) { next.vias } else { next.tracks });
            }
            if let Some(board) = board {
                next.prune_locked_board_selection(board, &mut view.board);
            }
            if !next.zones {
                view.board.zones.select(None);
            }
            if !next.graphics {
                view.board.drawing.select(None);
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
            egui::Id::unique(("selection_filters", self.views.host)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// A small edit selection uses logarithmic probes even when the board holds much more copper.
    #[test]
    fn pruning_selected_copper_only_probes_ids() {
        let records: Vec<_> = (1..=4096).map(|id| (Uuid::from_u128(id), id == 4096)).collect();
        let unlocked = Uuid::from_u128(1);
        let locked = Uuid::from_u128(4096);
        let absent = Uuid::from_u128(4097);
        let mut selected = BTreeSet::from([unlocked, locked, absent]);
        let mut probes = 0;
        retain_unlocked_ids(&mut selected, &records, |item| {
            probes += 1;
            *item
        });
        assert_eq!(selected, BTreeSet::from([unlocked, absent]));
        assert!(probes <= 3 * 15, "checked {probes} copper records");
    }
}
