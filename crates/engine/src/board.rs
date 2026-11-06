//! Footprint placements and exact schematic-to-board links.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::schematic::{MAX_CENTER_NM, Point, Terminal};

pub mod materials;
pub mod setup;

pub(crate) const FOOTPRINT_DEFINITION: &str = "generic-through-hole-2";
pub(crate) const FOOTPRINT_REVISION: u32 = 1;
const HALF_PAD_PITCH_NM: i64 = 1_270_000;

/// A stable pad within a placed footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    footprint: u64,
    number: u8,
}

impl Pad {
    /// Resolve the exact footprint ID and physical pad number used by inspection.
    pub fn parse(footprint: &str, pad: &str) -> Option<Self> {
        let digits = footprint.strip_prefix("footprint-")?;
        let id = digits.parse::<u64>().ok()?;
        let number = match pad {
            "1" => 1,
            "2" => 2,
            _ => return None,
        };
        (id != 0 && digits == id.to_string()).then_some(Self { footprint: id, number })
    }

    /// Report the address clients can pass back to cross-probing.
    pub fn inspect(self) -> Value {
        json!({"footprint": format!("footprint-{}", self.footprint), "pad": self.number.to_string()})
    }
}

/// An authored footprint and its two-pin mapping. Positions use integer nanometers.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Footprint {
    pub(crate) id: u64,
    pub(crate) component: u64,
    pub(crate) definition: String,
    pub(crate) definition_revision: u32,
    pub(crate) pin_1_pad: u8,
    pub(crate) x_nm: i64,
    pub(crate) y_nm: i64,
}

impl Footprint {
    /// Construct a placement using the pinned starter definition.
    pub(crate) fn new(id: u64, component: u64, pin_1_pad: u8, position: Point) -> Self {
        Self {
            id,
            component,
            definition: FOOTPRINT_DEFINITION.to_owned(),
            definition_revision: FOOTPRINT_REVISION,
            pin_1_pad,
            x_nm: position.x_nm,
            y_nm: position.y_nm,
        }
    }

    /// Validate the starter footprint revision, mapping, and bounded coordinates.
    pub(crate) fn valid(&self) -> bool {
        self.definition == FOOTPRINT_DEFINITION
            && self.definition_revision == FOOTPRINT_REVISION
            && (1..=2).contains(&self.pin_1_pad)
            && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&self.x_nm)
            && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&self.y_nm)
    }

    /// Resolve a physical pad to its component and logical pin, independently of placements.
    fn component_pin(&self, pad: u8) -> (u64, &'static str) {
        (self.component, if pad == self.pin_1_pad { "1" } else { "2" })
    }

    /// Locate the pad on the pinned two-pad definition's 2.54 mm pitch.
    fn pad_position(&self, pad: u8) -> Point {
        let offset = if pad == 1 { -HALF_PAD_PITCH_NM } else { HALF_PAD_PITCH_NM };
        Point { x_nm: self.x_nm + offset, y_nm: self.y_nm }
    }

    /// Publish the stored association and its exact addresses for clients.
    pub(crate) fn inspect(&self, project: &crate::Project) -> Value {
        json!({
            "id": format!("footprint-{}", self.id),
            "component": format!("component-{}", self.component),
            "definition": self.definition,
            "definition_revision": self.definition_revision,
            "x_nm": self.x_nm,
            "y_nm": self.y_nm,
            "pins": ([1, 2]).map(|pad| json!({
                "pad": Pad { footprint: self.id, number: pad }.inspect(),
                "terminal": project.component_terminal(self.component, self.component_pin(pad).1).map(Terminal::inspect),
                "x_nm": self.pad_position(pad).x_nm,
                "y_nm": self.pad_position(pad).y_nm,
            }))
        })
    }
}

/// Board placements with indexes in both directions. The indexes are rebuilt from saved links.
#[derive(Clone, Debug, Default)]
pub(crate) struct Board {
    pub(crate) setup: Option<setup::BoardSetup>,
    footprints: Vec<Footprint>,
    by_component: BTreeMap<u64, usize>,
}

impl Board {
    /// Return footprint records in stable ID order for saving.
    pub(crate) fn footprints(&self) -> &[Footprint] {
        &self.footprints
    }

    /// Append a footprint in ID order, refusing duplicate or invalid placements.
    pub(crate) fn append(&mut self, footprint: Footprint) -> Result<(), &'static str> {
        if self.by_component.contains_key(&footprint.component) {
            return Err("component already has a footprint");
        }
        if !footprint.valid() || self.footprints.last().is_some_and(|last| last.id >= footprint.id) {
            return Err("invalid footprint definition, pad mapping, position, or ID");
        }
        self.by_component.insert(footprint.component, self.footprints.len());
        self.footprints.push(footprint);
        Ok(())
    }

    /// Drop the footprint linked to a deleted component, if it has one, and rebuild the index.
    pub(crate) fn remove_component(&mut self, component: u64) {
        if self.by_component.remove(&component).is_some() {
            self.footprints.retain(|footprint| footprint.component != component);
            self.by_component = self.footprints.iter().enumerate().map(|(index, footprint)| (footprint.component, index)).collect();
        }
    }

    /// Find a terminal's corresponding physical pad without scanning the layout.
    pub(crate) fn pad_for(&self, component: u64, pin: &str) -> Option<Pad> {
        let footprint = self.footprints.get(*self.by_component.get(&component)?)?;
        let number = if pin == "1" { footprint.pin_1_pad } else { 3 - footprint.pin_1_pad };
        Some(Pad { footprint: footprint.id, number })
    }

    /// Find a pad's location by footprint ID, in board nanometers.
    pub(crate) fn pad_position(&self, pad: Pad) -> Option<Point> {
        Some(self.footprint_for_pad(pad)?.pad_position(pad.number))
    }

    /// Find a physical pad's component and logical pin by stable footprint ID.
    pub(crate) fn component_pin_for(&self, pad: Pad) -> Option<(u64, &'static str)> {
        Some(self.footprint_for_pad(pad)?.component_pin(pad.number))
    }

    /// Look up a footprint without scanning the board.
    fn footprint_for_pad(&self, pad: Pad) -> Option<&Footprint> {
        let index = self.footprints.binary_search_by_key(&pad.footprint, |footprint| footprint.id).ok()?;
        self.footprints.get(index)
    }
}
