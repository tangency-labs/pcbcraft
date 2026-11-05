//! In-memory schematic placement. Positions are nanometers in schematic space.

pub(crate) mod wiring;

pub use wiring::{Terminal, Wire};

use serde_json::{Value, json};

pub const GRID_NM: i64 = 1_270_000;
pub const PIN_OFFSET_NM: i64 = 5 * GRID_NM;
pub const MAX_CENTER_NM: i64 = 1_000_000_000 - PIN_OFFSET_NM;
pub(crate) const MAX_SYMBOLS: usize = 4096;

/// Format the symbol ID used by inspection and command endpoints.
fn symbol_id(ordinal: u64) -> String {
    format!("symbol-{ordinal}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x_nm: i64,
    pub y_nm: i64,
}

/// A resistor component and its single schematic symbol, with distinct identities.
/// The session assigns the ordinal and keeps it stable through undo and redo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResistorPlacement {
    pub(crate) ordinal: u64,
    pub(crate) reference_number: usize,
    pub position: Point,
}

impl ResistorPlacement {
    pub fn reference(&self) -> String {
        format!("R{}", self.reference_number)
    }

    pub fn value(&self) -> &'static str {
        "1k"
    }

    /// Return the two terminal references in the same order as the pin positions.
    pub fn terminals(&self) -> [Terminal; 2] {
        Terminal::for_symbol(self.ordinal)
    }

    /// Return pin 1 (left) and pin 2 (right) in absolute schematic coordinates.
    pub fn pins(&self) -> [Point; 2] {
        let Point { x_nm, y_nm } = self.position;
        [Point { x_nm: x_nm - PIN_OFFSET_NM, y_nm }, Point { x_nm: x_nm + PIN_OFFSET_NM, y_nm }]
    }

    /// Report the component and symbol IDs alongside the coordinates a command client can use.
    pub(crate) fn inspect(&self) -> Value {
        let pin = |number, position: Point| json!({"number": number, "x_nm": position.x_nm, "y_nm": position.y_nm});
        let [left, right] = self.pins();
        json!({
            "id": symbol_id(self.ordinal),
            "component_id": format!("component-{}", self.ordinal),
            "kind": "resistor",
            "reference": self.reference(),
            "value": self.value(),
            "x_nm": self.position.x_nm,
            "y_nm": self.position.y_nm,
            "pins": [pin("1", left), pin("2", right)],
        })
    }
}
