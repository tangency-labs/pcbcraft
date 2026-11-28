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
/// Resize a sheet symbol, optionally moving its origin, as one revision-checked edit.
pub const RESIZE_SHEET: &str = "schematic.resize_sheet";
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
    /// The direction the child-side glyph extends from its wire anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<schematic::PinOrientation>,
    /// Additional named anchors imported for this port on the same sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imported_aliases: Vec<u64>,
    /// Stable side-local slot: removing another label never moves this pin.
    pub slot: u8,
    /// An explicitly positioned parent pin, independent of its electrical direction.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "parent_pin_layout")]
    pub parent_pin: Option<ParentPin>,
}

fn automatic_offset(slot: u8) -> i64 {
    (i64::from(slot) + 2) * 2 * schematic::GRID_NM
}

pub(crate) fn available_pin_slot<'a>(labels: impl Iterator<Item = &'a Label> + Clone, direction: Direction, explicit: bool) -> Option<u8> {
    let right = direction.right();
    let edge = if right { PinEdge::Right } else { PinEdge::Left };
    (0..MAX_LABELS as u8).find(|&slot| {
        !labels.clone().any(|label| {
            (label.direction.right() == right && label.slot == slot)
                || (!explicit && label.parent_pin.is_some_and(|pin| pin.edge == edge && pin.offset_nm == automatic_offset(slot)))
        })
    })
}

impl Label {
    pub fn orientation(&self) -> schematic::PinOrientation {
        use schematic::PinOrientation::{Left, Right};
        self.orientation.unwrap_or(if self.direction.right() { Right } else { Left })
    }
}

fn parent_pin_layout<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<ParentPin>, D::Error> {
    ParentPin::deserialize(input).map(Some)
}

/// Which edge of a sheet symbol holds an interface pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Distance along the edge from its top or left corner, in nanometers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentPin {
    pub edge: PinEdge,
    pub offset_nm: i64,
}

impl ParentPin {
    fn on_border(self, origin: Point, size: Point) -> Point {
        let (x, y) = match self.edge {
            PinEdge::Left => (0, self.offset_nm),
            PinEdge::Right => (size.x_nm, self.offset_nm),
            PinEdge::Top => (self.offset_nm, 0),
            PinEdge::Bottom => (self.offset_nm, size.y_nm),
        };
        Point { x_nm: origin.x_nm.saturating_add(x), y_nm: origin.y_nm.saturating_add(y) }
    }
}

/// A sheet symbol refers to one child identity, independently of its name or filename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetSymbol {
    pub id: u64,
    pub child: Uuid,
    pub at: Point,
    /// Authored extent. Older records retain their default size and automatic pin clearance.
    #[serde(default, deserialize_with = "sheet_size", skip_serializing_if = "Option::is_none")]
    pub size: Option<Point>,
}

/// Missing size preserves older records; explicit null is rejected instead of dropping data.
fn sheet_size<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<Point>, D::Error> {
    Point::deserialize(input).map(Some)
}

/// Geometry and display metadata of a pin derived from its child label.
pub struct SheetPin<'a> {
    pub label: &'a Label,
    pub at: Point,
    pub edge: PinEdge,
}

impl Project {
    /// Preview a complete sheet rectangle without publishing state. Pins keep their IDs and
    /// stable vertical slots; right-side pins follow the new width. Refuse extents smaller
    /// than the interface clearance or outside schematic bounds.
    pub fn resized_sheet_symbol(&self, object: &str, at: Point, size: Point) -> Result<Self, CommandError> {
        let id = schematic::wiring::parse_id(object, "sheet-symbol-").ok_or_else(|| CommandError::InvalidParams("invalid sheet symbol ID".into()))?;
        let index =
            self.subsheets.iter().position(|s| s.id == id).ok_or_else(|| CommandError::NotFound("unknown sheet symbol on this sheet".into()))?;
        let minimum = self.sheet_symbol_min_size(&self.subsheets[index]);
        if size.x_nm < minimum.x_nm || size.y_nm < minimum.y_nm {
            return Err(CommandError::InvalidParams("sheet size must leave room for its pins".into()));
        }
        let mut next = self.clone();
        let block = &mut Arc::make_mut(&mut next.sheet_mut().subsheets)[index];
        block.at = at;
        block.size = Some(size);
        next.validate_hierarchy().map_err(CommandError::InvalidParams)?;
        Ok(next)
    }

    /// Preview an addressed sheet symbol or label move without publishing history. Fixed
    /// endpoint identities remain intact, so attached paths use the destination geometry.
    /// Foreign IDs and any hierarchy geometry outside the document bounds are rejected.
    pub fn moved_hierarchy(&self, object: &str, at: Point) -> Result<Self, CommandError> {
        let mut next = self.clone();
        if let Some(id) = schematic::wiring::parse_id(object, "sheet-symbol-") {
            let index =
                next.subsheets.iter().position(|s| s.id == id).ok_or_else(|| CommandError::NotFound("unknown sheet symbol on this sheet".into()))?;
            Arc::make_mut(&mut next.sheet_mut().subsheets)[index].at = at;
        } else if let Some(id) = schematic::wiring::parse_id(object, "label-") {
            let index = next
                .labels
                .iter()
                .position(|l| l.id == id)
                .ok_or_else(|| CommandError::NotFound("unknown hierarchical label on this sheet".into()))?;
            Arc::make_mut(&mut next.sheet_mut().labels)[index].at = at;
        } else {
            return Err(CommandError::NotFound("unknown hierarchy object ID".into()));
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
        let size = self.sheet_symbol_size(block);
        self.sheets.iter().find(|s| s.sheet_id == block.child).into_iter().flat_map(move |s| {
            s.labels.iter().map(move |label| {
                let pin = label.parent_pin.unwrap_or(ParentPin {
                    edge: if label.direction.right() { PinEdge::Right } else { PinEdge::Left },
                    offset_nm: automatic_offset(label.slot),
                });
                SheetPin { at: pin.on_border(block.at, size), edge: pin.edge, label }
            })
        })
    }

    /// Minimum editable extent with clearance below all occupied pin slots.
    pub fn sheet_symbol_min_size(&self, block: &SheetSymbol) -> Point {
        let mut size = Point { x_nm: 8 * schematic::GRID_NM, y_nm: 6 * schematic::GRID_NM };
        if let Some(child) = self.sheets.iter().find(|s| s.sheet_id == block.child) {
            for label in child.labels.iter() {
                match label.parent_pin {
                    Some(ParentPin { edge: PinEdge::Top | PinEdge::Bottom, offset_nm }) => size.x_nm = size.x_nm.max(offset_nm),
                    Some(ParentPin { offset_nm, .. }) => size.y_nm = size.y_nm.max(offset_nm),
                    None => size.y_nm = size.y_nm.max((i64::from(label.slot) * 2 + 6) * schematic::GRID_NM),
                }
            }
        }
        size
    }

    /// Authored size or legacy defaults, grown as needed when child pins are added.
    pub fn sheet_symbol_size(&self, block: &SheetSymbol) -> Point {
        let minimum = self.sheet_symbol_min_size(block);
        let authored = block.size.unwrap_or(Point { x_nm: 16 * schematic::GRID_NM, y_nm: 10 * schematic::GRID_NM });
        Point { x_nm: authored.x_nm.max(minimum.x_nm), y_nm: authored.y_nm.max(minimum.y_nm) }
    }

    /// Fixed hierarchy and named-net anchors participate in wire picking/healing beside pins.
    pub fn hierarchy_endpoints(&self) -> impl Iterator<Item = (WireEndpoint, Point)> + '_ {
        self.net_labels.iter().map(|label| (WireEndpoint::NetLabel(label.id), label.at)).chain(
            self.labels
                .iter()
                .map(|label| (WireEndpoint::Label(label.id), label.at))
                .chain(self.subsheets.iter().flat_map(|block| self.sheet_pins(block).map(|pin| (WireEndpoint::SheetPin(pin.label.id), pin.at)))),
        )
    }

    /// Resolve all sheets through explicit parent/child pins and scoped named anchors.
    /// Hierarchical names alone do not join unrelated sheets; global/power labels do.
    pub fn electrical_connectivity(&self) -> Connectivity {
        self.named_connectivity(true)
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
                    || label.parent_pin.is_some_and(|pin| !(0..=2 * MAX_CENTER_NM).contains(&pin.offset_nm))
                    || label.imported_aliases.len() > crate::net_labels::MAX_LABELS
                    || label.imported_aliases.windows(2).any(|pair| pair[0] >= pair[1])
                    || label.imported_aliases.iter().any(|id| !(1..=crate::MAX_OBJECT_ID).contains(id))
                    || usize::from(label.slot) >= MAX_LABELS
                    || !names.insert(&label.name)
                    || !slots.insert((label.direction.right(), label.slot))
                {
                    return Err("invalid or duplicate hierarchical label name, position or pin slot".into());
                }
            }
            for block in sheet.subsheets.iter() {
                persistence::validate_id(&mut ids, block.id)?;
                let mut occupied = BTreeSet::new();
                if self.sheet_pins(block).any(|pin| !occupied.insert((pin.at.x_nm, pin.at.y_nm))) {
                    return Err("parent sheet pins must occupy distinct positions".into());
                }
                if block.size.is_some_and(|size| {
                    size.x_nm < 8 * schematic::GRID_NM
                        || size.y_nm < 6 * schematic::GRID_NM
                        || size.x_nm > 2 * MAX_CENTER_NM
                        || size.y_nm > 2 * MAX_CENTER_NM
                }) {
                    return Err("invalid authored sheet dimensions".into());
                }
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
pub(crate) fn check_label_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("hierarchical label name must contain 1–200 characters without surrounding whitespace or control characters".into());
    }
    Ok(())
}

impl Session {
    /// Apply one hierarchy edit to an isolated sheet transaction. Cross-sheet pin cleanup is
    /// part of the same undo step, and validation failure never publishes partial state.
    /// Creating a child allocates its occurrence; reparenting retains the existing occurrence ID.
    pub(super) fn edit_hierarchy(&mut self, id: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let object = params.get("id").and_then(Value::as_str).and_then(|s| {
            schematic::wiring::parse_id(s, if matches!(id, MOVE_SHEET | RESIZE_SHEET | REMOVE_SHEET) { "sheet-symbol-" } else { "label-" })
        });
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
                        return Err(CommandError::CapExceeded("project already has 64 sheets".into()));
                    }
                    let name = name_param(params)?;
                    if project.sheets().any(|s| s.name.to_lowercase() == name.to_lowercase()) {
                        return Err(CommandError::InvalidParams("a sheet with that name already exists".into()));
                    }
                    let child = fresh_id()?;
                    let sheet = project.new_sheet_record(child, name)?;
                    Arc::make_mut(&mut project.sheets).push(sheet);
                    SheetTarget { project: project.id, sheet: child }
                } else {
                    project.resolve_sheet(params["child"].as_str())?
                };
                let block = SheetSymbol { id: self.next_object_id()?, child: child.sheet, at: at()?, size: None };
                Arc::make_mut(&mut project.sheet_mut().subsheets).push(block);
            }
            RESIZE_SHEET => {
                let address = params["id"].as_str().unwrap_or_default();
                let size = Point {
                    x_nm: params["width_nm"].as_i64().ok_or_else(|| CommandError::InvalidParams("missing width_nm".into()))?,
                    y_nm: params["height_nm"].as_i64().ok_or_else(|| CommandError::InvalidParams("missing height_nm".into()))?,
                };
                let point = at()?;
                let block = project
                    .subsheets
                    .iter()
                    .find(|b| Some(b.id) == object)
                    .ok_or_else(|| CommandError::NotFound("unknown sheet symbol on this sheet".into()))?;
                if block.at == point && project.sheet_symbol_size(block) == size {
                    return Ok(());
                }
                project = project.resized_sheet_symbol(address, point, size)?;
            }
            MOVE_SHEET | REMOVE_SHEET => {
                let index = project
                    .subsheets
                    .iter()
                    .position(|s| Some(s.id) == object)
                    .ok_or_else(|| CommandError::NotFound("unknown sheet symbol on this sheet".into()))?;
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
                            .ok_or_else(|| CommandError::NotFound("unknown hierarchical label on this sheet".into()))?,
                    )
                } else {
                    None
                };
                if existing.is_none() && project.labels.len() >= MAX_LABELS {
                    return Err(CommandError::CapExceeded("sheet already has 64 hierarchical labels".into()));
                }
                let explicit_pin = existing.and_then(|index| project.labels[index].parent_pin);
                let slot = existing
                    .filter(|&i| project.labels[i].direction.right() == direction.right())
                    .map(|i| project.labels[i].slot)
                    .or_else(|| {
                        available_pin_slot(
                            project.labels.iter().enumerate().filter(|(index, _)| Some(*index) != existing).map(|(_, label)| label),
                            direction,
                            explicit_pin.is_some(),
                        )
                    })
                    .ok_or_else(|| CommandError::CapExceeded("no free sheet pin slot".into()))?;
                let label = Label {
                    id: match existing {
                        Some(index) => project.labels[index].id,
                        None => self.next_object_id()?,
                    },
                    name,
                    direction,
                    at: at()?,
                    slot,
                    parent_pin: explicit_pin,
                    orientation: existing.and_then(|index| project.labels[index].orientation),
                    imported_aliases: existing.map_or_else(Vec::new, |index| project.labels[index].imported_aliases.clone()),
                };
                if let Some(index) = existing {
                    let old = &project.labels[index];
                    if *old == label {
                        return Ok(());
                    }
                    if old.name != label.name && !label.imported_aliases.is_empty() {
                        let old_name = old.name.clone();
                        for alias in Arc::make_mut(&mut project.sheet_mut().net_labels) {
                            if alias.name == old_name
                                && alias.style == crate::net_labels::Style::Local
                                && label.imported_aliases.binary_search(&alias.id).is_ok()
                            {
                                alias.name.clone_from(&label.name);
                            }
                        }
                    }
                    Arc::make_mut(&mut project.sheet_mut().labels)[index] = label;
                } else {
                    Arc::make_mut(&mut project.sheet_mut().labels).push(label);
                }
            }
            DELETE_LABEL => {
                let label = project
                    .labels
                    .iter()
                    .find(|l| Some(l.id) == object)
                    .ok_or_else(|| CommandError::NotFound("unknown hierarchical label on this sheet".into()))?
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
        self.change(project)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
