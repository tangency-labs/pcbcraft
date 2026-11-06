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

impl SymbolDefinition {
    /// The supported unit identity, stored on placements separately from the component.
    pub(crate) fn unit_id(&self) -> &str {
        "main"
    }

    /// The definition revision pinned by a component.
    pub(crate) fn revision(&self) -> u32 {
        1
    }

    /// The initial value copied into a newly placed component.
    pub(crate) fn default_value(&self) -> &str {
        self.value
    }

    /// The letter automatic references start with, such as R for a resistor.
    pub fn reference_prefix(&self) -> char {
        self.reference_prefix
    }
}

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

/// One logical component, independent of the position of its schematic unit.
/// The component owns the definition, designator and value; footprints link to its ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    pub definition: &'static SymbolDefinition,
    pub(crate) ordinal: u64,
    pub(crate) reference_number: usize,
    pub(crate) reference: String,
    pub(crate) value: String,
}

impl Component {
    /// Return the component identity, which is independent of any symbol placement ID.
    pub fn id(&self) -> String {
        format!("component-{}", self.ordinal)
    }

    /// Return the designator shared by this component's schematic units and board footprint.
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Return the value shared by this component's schematic units.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Publish component-owned fields independently of schematic placement geometry.
    pub(crate) fn inspect(&self) -> Value {
        json!({"id": self.id(), "definition": self.definition.id, "definition_revision": self.definition.revision(),
            "reference": self.reference, "value": self.value})
    }
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

/// A placed schematic unit referencing a component. The resolved drawing is a cache;
/// only the component ID, unit ID, placement ID and geometry are saved here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolPlacement {
    /// Drawing resolved from the owning component, never separately serialized.
    pub definition: &'static SymbolDefinition,
    pub(crate) ordinal: u64,
    pub(crate) component: u64,
    pub unit: &'static str,
    pub position: Point,
    /// Quarter turns counterclockwise, 0 to 3.
    pub(crate) rotation: u8,
}

impl SymbolPlacement {
    /// Quarter turns counterclockwise, 0 to 3.
    pub fn turns(&self) -> u8 {
        self.rotation
    }

    /// Return the owning component's stable identity, independent of this placement's ID.
    pub fn component_id(&self) -> String {
        format!("component-{}", self.component)
    }

    /// Return the identifier shared by command clients and the saved schematic.
    pub fn id(&self) -> String {
        symbol_id(self.ordinal)
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
            "component_id": self.component_id(),
            "unit": self.unit,
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
