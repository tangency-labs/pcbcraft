//! Single-instance sheet hierarchy and its explicit electrical interfaces.

use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Place an existing, currently unparented sheet inside the addressed parent.
pub const PLACE_SHEET: &str = "schematic.place_sheet";
/// Create a named child and place its parent symbol in one undo step.
pub const CREATE_CHILD: &str = "schematic.create_sheet";
/// Move a sheet symbol while retaining its interface identities and connections.
pub const MOVE_SHEET: &str = "schematic.move_sheet";
/// Remove a sheet symbol and parent wires to its pins, retaining the child as a root sheet.
pub const REMOVE_SHEET: &str = "schematic.remove_sheet";
/// Add a named hierarchical label and automatically expose its pin on the parent symbol.
pub const ADD_LABEL: &str = "schematic.add_hierarchical_label";
/// Rename, change direction or move a hierarchical label, retaining its electrical identity.
pub const EDIT_LABEL: &str = "schematic.edit_hierarchical_label";
/// Remove a label and its parent pin, including wires directly attached to either endpoint.
pub const DELETE_LABEL: &str = "schematic.delete_hierarchical_label";
/// Limit interface size independently of wire and symbol limits.
pub const MAX_LABELS: usize = 64;

/// Electrical direction, also used to arrange parent pins. It does not change connectivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
    Bidirectional,
    TriState,
    Passive,
}

impl Direction {
    /// Directions accepted by the GUI and command catalog, in display order.
    pub const ALL: [Self; 5] = [Self::Input, Self::Output, Self::Bidirectional, Self::TriState, Self::Passive];

    /// Stable command spelling, independent of translated presentation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Bidirectional => "bidirectional",
            Self::TriState => "tri_state",
            Self::Passive => "passive",
        }
    }

    /// Output-like pins occupy the right edge; other interface pins occupy the left edge.
    pub fn right(self) -> bool {
        matches!(self, Self::Output | Self::TriState)
    }
}

/// One child-side interface anchor. Its ID also addresses the distinct parent-side sheet pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub id: u64,
    pub name: String,
    pub direction: Direction,
    pub at: Point,
    /// Stable side-local slot: removing another label never moves this pin.
    pub slot: u8,
}

/// A sheet symbol refers to one child identity, independently of its name or filename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetSymbol {
    pub id: u64,
    pub child: Uuid,
    pub at: Point,
}

/// Geometry and display metadata of a pin derived from its child label.
pub struct SheetPin<'a> {
    pub label: &'a Label,
    pub at: Point,
}

impl Project {
    /// Preview an addressed sheet symbol or label move without publishing history. Fixed
    /// endpoint identities remain intact, so attached paths use the destination geometry.
    /// Foreign IDs and any hierarchy geometry outside the document bounds are rejected.
    pub fn moved_hierarchy(&self, object: &str, at: Point) -> Result<Self, CommandError> {
        let mut next = self.clone();
        if let Some(id) = schematic::wiring::parse_id(object, "sheet-symbol-") {
            let index = next
                .subsheets
                .iter()
                .position(|s| s.id == id)
                .ok_or_else(|| CommandError::InvalidParams("unknown sheet symbol on this sheet".into()))?;
            Arc::make_mut(&mut next.sheet_mut().subsheets)[index].at = at;
        } else if let Some(id) = schematic::wiring::parse_id(object, "label-") {
            let index = next
                .labels
                .iter()
                .position(|l| l.id == id)
                .ok_or_else(|| CommandError::InvalidParams("unknown hierarchical label on this sheet".into()))?;
            Arc::make_mut(&mut next.sheet_mut().labels)[index].at = at;
        } else {
            return Err(CommandError::InvalidParams("unknown hierarchy object ID".into()));
        }
        next.validate_hierarchy().map_err(CommandError::InvalidParams)?;
        Ok(next)
    }

    /// Placed child sheet symbols on this projection.
    pub fn sheet_symbols(&self) -> &[SheetSymbol] {
        &self.subsheets
    }

    /// Hierarchical labels owned by this projection, including labels on an unparented root.
    pub fn hierarchical_labels(&self) -> &[Label] {
        &self.labels
    }

    /// Resolve a child's parent without changing the active document or inferring from order.
    pub fn parent_sheet(&self, child: SheetTarget) -> Option<SheetTarget> {
        (child.project == self.id).then_some(())?;
        self.sheets
            .iter()
            .find(|s| s.subsheets.iter().any(|block| block.child == child.sheet))
            .map(|s| SheetTarget { project: self.id, sheet: s.sheet_id })
    }

    /// Resolve a sheet symbol's child address only when the symbol belongs to this projection.
    pub fn child_sheet(&self, id: u64) -> Option<SheetTarget> {
        let child = self.subsheets.iter().find(|s| s.id == id)?.child;
        self.sheets().find(|s| s.target.sheet == child).map(|s| s.target)
    }

    /// Current child name; renames do not require editing the parent symbol.
    pub fn child_name(&self, block: &SheetSymbol) -> &str {
        self.sheets.iter().find(|s| s.sheet_id == block.child).map_or("", |s| s.sheet_name.as_str())
    }

    /// Pins derive names/directions from child records while stable slots preserve other pins.
    pub fn sheet_pins<'a>(&'a self, block: &'a SheetSymbol) -> impl Iterator<Item = SheetPin<'a>> + 'a {
        self.sheets.iter().find(|s| s.sheet_id == block.child).into_iter().flat_map(move |s| {
            s.labels.iter().map(move |label| SheetPin {
                at: Point {
                    x_nm: block.at.x_nm.saturating_add(if label.direction.right() { 16 * schematic::GRID_NM } else { 0 }),
                    y_nm: block.at.y_nm.saturating_add((i64::from(label.slot) + 2) * 2 * schematic::GRID_NM),
                },
                label,
            })
        })
    }

    /// Sheet block size in model coordinates; pin slots grow the height without moving anchors.
    pub fn sheet_symbol_size(&self, block: &SheetSymbol) -> Point {
        let slots = self.sheet_pins(block).map(|pin| i64::from(pin.label.slot) + 1).max().unwrap_or(0);
        Point { x_nm: 16 * schematic::GRID_NM, y_nm: (slots * 2 + 4).max(10) * schematic::GRID_NM }
    }

    /// Fixed hierarchy anchors participate in wire picking/healing alongside component pins.
    pub fn hierarchy_endpoints(&self) -> impl Iterator<Item = (WireEndpoint, Point)> + '_ {
        self.labels
            .iter()
            .map(|label| (WireEndpoint::Label(label.id), label.at))
            .chain(self.subsheets.iter().flat_map(|block| self.sheet_pins(block).map(|pin| (WireEndpoint::SheetPin(pin.label.id), pin.at))))
    }

    /// Resolve the electrical graph across all sheets, joining only explicit parent/child pins.
    /// Repeated text on different sheets has no effect on this graph.
    pub fn electrical_connectivity(&self) -> Connectivity {
        let symbols: Vec<_> = self.all_symbols().cloned().collect();
        let wires: Vec<_> = self.sheets.iter().flat_map(|s| s.wires.iter().cloned()).collect();
        let links = self.sheets.iter().flat_map(|s| s.subsheets.iter()).flat_map(|block| {
            self.sheets
                .iter()
                .find(|s| s.sheet_id == block.child)
                .into_iter()
                .flat_map(|s| s.labels.iter().map(|label| [WireEndpoint::SheetPin(label.id), WireEndpoint::Label(label.id)]))
        });
        Connectivity::build_linked(&symbols, &wires, links)
    }

    /// Check interface identities, bounds and a single-parent acyclic tree before adoption.
    /// Called for codec input and completed commands; it never changes authored state.
    pub(crate) fn validate_hierarchy(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        let mut parents = BTreeMap::new();
        for sheet in self.sheets.iter() {
            if sheet.labels.len() > MAX_LABELS || sheet.subsheets.len() >= sheets::MAX_SHEETS {
                return Err("sheet exceeds its hierarchy interface limit".into());
            }
            let mut names = BTreeSet::new();
            let mut slots = BTreeSet::new();
            for label in sheet.labels.iter() {
                persistence::validate_id(&mut ids, label.id)?;
                check_label_name(&label.name)?;
                if !wire::valid_point(&label.at)
                    || usize::from(label.slot) >= MAX_LABELS
                    || !names.insert(&label.name)
                    || !slots.insert((label.direction.right(), label.slot))
                {
                    return Err("invalid or duplicate hierarchical label name, position or pin slot".into());
                }
            }
            for block in sheet.subsheets.iter() {
                persistence::validate_id(&mut ids, block.id)?;
                if !self.sheets.iter().any(|s| s.sheet_id == block.child) || parents.insert(block.child, sheet.sheet_id).is_some() {
                    return Err("child sheet is missing or has more than one parent".into());
                }
                let projection = self.for_sheet(SheetTarget { project: self.id, sheet: sheet.sheet_id }).ok_or("missing parent")?;
                let size = projection.sheet_symbol_size(block);
                if !wire::valid_point(&block.at)
                    || !wire::valid_point(&Point { x_nm: block.at.x_nm.saturating_add(size.x_nm), y_nm: block.at.y_nm.saturating_add(size.y_nm) })
                {
                    return Err("sheet symbol extends outside the schematic bounds".into());
                }
            }
        }
        for sheet in self.sheets.iter() {
            let mut seen = BTreeSet::new();
            let mut current = sheet.sheet_id;
            while let Some(&parent) = parents.get(&current) {
                if !seen.insert(current) {
                    return Err("sheet hierarchy contains a cycle".into());
                }
                current = parent;
            }
        }
        Ok(())
    }
}

/// Hierarchy names are unique, case-sensitive interface names within a sheet.
fn check_label_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("hierarchical label name must contain 1–200 characters without surrounding whitespace or control characters".into());
    }
    Ok(())
}

impl Session {
    /// Apply one hierarchy edit to an isolated sheet transaction. Cross-sheet pin cleanup is
    /// part of the same undo step, and validation failure never publishes partial state.
    pub(super) fn edit_hierarchy(&mut self, id: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let object = params
            .get("id")
            .and_then(Value::as_str)
            .and_then(|s| schematic::wiring::parse_id(s, if matches!(id, MOVE_SHEET | REMOVE_SHEET) { "sheet-symbol-" } else { "label-" }));
        let at = || -> Result<Point, CommandError> {
            Ok(Point {
                x_nm: params["x_nm"].as_i64().ok_or_else(|| CommandError::InvalidParams("missing x_nm".into()))?,
                y_nm: params["y_nm"].as_i64().ok_or_else(|| CommandError::InvalidParams("missing y_nm".into()))?,
            })
        };
        match id {
            PLACE_SHEET | CREATE_CHILD => {
                let child = if id == CREATE_CHILD {
                    if project.sheets.len() >= sheets::MAX_SHEETS {
                        return Err(CommandError::Unavailable("project already has 64 sheets".into()));
                    }
                    let name = name_param(params)?;
                    if project.sheets().any(|s| s.name.to_lowercase() == name.to_lowercase()) {
                        return Err(CommandError::InvalidParams("a sheet with that name already exists".into()));
                    }
                    let child = fresh_id()?;
                    Arc::make_mut(&mut project.sheets).push(sheets::SchematicSheet::empty(child, name));
                    SheetTarget { project: project.id, sheet: child }
                } else {
                    project.resolve_sheet(params["child"].as_str())?
                };
                let block = SheetSymbol {
                    id: project
                        .highest_id()
                        .max(self.revision)
                        .checked_add(1)
                        .ok_or_else(|| CommandError::Unavailable("object IDs exhausted".into()))?,
                    child: child.sheet,
                    at: at()?,
                };
                Arc::make_mut(&mut project.sheet_mut().subsheets).push(block);
            }
            MOVE_SHEET | REMOVE_SHEET => {
                let index = project
                    .subsheets
                    .iter()
                    .position(|s| Some(s.id) == object)
                    .ok_or_else(|| CommandError::InvalidParams("unknown sheet symbol on this sheet".into()))?;
                if id == MOVE_SHEET {
                    let point = at()?;
                    if project.subsheets[index].at == point {
                        return Ok(());
                    }
                    project = project.moved_hierarchy(params["id"].as_str().unwrap_or_default(), point)?;
                } else {
                    let pins: BTreeSet<_> = project.sheet_pins(&project.subsheets[index]).map(|p| WireEndpoint::SheetPin(p.label.id)).collect();
                    Arc::make_mut(&mut project.sheet_mut().wires).retain(|w| !w.endpoints.iter().any(|end| pins.contains(end)));
                    Arc::make_mut(&mut project.sheet_mut().subsheets).remove(index);
                    project.prune_junctions();
                }
            }
            ADD_LABEL | EDIT_LABEL => {
                let name = name_param(params)?;
                check_label_name(&name).map_err(CommandError::InvalidParams)?;
                let direction: Direction = serde_json::from_value(params["direction"].clone())
                    .map_err(|_| CommandError::InvalidParams("unknown hierarchical label direction".into()))?;
                let existing = if id == EDIT_LABEL {
                    Some(
                        project
                            .labels
                            .iter()
                            .position(|l| Some(l.id) == object)
                            .ok_or_else(|| CommandError::InvalidParams("unknown hierarchical label on this sheet".into()))?,
                    )
                } else {
                    None
                };
                if existing.is_none() && project.labels.len() >= MAX_LABELS {
                    return Err(CommandError::Unavailable("sheet already has 64 hierarchical labels".into()));
                }
                let slot = existing
                    .filter(|&i| project.labels[i].direction.right() == direction.right())
                    .map(|i| project.labels[i].slot)
                    .or_else(|| {
                        (0..MAX_LABELS as u8)
                            .find(|slot| !project.labels.iter().any(|label| label.slot == *slot && label.direction.right() == direction.right()))
                    })
                    .ok_or_else(|| CommandError::Unavailable("no free sheet pin slot".into()))?;
                let label = Label {
                    id: existing.map_or_else(|| project.highest_id().max(self.revision).saturating_add(1), |i| project.labels[i].id),
                    name,
                    direction,
                    at: at()?,
                    slot,
                };
                let labels = Arc::make_mut(&mut project.sheet_mut().labels);
                if let Some(index) = existing {
                    if labels[index] == label {
                        return Ok(());
                    }
                    labels[index] = label;
                } else {
                    labels.push(label);
                }
            }
            DELETE_LABEL => {
                let label = project
                    .labels
                    .iter()
                    .find(|l| Some(l.id) == object)
                    .ok_or_else(|| CommandError::InvalidParams("unknown hierarchical label on this sheet".into()))?
                    .id;
                Arc::make_mut(&mut project.sheet_mut().labels).retain(|l| l.id != label);
                let active = project.sheet_index;
                for index in 0..project.sheets.len() {
                    project.sheet_index = index;
                    Arc::make_mut(&mut project.sheet_mut().wires)
                        .retain(|w| !w.endpoints.contains(&WireEndpoint::Label(label)) && !w.endpoints.contains(&WireEndpoint::SheetPin(label)));
                    project.prune_junctions();
                }
                project.sheet_index = active;
            }
            _ => return Err(CommandError::Unknown(id.into())),
        }
        project.validate_hierarchy().map_err(CommandError::InvalidParams)?;
        for sheet in Arc::make_mut(&mut project.sheets) {
            sheet.connectivity = Arc::default();
        }
        self.change(project);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
