//! In-memory schematic placement. Positions are nanometers in schematic space.

pub(crate) mod wiring;

pub use wiring::{Connectivity, Terminal, Wire};

use serde_json::{Value, json};

pub const GRID_NM: i64 = 1_270_000;
pub const PIN_OFFSET_NM: i64 = 5 * GRID_NM;
pub const MAX_CENTER_NM: i64 = 1_000_000_000 - PIN_OFFSET_NM;
pub(crate) const MAX_SYMBOLS: usize = 4096;

/// Format the symbol ID used by inspection and command endpoints.
pub(crate) fn symbol_id(ordinal: u64) -> String {
    format!("symbol-{ordinal}")
}

/// Parse the stable symbol ID accepted by schematic commands.
pub(crate) fn parse_symbol_id(id: &str) -> Option<u64> {
    let digits = id.strip_prefix("symbol-")?;
    let number = digits.parse::<u64>().ok()?;
    (number != 0 && !digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(number)
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
    Capacitor,
    /// A two-pin connector: a body with a contact for each pin.
    Connector,
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
    &SymbolDefinition {
        id: "capacitor",
        name: "Capacitor",
        kind: SymbolKind::Capacitor,
        reference_prefix: 'C',
        value: "100nF",
        pin_names: [None, None],
    },
    &SymbolDefinition {
        id: "connector",
        name: "2-pin connector",
        kind: SymbolKind::Connector,
        reference_prefix: 'J',
        value: "Input",
        pin_names: [None, None],
    },
];

/// Resolve an exact built-in definition ID without falling back to another symbol.
pub(crate) fn builtin_symbol(id: &str) -> Option<&'static SymbolDefinition> {
    BUILTIN_SYMBOLS.iter().copied().find(|definition| definition.id == id)
}

/// Return pin 1 and pin 2 for a built-in symbol centered at this point and turned this many
/// quarter turns counterclockwise. Unturned, pin 1 is on the left and pin 2 on the right.
pub fn pin_positions(Point { x_nm, y_nm }: Point, turns: u8) -> [Point; 2] {
    [-PIN_OFFSET_NM, PIN_OFFSET_NM].map(|offset| {
        let (x, y) = rotate(offset, 0, turns);
        Point { x_nm: x_nm + x, y_nm: y_nm + y }
    })
}

/// Turn an offset from a symbol's center by quarter turns counterclockwise as seen on the
/// schematic, where y grows downward: one turn takes a pin on the right to the top.
pub fn rotate(x: i64, y: i64, turns: u8) -> (i64, i64) {
    match turns % 4 {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

/// Text a person gave a symbol in place of its automatic reference or its definition's value.
/// `None` keeps the automatic text, so a project that was never edited saves as it always did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Labels {
    pub(crate) reference: Option<String>,
    pub(crate) value: Option<String>,
}

/// Longest edited reference and value, in characters.
pub(crate) const MAX_REFERENCE: usize = 32;
pub(crate) const MAX_VALUE: usize = 64;

/// Check edited reference text: 1 to 32 characters, no spaces or control characters.
/// The command and the file loader share this, so a saved project always reopens.
pub(crate) fn check_reference(text: &str) -> Result<(), String> {
    check_text("reference", text, MAX_REFERENCE)?;
    if text.chars().any(char::is_whitespace) {
        return Err("a reference can't contain spaces".to_owned());
    }
    Ok(())
}

/// Check edited value text: 1 to 64 characters on one line, without surrounding spaces.
pub(crate) fn check_value(text: &str) -> Result<(), String> {
    check_text("value", text, MAX_VALUE)
}

/// Non-empty, trimmed, at most `limit` characters and free of control characters.
fn check_text(name: &str, text: &str, limit: usize) -> Result<(), String> {
    if text.is_empty() || text.trim() != text || text.chars().count() > limit || text.chars().any(char::is_control) {
        return Err(format!("`{name}` must be 1 to {limit} characters on one line"));
    }
    Ok(())
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
    /// Quarter turns counterclockwise, 0 to 3.
    pub(crate) rotation: u8,
}

impl SymbolPlacement {
    /// Quarter turns counterclockwise, 0 to 3.
    pub fn turns(&self) -> u8 {
        self.rotation
    }

    /// The automatic reference: the definition's prefix and the number assigned during placement.
    /// [`crate::Project::reference`] returns the text shown, which a person may have edited.
    pub fn reference(&self) -> String {
        format!("{}{}", self.definition.reference_prefix, self.reference_number)
    }

    /// Return the identifier shared by command clients and the saved schematic.
    pub fn id(&self) -> String {
        symbol_id(self.ordinal)
    }

    /// Return the definition's default value. [`crate::Project::value`] returns the text shown.
    pub fn value(&self) -> &'static str {
        self.definition.value
    }

    /// Return the two terminal references in the same order as the pin positions.
    pub fn terminals(&self) -> [Terminal; 2] {
        Terminal::for_symbol(self.ordinal)
    }

    /// Return pin 1 and pin 2 in absolute schematic coordinates, turned with the symbol.
    pub fn pins(&self) -> [Point; 2] {
        pin_positions(self.position, self.rotation)
    }

    /// Report the component and symbol IDs, the reference and value shown, the rotation, and the
    /// numbered pin coordinates with, for LEDs, their polarity.
    pub(crate) fn inspect(&self, reference: &str, value: &str) -> Value {
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
            "reference": reference,
            "value": value,
            "rotation": u16::from(self.rotation) * 90,
            "x_nm": self.position.x_nm,
            "y_nm": self.position.y_nm,
            "pins": pins,
        })
    }
}
