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
    /// A rectangular module body with named pins on its sides. Half sizes are grid steps.
    Module {
        half_width: i64,
        half_height: i64,
    },
}

/// One pin of a definition: its number, optional name, and offset from the symbol center in
/// grid steps.
#[derive(Debug, PartialEq, Eq)]
pub struct PinDefinition {
    pub number: &'static str,
    pub name: Option<&'static str>,
    pub x: i64,
    pub y: i64,
}

/// An in-memory schematic definition with a stable command ID and numbered pins.
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
    /// Pins in number order, which is also the order of [`SymbolPlacement::pins`].
    pub pins: &'static [PinDefinition],
}

impl SymbolDefinition {
    /// Return each pin's absolute position for a symbol centered at this point, in pin order.
    pub fn pin_positions(&self, Point { x_nm, y_nm }: Point) -> Vec<Point> {
        self.pins.iter().map(|pin| Point { x_nm: x_nm + pin.x * GRID_NM, y_nm: y_nm + pin.y * GRID_NM }).collect()
    }

    /// Find a pin by its number. The number returned lives as long as the definition.
    pub fn pin(&self, number: &str) -> Option<&'static PinDefinition> {
        self.pins.iter().find(|pin| pin.number == number)
    }

    /// Whether the two-pad starter footprint can map this symbol: it has exactly pins 1 and 2.
    pub(crate) fn fits_two_pad_footprint(&self) -> bool {
        self.pins.len() == 2 && self.pins.iter().zip(["1", "2"]).all(|(pin, number)| pin.number == number)
    }

    /// The position of a pin number in [`Self::pins`], or `None` when this definition has no such pin.
    pub fn pin_index(&self, number: &str) -> Option<usize> {
        self.pins.iter().position(|pin| pin.number == number)
    }
}

/// Pin 1 at the left and pin 2 at the right, five grid steps from the center.
const fn two_pins(first: Option<&'static str>, second: Option<&'static str>) -> [PinDefinition; 2] {
    [PinDefinition { number: "1", name: first, x: -5, y: 0 }, PinDefinition { number: "2", name: second, x: 5, y: 0 }]
}

static UNNAMED_PINS: [PinDefinition; 2] = two_pins(None, None);
static LED_PINS: [PinDefinition; 2] = two_pins(Some("Anode"), Some("Cathode"));

/// Shorthand for a named module pin.
const fn pin(number: &'static str, name: &'static str, x: i64, y: i64) -> PinDefinition {
    PinDefinition { number, name: Some(name), x, y }
}

pub(crate) static RESISTOR: SymbolDefinition =
    SymbolDefinition { id: "resistor", name: "Resistor", kind: SymbolKind::Resistor, reference_prefix: 'R', value: "1k", pins: &UNNAMED_PINS };

/// The built-in choices available to both the symbol picker and command clients. The module
/// definitions follow PCBcraft Next's original generic parts.
pub static BUILTIN_SYMBOLS: &[&SymbolDefinition] = &[
    &RESISTOR,
    &SymbolDefinition { id: "led", name: "LED", kind: SymbolKind::Led, reference_prefix: 'D', value: "LED", pins: &LED_PINS },
    &SymbolDefinition { id: "capacitor", name: "Capacitor", kind: SymbolKind::Capacitor, reference_prefix: 'C', value: "100nF", pins: &UNNAMED_PINS },
    &SymbolDefinition {
        id: "connector",
        name: "2-pin connector",
        kind: SymbolKind::Connector,
        reference_prefix: 'J',
        value: "Input",
        pins: &UNNAMED_PINS,
    },
    &SymbolDefinition {
        id: "controller-module",
        name: "Controller module",
        kind: SymbolKind::Module { half_width: 15, half_height: 16 },
        reference_prefix: 'U',
        value: "Fan controller",
        pins: &[
            pin("1", "3V3", -20, -12),
            pin("2", "GND", -20, 12),
            pin("3", "TEMP", -20, -4),
            pin("4", "SET", -20, 4),
            pin("5", "PWM", 20, 4),
            pin("6", "TACH", 20, -4),
            pin("7", "STATUS", 20, -12),
            pin("8", "MODE", 20, 12),
        ],
    },
    &SymbolDefinition {
        id: "fan-connector",
        name: "4-wire fan connector",
        kind: SymbolKind::Module { half_width: 7, half_height: 9 },
        reference_prefix: 'J',
        value: "12 V PWM fan",
        pins: &[pin("1", "GND", -12, -6), pin("2", "+12V", -12, -2), pin("3", "TACH", -12, 2), pin("4", "PWM", -12, 6)],
    },
    &SymbolDefinition {
        id: "regulator-module",
        name: "Regulator module",
        kind: SymbolKind::Module { half_width: 7, half_height: 8 },
        reference_prefix: 'U',
        value: "12 V to 3.3 V",
        pins: &[pin("1", "VIN", -12, -4), pin("2", "GND", -12, 4), pin("3", "3V3", 12, -4)],
    },
    &SymbolDefinition {
        id: "temperature-sensor",
        name: "Analog temperature sensor",
        kind: SymbolKind::Module { half_width: 7, half_height: 8 },
        reference_prefix: 'U',
        value: "Analog temperature",
        pins: &[pin("1", "3V3", -12, -4), pin("2", "GND", -12, 4), pin("3", "OUT", 12, 0)],
    },
];

/// Resolve an exact built-in definition ID without falling back to another symbol.
pub(crate) fn builtin_symbol(id: &str) -> Option<&'static SymbolDefinition> {
    BUILTIN_SYMBOLS.iter().copied().find(|definition| definition.id == id)
}

/// Every pin number some built-in definition uses, once each, in numeric order. The terminal
/// schema offers these; the command then checks the pin against the symbol's own definition.
pub fn all_pin_numbers() -> Vec<&'static str> {
    let mut numbers: Vec<_> = BUILTIN_SYMBOLS.iter().flat_map(|definition| definition.pins).map(|pin| pin.number).collect();
    numbers.sort_unstable_by_key(|number| (number.len(), *number));
    numbers.dedup();
    numbers
}

/// The pin number as spelled by some built-in definition, so terminals can hold it for the
/// program's lifetime. Whether a given symbol has that pin is checked against its definition.
pub(crate) fn known_pin_number(number: &str) -> Option<&'static str> {
    BUILTIN_SYMBOLS.iter().flat_map(|definition| definition.pins).map(|pin| pin.number).find(|known| *known == number)
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

    /// Return the identifier shared by command clients and the saved schematic.
    pub fn id(&self) -> String {
        symbol_id(self.ordinal)
    }

    /// Return the default display value for this built-in symbol.
    pub fn value(&self) -> &'static str {
        self.definition.value
    }

    /// Return the terminal references in the same order as the pin positions.
    pub fn terminals(&self) -> Vec<Terminal> {
        Terminal::for_symbol(self.ordinal, self.definition)
    }

    /// Return the pins' absolute schematic coordinates, in the definition's pin order.
    pub fn pins(&self) -> Vec<Point> {
        self.definition.pin_positions(self.position)
    }

    /// Report the component and symbol IDs alongside numbered pin coordinates and any pin names,
    /// such as an LED's polarity or a module's signals.
    pub(crate) fn inspect(&self) -> Value {
        let pins = self
            .definition
            .pins
            .iter()
            .zip(self.pins())
            .map(|(definition, position)| {
                let mut pin = json!({"number": definition.number, "x_nm": position.x_nm, "y_nm": position.y_nm});
                if let Some(name) = definition.name {
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
