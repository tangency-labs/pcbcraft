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

/// The drawing used for a built-in symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Resistor,
    Led,
}

/// An in-memory schematic definition with a stable command ID and two numbered pins.
#[derive(Debug, PartialEq, Eq)]
pub struct SymbolDefinition {
    /// Stable ID accepted by the shared placement command.
    pub id: &'static str,
    /// Label shown in the symbol picker.
    pub name: &'static str,
    /// Drawing used on the schematic canvas.
    pub kind: SymbolKind,
    pub(crate) reference_prefix: char,
    value: &'static str,
    pin_names: [Option<&'static str>; 2],
}

pub(crate) static RESISTOR: SymbolDefinition =
    SymbolDefinition { id: "resistor", name: "Resistor", kind: SymbolKind::Resistor, reference_prefix: 'R', value: "1k", pin_names: [None, None] };

/// The built-in choices available to both the symbol picker and command clients.
pub static BUILTIN_SYMBOLS: &[&SymbolDefinition] = &[
    &RESISTOR,
    &SymbolDefinition {
        id: "led",
        name: "LED",
        kind: SymbolKind::Led,
        reference_prefix: 'D',
        value: "LED",
        pin_names: [Some("Anode"), Some("Cathode")],
    },
];

/// Resolve an exact built-in definition ID without falling back to another symbol.
pub(crate) fn builtin_symbol(id: &str) -> Option<&'static SymbolDefinition> {
    BUILTIN_SYMBOLS.iter().copied().find(|definition| definition.id == id)
}

/// Return pin 1 (left) and pin 2 (right) for a built-in symbol centered at this point.
pub fn pin_positions(Point { x_nm, y_nm }: Point) -> [Point; 2] {
    [Point { x_nm: x_nm - PIN_OFFSET_NM, y_nm }, Point { x_nm: x_nm + PIN_OFFSET_NM, y_nm }]
}

/// A component and its single schematic symbol, with distinct identities.
/// The session assigns the ordinal and keeps it stable through undo and redo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolPlacement {
    /// Definition selected when the symbol was placed.
    pub definition: &'static SymbolDefinition,
    pub(crate) ordinal: u64,
    pub(crate) reference_number: usize,
    pub position: Point,
}

impl SymbolPlacement {
    /// Format the reference from the definition's prefix and the number assigned during placement.
    pub fn reference(&self) -> String {
        format!("{}{}", self.definition.reference_prefix, self.reference_number)
    }

    /// Return the default display value for this built-in symbol.
    pub fn value(&self) -> &'static str {
        self.definition.value
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
        let pins = ["1", "2"]
            .into_iter()
            .zip(self.pins())
            .zip(self.definition.pin_names)
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
            "kind": self.definition.id,
            "reference": self.reference(),
            "value": self.value(),
            "x_nm": self.position.x_nm,
            "y_nm": self.position.y_nm,
            "pins": pins,
        })
    }
}
