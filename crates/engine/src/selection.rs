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

    /// Total number of selected symbols and wires.
    pub fn len(&self) -> usize {
        self.symbols.len() + self.wires.len()
    }

    /// Whether no item is selected.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.wires.is_empty()
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
        for (out, incoming) in [(&mut result.symbols, &hits.symbols), (&mut result.wires, &hits.wires)] {
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
        self.symbols.retain(|id| parse_id(id, "symbol-").and_then(|id| project.symbol_index(id)).is_some());
        self.wires.retain(|id| parse_id(id, "wire-").is_some_and(|id| project.wires.binary_search_by_key(&id, |wire| wire.ordinal).is_ok()));
    }

    /// Parse bounded canonical ID lists, rejecting extra fields and duplicate identities.
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 2 {
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
        Some(Self { symbols: read("symbols", "symbol-", MAX_SYMBOLS)?, wires: read("wires", "wire-", MAX_WIRES)? })
    }

    /// Reject an empty group or any missing identity before constructing an edit.
    fn validate(&self, project: &Project) -> Result<(), CommandError> {
        let mut found = self.clone();
        found.retain(project);
        if self.is_empty() || &found != self {
            Err(CommandError::InvalidParams("selection must contain existing symbols or wires".to_owned()))
        } else {
            Ok(())
        }
    }

    /// Build the same bounded move for a preview or command without changing the source snapshot.
    /// Selected wire routes and their junctions move once; unselected component pins stay fixed,
    /// with orthogonal end legs connecting them to the moved route. Any invalid item rejects all.
    pub fn moved(&self, project: &Project, delta: Point) -> Result<Project, CommandError> {
        self.validate(project)?;
        if delta == (Point { x_nm: 0, y_nm: 0 }) {
            return Ok(project.clone());
        }
        let mut next = project.clone();
        for symbol in Arc::make_mut(&mut next.symbols) {
            if self.symbols.contains(&symbol.id()) {
                symbol.position = translated(symbol.position, delta)?;
            }
        }
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
        for (id, point) in Arc::make_mut(&mut next.junctions) {
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
            Arc::make_mut(&mut next.wires)[index].bends = Some(path[1..path.len() - 1].to_vec());
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
        let next = items.moved(project, delta)?.healed(project, self.revision())?;
        if next.symbols != project.symbols || next.wires != project.wires || next.junctions != project.junctions {
            self.change(next);
        }
        Ok(())
    }

    /// Remove a validated group, incident wires, orphan junctions and unused components/footprints
    /// together. A failure leaves every item and history entry unchanged.
    pub(crate) fn delete_items(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid selection".to_owned()))?;
        let project = self.open_project()?;
        items.validate(project)?;
        let mut next = project.clone();
        let deleted: BTreeSet<_> = project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).map(|s| s.ordinal).collect();
        Arc::make_mut(&mut next.symbols).retain(|s| !deleted.contains(&s.ordinal));
        Arc::make_mut(&mut next.wires)
            .retain(|wire| !items.wires.contains(&wire.id()) && wire.terminals().iter().all(|pin| !deleted.contains(&pin.symbol)));
        Arc::make_mut(&mut next.units).retain(|_, symbol| !deleted.contains(symbol));
        let used: BTreeSet<_> = next.symbols.iter().map(|s| s.component).collect();
        let removed: BTreeSet<_> =
            project.symbols().iter().filter(|s| deleted.contains(&s.ordinal) && !used.contains(&s.component)).map(|s| s.component).collect();
        for id in removed {
            Arc::make_mut(&mut next.components).remove(&id);
            Arc::make_mut(&mut next.board).remove_component(id);
        }
        next.prune_junctions();
        next.connectivity = Arc::default();
        self.change(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
