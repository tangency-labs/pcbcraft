//! Stable item sets and atomic group edits shared by the editor and MCP.

use crate::{
    CommandError, Project, Session,
    schematic::{
        MAX_SYMBOLS, Point, WireEndpoint,
        wiring::{MAX_WIRES, parse_id},
    },
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

/// A disposable set of schematic identities; never stored in the native project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Selection {
    pub symbols: BTreeSet<String>,
    pub wires: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub net_labels: BTreeSet<String>,
}

impl Selection {
    /// Select exactly one symbol.
    pub fn one_symbol(id: String) -> Self {
        Self { symbols: [id].into(), ..Self::default() }
    }

    /// Select exactly one wire.
    pub fn one_wire(id: String) -> Self {
        Self { wires: [id].into(), ..Self::default() }
    }

    /// Select exactly one named-net anchor, including power and ground symbols.
    pub fn one_net_label(id: u64) -> Self {
        Self { net_labels: [format!("net-label-{id}")].into(), ..Self::default() }
    }

    /// The sole named-net anchor, only when no other object is selected.
    pub fn single_net_label(&self) -> Option<u64> {
        if self.len() == 1 { self.net_labels.first().and_then(|id| parse_id(id, "net-label-")) } else { None }
    }

    /// Whether every selected identity belongs to another selection.
    pub fn is_subset(&self, other: &Self) -> bool {
        self.symbols.is_subset(&other.symbols) && self.wires.is_subset(&other.wires) && self.net_labels.is_subset(&other.net_labels)
    }

    /// Total number of selected components, wires and named-net anchors.
    pub fn len(&self) -> usize {
        self.symbols.len() + self.wires.len() + self.net_labels.len()
    }

    /// Whether no item is selected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The sole symbol, only when no other object is selected.
    pub fn single_symbol(&self) -> Option<&str> {
        if self.len() == 1 { self.symbols.first().map(String::as_str) } else { None }
    }

    /// The sole wire, only when no other object is selected.
    pub fn single_wire(&self) -> Option<&str> {
        if self.len() == 1 { self.wires.first().map(String::as_str) } else { None }
    }

    /// Replace, add or toggle hits. Toggle takes precedence over addition.
    pub fn combine(&self, hits: &Self, add: bool, toggle: bool) -> Self {
        if !add && !toggle {
            return hits.clone();
        }
        let mut result = self.clone();
        for (out, incoming) in [(&mut result.symbols, &hits.symbols), (&mut result.wires, &hits.wires), (&mut result.net_labels, &hits.net_labels)] {
            for id in incoming {
                if toggle && out.contains(id) {
                    out.remove(id);
                } else {
                    out.insert(id.clone());
                }
            }
        }
        result
    }

    /// Remove identities absent from the current snapshot, including after undo or deletion.
    pub fn retain(&mut self, project: &Project) {
        self.net_labels.retain(|id| project.net_labels().iter().any(|label| label.object_id() == *id));
        self.symbols.retain(|id| parse_id(id, "symbol-").and_then(|id| project.symbol_index(id)).is_some());
        self.wires.retain(|id| parse_id(id, "wire-").is_some_and(|id| project.wires.binary_search_by_key(&id, |wire| wire.ordinal).is_ok()));
    }

    /// Parse bounded canonical ID lists, rejecting extra fields and duplicate identities.
    /// The optional `net_labels` list extends the original symbol/wire contract.
    /// Identities are checked for form only; [`Self::retain`] drops those absent from a project.
    pub fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.keys().any(|key| !matches!(key.as_str(), "symbols" | "wires" | "net_labels")) {
            return None;
        }
        let read = |key: &str, prefix: &str, limit| -> Option<BTreeSet<String>> {
            let array = object.get(key)?.as_array()?;
            if array.len() > limit {
                return None;
            }
            let mut ids = BTreeSet::new();
            for value in array {
                let id = value.as_str()?;
                parse_id(id, prefix)?;
                if !ids.insert(id.to_owned()) {
                    return None;
                }
            }
            Some(ids)
        };
        Some(Self {
            symbols: read("symbols", "symbol-", MAX_SYMBOLS)?,
            wires: read("wires", "wire-", MAX_WIRES)?,
            net_labels: if object.contains_key("net_labels") {
                read("net_labels", "net-label-", crate::net_labels::MAX_LABELS)?
            } else {
                BTreeSet::new()
            },
        })
    }

    /// Reject an empty group or any missing identity before constructing an edit.
    fn validate(&self, project: &Project) -> Result<(), CommandError> {
        let mut found = self.clone();
        found.retain(project);
        if self.is_empty() || &found != self {
            Err(CommandError::InvalidParams("selection must contain existing symbols, wires or named-net anchors".to_owned()))
        } else {
            Ok(())
        }
    }

    /// Build the same bounded move for a preview or command without changing the source snapshot.
    /// Named anchors retain their identities and attached endpoints. Selected wire routes and
    /// their junctions move once; unselected component pins stay fixed,
    /// with orthogonal end legs connecting them to the moved route. Any invalid item rejects all.
    pub fn moved(&self, project: &Project, delta: Point) -> Result<Project, CommandError> {
        self.validate(project)?;
        if delta == (Point { x_nm: 0, y_nm: 0 }) {
            return Ok(project.clone());
        }
        let mut next = project.clone();
        for symbol in Arc::make_mut(&mut next.sheet_mut().symbols) {
            if self.symbols.contains(&symbol.id()) {
                symbol.position = translated(symbol.position, delta)?;
            }
        }
        for label in Arc::make_mut(&mut next.sheet_mut().net_labels) {
            if self.net_labels.contains(&label.object_id()) {
                label.at = translated(label.at, delta)?;
            }
        }
        next.sheet_mut().connectivity = Arc::default();
        let junctions: BTreeSet<_> = project
            .wires()
            .iter()
            .filter(|wire| self.wires.contains(&wire.id()))
            .flat_map(|wire| wire.endpoints())
            .filter_map(|end| match end {
                WireEndpoint::Junction(id) => Some(id),
                _ => None,
            })
            .collect();
        for (id, point) in Arc::make_mut(&mut next.sheet_mut().junctions) {
            if junctions.contains(id) {
                *point = translated(*point, delta)?;
            }
        }
        for (index, wire) in project.wires().iter().enumerate() {
            if !self.wires.contains(&wire.id()) {
                continue;
            }
            let path = project.wire_path(wire).into_iter().map(|point| translated(point, delta)).collect::<Result<Vec<_>, _>>()?;
            let [Some(from), Some(to)] = wire.endpoints().map(|end| next.endpoint_position(end)) else {
                return Err(CommandError::InvalidParams("wire endpoint does not exist".to_owned()));
            };
            let path = crate::schematic::orthogonal_path(std::iter::once(from).chain(path).chain(std::iter::once(to)), true);
            if path.len() < 2 || path.len() > crate::wire::MAX_BENDS + 2 {
                return Err(CommandError::InvalidParams("moved wire route exceeds its bounds".to_owned()));
            }
            Arc::make_mut(&mut next.sheet_mut().wires)[index].bends = Some(path[1..path.len() - 1].to_vec());
        }
        Ok(next)
    }
}

/// Translate with checked arithmetic and the same coordinate bounds as individual edits.
fn translated(point: Point, delta: Point) -> Result<Point, CommandError> {
    let next = point.x_nm.checked_add(delta.x_nm).zip(point.y_nm.checked_add(delta.y_nm)).map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    next.filter(crate::wire::valid_point).ok_or_else(|| CommandError::InvalidParams("move exceeds schematic coordinate bounds".to_owned()))
}

impl Session {
    /// Validate the revision and selection, attach exact pin contacts and heal affected routes
    /// before committing one undoable group move.
    pub(crate) fn move_items(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid selection".to_owned()))?;
        let project = self.open_project()?;
        let delta = Point { x_nm: params["dx_nm"].as_i64().unwrap_or_default(), y_nm: params["dy_nm"].as_i64().unwrap_or_default() };
        let next = items.moved(project, delta)?.healed(project, self.revision(), crate::wire::auto_heal(params))?;
        if next.symbols != project.symbols
            || next.wires != project.wires
            || next.junctions != project.junctions
            || next.net_labels != project.net_labels
        {
            self.change(next);
        }
        Ok(())
    }

    /// Remove a validated group of components, wires and named anchors, incident wires,
    /// orphan junctions and unused live components together. Accepted footprints
    /// remain pending removal when a board baseline is enabled. A failure leaves
    /// every item and history entry unchanged.
    pub(crate) fn delete_items(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid selection".to_owned()))?;
        let project = self.open_project()?;
        items.validate(project)?;
        let mut next = project.clone();
        let deleted: BTreeSet<_> = project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).map(|s| s.ordinal).collect();
        Arc::make_mut(&mut next.sheet_mut().symbols).retain(|s| !deleted.contains(&s.ordinal));
        Arc::make_mut(&mut next.sheet_mut().net_labels).retain(|label| !items.net_labels.contains(&label.object_id()));
        Arc::make_mut(&mut next.sheet_mut().wires).retain(|wire| {
            !items.wires.contains(&wire.id())
                && wire.terminals().iter().all(|pin| !deleted.contains(&pin.symbol))
                && wire.endpoints().iter().all(|end| match end {
                    WireEndpoint::NetLabel(id) => !items.net_labels.contains(&format!("net-label-{id}")),
                    _ => true,
                })
        });
        Arc::make_mut(&mut next.units).retain(|_, symbol| !deleted.contains(symbol));
        let used: BTreeSet<_> = next.all_symbols().map(|s| s.component).collect();
        let removed: BTreeSet<_> =
            project.symbols().iter().filter(|s| deleted.contains(&s.ordinal) && !used.contains(&s.component)).map(|s| s.component).collect();
        for id in removed {
            next.retire_component(id);
        }
        next.prune_junctions();
        next.sheet_mut().connectivity = Arc::default();
        self.change(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
