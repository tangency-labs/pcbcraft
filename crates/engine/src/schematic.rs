//! In-memory schematic placement. Positions are nanometers in schematic space.

pub(crate) mod wiring;

pub use wiring::{Connectivity, PinNumber, Terminal, Wire, WireEndpoint, orthogonal_path};

use std::sync::Arc;

use serde_json::{Value, json};

pub const GRID_NM: i64 = 1_270_000;
/// Original two-pin starter lead extent, retained by revision 1 definitions.
pub const PIN_OFFSET_NM: i64 = 5 * GRID_NM;
/// How far a library pin may sit from its symbol's center on either axis.
pub const MAX_PIN_OFFSET_NM: i64 = 2 * PIN_OFFSET_NM;
/// Symbol centers stay this far inside the sheet edge, so every library pin stays on the sheet.
pub const MAX_CENTER_NM: i64 = 1_000_000_000 - MAX_PIN_OFFSET_NM;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x_nm: i64,
    pub y_nm: i64,
}

/// How a terminal behaves electrically, as its symbol library declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElectricalType {
    Passive,
    Input,
    Output,
    Bidirectional,
    PowerIn,
    PowerOut,
    Unspecified,
}

impl ElectricalType {
    /// The name used in symbol files and inspection, such as `power_in`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::Input => "input",
            Self::Output => "output",
            Self::Bidirectional => "bidirectional",
            Self::PowerIn => "power_in",
            Self::PowerOut => "power_out",
            Self::Unspecified => "unspecified",
        }
    }
}

/// One stroke of a symbol's drawing, in nanometers from the symbol's center before rotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Line(Point, Point),
    /// Connected segments through each point in order; repeat the first point to close it.
    Polyline(Vec<Point>),
    /// An outline between two opposite corners.
    Rect(Point, Point),
    /// An outline around a center, with its radius in nanometers.
    Circle(Point, i64),
    /// A short upright label, such as a pin name, anchored at a point. It stays horizontal when
    /// the symbol turns.
    Text(Point, String, TextAlign),
}

/// Which part of a text label sits on its anchor point, before rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// The text starts at the anchor and runs right.
    Start,
    Middle,
    /// The text ends at the anchor.
    End,
}

/// One pin of a symbol: its number, where it sits before rotation, its optional name and its
/// electrical type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinDefinition {
    /// The terminal ID from the library, such as `"1"` or `"A3"`. Wires and footprints use it.
    pub number: String,
    pub offset: Point,
    pub name: Option<String>,
    pub electrical_type: ElectricalType,
}

/// A schematic symbol definition loaded from a symbol library (see [`crate::library`]). It
/// describes only the schematic symbol; layout footprints are separate definitions.
#[derive(Debug, PartialEq, Eq)]
pub struct SymbolDefinition {
    /// The ID of the library this definition came from, such as `starter`. Projects save it with
    /// their copy of the definition.
    pub library: String,
    /// Stable ID accepted by the shared placement command and saved in project files.
    pub id: String,
    /// Label shown in the parts browser.
    pub name: String,
    /// The library's revision of this definition, pinned by each component.
    pub(crate) revision: u32,
    pub(crate) reference_prefix: String,
    pub(crate) value: String,
    /// The library-defined identity of the supported unit.
    pub(crate) unit: String,
    /// Every pin, in the library's terminal order.
    pub(crate) pins: Vec<PinDefinition>,
    /// The strokes drawn on the schematic canvas.
    pub drawing: Vec<Shape>,
}

impl SymbolDefinition {
    /// Return the unit identity supplied by the library, used by saved placements.
    pub fn unit_id(&self) -> &str {
        &self.unit
    }

    /// Return the definition revision pinned by a component.
    pub(crate) fn revision(&self) -> u32 {
        self.revision
    }

    /// Return the initial value copied into a new component.
    pub fn default_value(&self) -> &str {
        &self.value
    }

    /// The letters automatic references start with, such as R for a resistor.
    pub fn reference_prefix(&self) -> &str {
        &self.reference_prefix
    }

    /// Every pin in the library's terminal order, as defined before rotation.
    pub fn pins(&self) -> &[PinDefinition] {
        &self.pins
    }

    /// The position of this pin number in [`Self::pins`], or `None` when the symbol has no such pin.
    pub fn pin_index(&self, number: &str) -> Option<usize> {
        self.pins.iter().position(|pin| pin.number == number)
    }

    /// Whether the starter two-pad footprint can map this symbol: it has exactly pins 1 and 2.
    pub(crate) fn fits_two_pad_footprint(&self) -> bool {
        self.pins.len() == 2 && self.pins.iter().zip(["1", "2"]).all(|(pin, number)| pin.number == number)
    }

    /// Return every pin's position, in [`Self::pins`] order, for this symbol centered at this
    /// point and turned this many quarter turns counterclockwise.
    pub fn pin_positions(&self, Point { x_nm, y_nm }: Point, turns: u8) -> Vec<Point> {
        self.pins
            .iter()
            .map(|pin| {
                let (x, y) = rotate(pin.offset.x_nm, pin.offset.y_nm, turns);
                Point { x_nm: x_nm + x, y_nm: y_nm + y }
            })
            .collect()
    }
}

/// Resolve an exact definition ID in the starter library without falling back to another symbol.
pub(crate) fn library_symbol(id: &str) -> Option<Arc<SymbolDefinition>> {
    crate::library::starter().symbol(id).cloned()
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
    pub definition: Arc<SymbolDefinition>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolPlacement {
    /// Drawing resolved from the owning component and shared with it, never separately serialized.
    pub definition: Arc<SymbolDefinition>,
    pub(crate) ordinal: u64,
    pub(crate) component: u64,
    pub unit: String,
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

    /// Return the terminal references in the same order as the pin positions.
    pub fn terminals(&self) -> Vec<Terminal> {
        Terminal::for_symbol(self.ordinal, &self.definition)
    }

    /// Return every pin in absolute schematic coordinates, turned with the symbol, in the
    /// definition's pin order.
    pub fn pins(&self) -> Vec<Point> {
        self.definition.pin_positions(self.position, self.rotation)
    }

    /// Report the component and symbol IDs, the reference and value shown, the rotation, and the
    /// numbered pin coordinates with their electrical types and any names, such as an LED's polarity.
    pub(crate) fn inspect(&self, reference: &str, value: &str) -> Value {
        let pins = self
            .pins()
            .into_iter()
            .zip(&self.definition.pins)
            .map(|(position, definition)| {
                let mut pin = json!({
                    "number": definition.number,
                    "electrical_type": definition.electrical_type.as_str(),
                    "x_nm": position.x_nm,
                    "y_nm": position.y_nm,
                });
                if let Some(name) = &definition.name {
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
