//! In-memory schematic placement. Positions are nanometers in schematic space.

pub(crate) mod wiring;

pub use wiring::{Terminal, Wire};

use serde_json::{Value, json};

use crate::{PLACE_LED, PLACE_RESISTOR};

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

/// The built-in two-terminal parts available for placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Resistor,
    Led,
}

impl SymbolKind {
    /// Return the shared catalog command for placing this kind of symbol.
    pub fn placement_command(self) -> &'static str {
        match self {
            Self::Resistor => PLACE_RESISTOR,
            Self::Led => PLACE_LED,
        }
    }
}

/// Return pin 1 (left) and pin 2 (right) for a built-in symbol centered at this point.
pub fn pin_positions(Point { x_nm, y_nm }: Point) -> [Point; 2] {
    [Point { x_nm: x_nm - PIN_OFFSET_NM, y_nm }, Point { x_nm: x_nm + PIN_OFFSET_NM, y_nm }]
}

/// A component and its single schematic symbol, with distinct identities.
/// The session assigns the ordinal and keeps it stable through undo and redo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolPlacement {
    pub kind: SymbolKind,
    pub(crate) ordinal: u64,
    pub(crate) reference_number: usize,
    pub position: Point,
}

impl SymbolPlacement {
    /// Format the reference from the kind's prefix and its per-kind number.
    pub fn reference(&self) -> String {
        let prefix = match self.kind {
            SymbolKind::Resistor => 'R',
            SymbolKind::Led => 'D',
        };
        format!("{prefix}{}", self.reference_number)
    }

    /// Return the default display value for this built-in symbol.
    pub fn value(&self) -> &'static str {
        match self.kind {
            SymbolKind::Resistor => "1k",
            SymbolKind::Led => "Green LED",
        }
    }

    /// Return the two terminal references in the same order as the pin positions.
    pub fn terminals(&self) -> [Terminal; 2] {
        Terminal::for_symbol(self.ordinal)
    }

    /// Return pin 1 (left) and pin 2 (right) in absolute schematic coordinates.
    pub fn pins(&self) -> [Point; 2] {
        pin_positions(self.position)
    }

    /// Report the component and symbol IDs alongside numbered pin coordinates and, for LEDs, their polarity.
    pub(crate) fn inspect(&self) -> Value {
        let (kind, pin_names) = match self.kind {
            SymbolKind::Resistor => ("resistor", [None, None]),
            SymbolKind::Led => ("led", [Some("Anode"), Some("Cathode")]),
        };
        let pins = ["1", "2"]
            .into_iter()
            .zip(self.pins())
            .zip(pin_names)
            .map(|((number, position), name)| {
                let mut pin = json!({"number": number, "x_nm": position.x_nm, "y_nm": position.y_nm});
                if let Some(name) = name {
                    pin["name"] = json!(name);
                }
                pin
            })
            .collect::<Vec<_>>();
        json!({
            "id": symbol_id(self.ordinal),
            "component_id": format!("component-{}", self.ordinal),
            "kind": kind,
            "reference": self.reference(),
            "value": self.value(),
            "x_nm": self.position.x_nm,
            "y_nm": self.position.y_nm,
            "pins": pins,
        })
    }
}
