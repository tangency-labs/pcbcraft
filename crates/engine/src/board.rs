//! Footprint placements, pinned physical definitions and exact schematic-to-board links.
use crate::schematic::{MAX_CENTER_NM, Point, SymbolDefinition, Terminal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub mod assignment;
pub mod definition;
pub mod drawing;
pub mod materials;
pub mod outline;
pub mod setup;
pub mod silkscreen;
mod transform;
pub mod view;

pub(crate) const FOOTPRINT_DEFINITION: &str = "generic-through-hole-2";
pub(crate) const FOOTPRINT_REVISION: u32 = 1;
const HALF_PAD_PITCH_NM: i64 = 1_270_000;

/// A stable numeric pad within a placed footprint. Resolution validates that the pad exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    pub(crate) board: Option<uuid::Uuid>,
    footprint: u64,
    number: u8,
}
impl Pad {
    /// Parse canonical footprint and pad identities without guessing an owner or pin mapping.
    pub fn parse(footprint: &str, pad: &str) -> Option<Self> {
        let digits = footprint.strip_prefix("footprint-")?;
        let id = digits.parse::<u64>().ok()?;
        let number = pad.parse::<u8>().ok()?;
        (id != 0 && digits == id.to_string() && number != 0 && number.to_string() == pad).then_some(Self { board: None, footprint: id, number })
    }
    /// Qualify a legacy pad address with the board it belongs to. Resolution validates ownership.
    pub fn on_board(self, board: uuid::Uuid) -> Self {
        Self { board: Some(board), ..self }
    }
    /// Report a stable address suitable for cross-probing. Resolved addresses include their board.
    pub fn inspect(self) -> Value {
        let mut address = json!({"footprint":format!("footprint-{}",self.footprint),"pad":self.number.to_string()});
        if let Some(board) = self.board {
            address["board"] = json!(board);
        }
        address
    }
}

/// A placement links a component to one immutable definition and an explicit terminal-to-pad map.
/// Legacy records retain their original two-pad vocabulary and byte encoding.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Footprint {
    pub(crate) id: u64,
    pub(crate) component: u64,
    pub(crate) definition: String,
    pub(crate) definition_revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) library: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mapping: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "zero_pad")]
    pub(crate) pin_1_pad: u8,
    pub(crate) x_nm: i64,
    pub(crate) y_nm: i64,
    #[serde(default, skip_serializing_if = "zero_rotation")]
    pub(crate) rotation: u16,
}
/// Preserve omitted orientation on legacy records.
fn zero_rotation(rotation: &u16) -> bool {
    *rotation == 0
}
/// Explicit new mappings do not also serialize the legacy two-pad shortcut.
fn zero_pad(pad: &u8) -> bool {
    *pad == 0
}

impl Footprint {
    /// Construct the retained legacy marker definition and its two-pin mapping.
    pub(crate) fn new(id: u64, component: u64, pin_1_pad: u8, position: Point) -> Self {
        Self {
            id,
            component,
            definition: FOOTPRINT_DEFINITION.into(),
            definition_revision: FOOTPRINT_REVISION,
            library: None,
            mapping: None,
            pin_1_pad,
            x_nm: position.x_nm,
            y_nm: position.y_nm,
            rotation: 0,
        }
    }
    /// Validate placement vocabulary, bounded origin and whole-degree orientation; the board
    /// separately resolves the pinned definition and validates the electrical mapping.
    pub(crate) fn valid(&self) -> bool {
        let identity = match &self.library {
            Some(library) => {
                definition::valid_id(library)
                    && definition::valid_id(&self.definition)
                    && self.definition_revision > 0
                    && self.pin_1_pad == 0
                    && self.mapping.as_ref().is_some_and(|m| !m.is_empty() && m.len() <= 64)
            }
            None => {
                self.definition == FOOTPRINT_DEFINITION
                    && self.definition_revision == FOOTPRINT_REVISION
                    && (1..=2).contains(&self.pin_1_pad)
                    && self.mapping.is_none()
            }
        };
        identity
            && self.rotation < 360
            && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&self.x_nm)
            && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&self.y_nm)
    }
    /// Resolve the pinned library identity; legacy marker geometry is explicit compatibility behavior.
    pub(crate) fn key(&self) -> Option<definition::Key> {
        Some((self.library.clone()?, self.definition.clone(), self.definition_revision))
    }
    /// Locate a mapped physical pad from a logical terminal, preserving explicit permutation.
    fn pad_for(&self, pin: &str) -> Option<u8> {
        if let Some(mapping) = &self.mapping {
            return mapping.get(pin)?.parse().ok();
        }
        match pin {
            "1" => Some(self.pin_1_pad),
            "2" => Some(3 - self.pin_1_pad),
            _ => None,
        }
    }
    /// Find the logical terminal associated with a physical pad, without relying on position.
    fn component_pin(&self, pad: u8) -> Option<(u64, &str)> {
        if let Some(mapping) = &self.mapping {
            return mapping.iter().find(|(_, number)| number.parse::<u8>().ok() == Some(pad)).map(|(pin, _)| (self.component, pin.as_str()));
        }
        (1..=2).contains(&pad).then_some((self.component, if pad == self.pin_1_pad { "1" } else { "2" }))
    }
    /// Transform a local point around the footprint origin. Quarter turns are exact; other
    /// whole-degree rotations round local offsets to the nearest nanometer without accumulation.
    pub(crate) fn point(&self, local: Point) -> Point {
        let (x, y) = rotate(local.x_nm, local.y_nm, self.rotation);
        Point { x_nm: self.x_nm + x, y_nm: self.y_nm + y }
    }
    /// Locate a real pad center from the pinned definition, or the legacy marker's known pitch.
    fn pad_position(&self, pad: u8, board: &Board) -> Option<Point> {
        let local = if self.library.is_some() {
            board.definition_for(self)?.pads.iter().find(|p| p.number.parse::<u8>().ok() == Some(pad))?.center
        } else {
            if !(1..=2).contains(&pad) {
                return None;
            }
            Point { x_nm: if pad == 1 { -HALF_PAD_PITCH_NM } else { HALF_PAD_PITCH_NM }, y_nm: 0 }
        };
        Some(self.point(local))
    }
    /// Publish immutable identity, mapping and resolved centers for GUI and MCP clients.
    pub(crate) fn inspect(&self, project: &crate::Project) -> Value {
        let mut value = json!({"id":format!("footprint-{}",self.id),"board":project.board_target().board,
            "component":format!("component-{}",self.component),"definition":self.definition,"definition_revision":self.definition_revision,
            "x_nm":self.x_nm,"y_nm":self.y_nm,"rotation":self.rotation,
            "pins":project.board.pad_numbers(self).into_iter().filter_map(|pad| {
                let point=self.pad_position(pad,&project.board)?;
                let (_,pin)=self.component_pin(pad)?;
                let mut record=json!({"pad":Pad{board:Some(project.board_target().board),footprint:self.id,number:pad}.inspect(),
                    "terminal":project.component_terminal(self.component,pin).map(Terminal::inspect),"x_nm":point.x_nm,"y_nm":point.y_nm});
                if let Some(definition)=project.board.definition_for(self).and_then(|definition|definition.pads.iter().find(|item|item.number==pad.to_string())) {
                    record["copper_layers"]=json!(view::CopperLayers::resolve(project.board.setup.as_ref(),definition.copper));
                }
                Some(record)
            }).collect::<Vec<_>>()});
        if let Some(library) = &self.library {
            value["library"] = json!(library);
            value["mapping"] = json!(self.mapping);
        }
        value
    }
}

/// Rotate a bounded local footprint offset, with exact quarter turns and nearest-nanometer output.
pub(crate) fn rotate(x: i64, y: i64, degrees: u16) -> (i64, i64) {
    if degrees.is_multiple_of(90) {
        return crate::schematic::rotate(x, y, (degrees / 90) as u8);
    }
    let angle = f64::from(degrees).to_radians();
    ((x as f64 * angle.cos() + y as f64 * angle.sin()).round() as i64, (y as f64 * angle.cos() - x as f64 * angle.sin()).round() as i64)
}

/// Board-local placements and immutable definitions, indexed independently from view state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Board {
    pub(crate) assignments: BTreeMap<u64, assignment::Assignment>,
    pub(crate) outline: Option<outline::Outline>,
    pub(crate) drawings: Option<drawing::Drawings>,
    pub(crate) setup: Option<setup::BoardSetup>,
    pub(crate) silkscreen: Option<silkscreen::Settings>,
    pub(crate) definitions: BTreeMap<definition::Key, Arc<definition::FootprintDefinition>>,
    footprints: Vec<Footprint>,
    by_component: BTreeMap<u64, usize>,
}
impl Board {
    /// Return placement records in stable ID order for independently diffable saves.
    pub(crate) fn footprints(&self) -> &[Footprint] {
        &self.footprints
    }
    /// Resolve only the exact pinned definition; missing revisions never use the current catalog.
    pub(crate) fn definition_for(&self, footprint: &Footprint) -> Option<&definition::FootprintDefinition> {
        self.definitions.get(&footprint.key()?).map(Arc::as_ref)
    }
    /// List real pad identities, retaining the legacy two-marker definition only for old records.
    pub(crate) fn pad_numbers(&self, footprint: &Footprint) -> Vec<u8> {
        if footprint.library.is_none() {
            return vec![1, 2];
        }
        self.definition_for(footprint).into_iter().flat_map(|d| d.pads.iter()).filter_map(|p| p.number.parse().ok()).collect()
    }
    /// Require a bijection between this symbol's terminal set and the pinned footprint's pads.
    pub(crate) fn mapping_valid(&self, footprint: &Footprint, symbol: &SymbolDefinition) -> bool {
        if footprint.library.is_none() {
            return symbol.fits_two_pad_footprint();
        }
        let Some(definition) = self.definition_for(footprint) else {
            return false;
        };
        let Some(mapping) = &footprint.mapping else {
            return false;
        };
        mapping.len() == definition.pads.len()
            && mapping.len() == symbol.pins().len()
            && mapping.keys().map(String::as_str).collect::<BTreeSet<_>>() == symbol.pins().iter().map(|p| p.number.as_str()).collect()
            && mapping.values().map(String::as_str).collect::<BTreeSet<_>>() == definition.pads.iter().map(|p| p.number.as_str()).collect()
    }
    /// Append a placement after identity, mapping shape and definition resolution succeed.
    pub(crate) fn append(&mut self, footprint: Footprint) -> Result<(), &'static str> {
        if self.by_component.contains_key(&footprint.component) {
            return Err("component already has a footprint");
        }
        if !footprint.valid()
            || (footprint.library.is_some() && self.definition_for(&footprint).is_none())
            || self.footprints.last().is_some_and(|last| last.id >= footprint.id)
        {
            return Err("invalid footprint definition, pad mapping, position, or ID");
        }
        self.by_component.insert(footprint.component, self.footprints.len());
        self.footprints.push(footprint);
        Ok(())
    }
    /// Drop a deleted component's assignment, placement and unused definitions while retaining all
    /// other placements, IDs and library revisions. Undo restores the previous board snapshot.
    pub(crate) fn remove_component(&mut self, component: u64) {
        let assigned = self.assignments.remove(&component).is_some();
        if self.by_component.remove(&component).is_some() || assigned {
            self.footprints.retain(|f| f.component != component);
            self.by_component = self.footprints.iter().enumerate().map(|(i, f)| (f.component, i)).collect();
            let used: BTreeSet<_> =
                self.footprints.iter().filter_map(Footprint::key).chain(self.assignments.values().map(assignment::Assignment::key)).collect();
            self.definitions.retain(|key, _| used.contains(key));
        }
    }
    /// Find a terminal's corresponding physical pad by the explicit component mapping.
    pub(crate) fn pad_for(&self, component: u64, pin: &str) -> Option<Pad> {
        let f = self.footprints.get(*self.by_component.get(&component)?)?;
        Some(Pad { board: None, footprint: f.id, number: f.pad_for(pin)? })
    }
    /// Resolve a physical pad's board-local position without scanning the board.
    pub(crate) fn pad_position(&self, pad: Pad) -> Option<Point> {
        self.footprint_for_pad(pad)?.pad_position(pad.number, self)
    }
    /// Resolve a physical pad back to its logical terminal without inferring matching numbers.
    pub(crate) fn component_pin_for(&self, pad: Pad) -> Option<(u64, &str)> {
        self.footprint_for_pad(pad)?.component_pin(pad.number)
    }
    /// Find a placement in stable ID order; unknown IDs and pads do not authorize fallback geometry.
    fn footprint_for_pad(&self, pad: Pad) -> Option<&Footprint> {
        let index = self.footprints.binary_search_by_key(&pad.footprint, |f| f.id).ok()?;
        self.footprints.get(index)
    }
}

/// Parse bounded terminal-to-pad mappings before schema dispatch; resolution checks actual identities.
pub(crate) fn parse_mapping(value: &Value) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    if object.is_empty() || object.len() > 64 {
        return None;
    }
    object
        .iter()
        .map(|(terminal, pad)| {
            let pad = pad.as_str()?;
            let number = pad.parse::<u8>().ok()?;
            if terminal.is_empty()
                || terminal.len() > 8
                || !terminal.bytes().all(|c| c.is_ascii_alphanumeric() || b"+_-".contains(&c))
                || number == 0
                || number.to_string() != pad
            {
                return None;
            }
            Some((terminal.clone(), pad.to_owned()))
        })
        .collect()
}
