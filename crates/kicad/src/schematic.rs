//! One KiCad schematic file (`.kicad_sch`, KiCad 6 to 9), read into a KiCad-shaped model.
//!
//! The reader keeps the file's own terms and frames: sheet points have y growing downward,
//! symbol-library points y growing upward, angles are counterclockwise millidegrees, and
//! lengths are nanometres. It resolves what the format leaves to the reader:
//!
//! - A library symbol that `extends` another draws as its parent does; only its own fields
//!   differ. The drawing is copied from the parent chain.
//! - A placed symbol's reference and unit come from its instance record for this file's own
//!   place (KiCad 7 and later), else its first instance record, else the file's
//!   `symbol_instances` list (KiCad 6), else its `Reference` field.
//!
//! Sheet symbols are read but their files are not: a caller that only accepts self-contained
//! schematics can refuse them by looking at [`Schematic::sheets`].
//!
//! Adapted from CircuitCraft's `crates/import/src/kicad_sch.rs` (MIT OR Apache-2.0); see
//! ATTRIBUTION.md.

use crate::sexp::{Node, Reader};
use crate::{Error, Notes, Point, millidegrees, nm};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// The largest schematic file accepted, in bytes.
pub const MAX_BYTES: usize = 64 << 20;

/// The most items (placed symbols, wires, labels, pins and drawings together) one file may give.
pub const MAX_ITEMS: usize = 1_000_000;

/// The newest file version this reader was written against (KiCad 9). Newer files are read as
/// far as they go, with a note.
const NEWEST_KNOWN: f64 = 20_250_114.0;

/// How many `extends` steps are followed before a chain counts as broken.
const MAX_EXTENDS: usize = 16;

/// A schematic file as KiCad wrote it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schematic {
    /// The file format version, a date such as `20231120`.
    pub version: Option<u64>,
    /// The file's own identity, lowercase.
    pub uuid: Option<String>,
    /// The sheet's paper, when the file names a known size.
    pub paper: Option<Paper>,
    /// The symbols placed symbols draw with, by the name placed symbols use.
    pub lib_symbols: BTreeMap<String, LibSymbol>,
    /// Placed symbols, in file order.
    pub symbols: Vec<PlacedSymbol>,
    /// Wires, each a run of points.
    pub wires: Vec<Line>,
    /// Buses, each a run of points; carried for callers that refuse or convert them.
    pub buses: Vec<Line>,
    /// Bus entries; carried for callers that refuse or convert them.
    pub bus_entries: Vec<BusEntry>,
    /// Junction dots.
    pub junctions: Vec<Marker>,
    /// No-connect marks.
    pub no_connects: Vec<Marker>,
    /// Local, global and hierarchical labels, in file order.
    pub labels: Vec<Label>,
    /// Sheet symbols, each naming a file this reader does not open.
    pub sheets: Vec<SheetSymbol>,
    /// Free text on the sheet.
    pub texts: Vec<Text>,
    /// Drawings on the sheet.
    pub graphics: Vec<Graphic>,
    /// What was left out that doesn't change connections (drawings, text, appearance), and how
    /// often.
    pub notes: Notes,
    /// What was left out that can carry connections: wires, junctions, labels, pins, placed
    /// symbols, sheet symbols and items this reader doesn't know. A caller that must keep every
    /// connection refuses a file with any.
    pub lost: Notes,
}

/// A sheet's paper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paper {
    /// The size's name as the file gives it, such as `A4` or `User`.
    pub name: String,
    /// Width and height as drawn, after `portrait` is applied.
    pub width_nm: i64,
    pub height_nm: i64,
    pub portrait: bool,
}

/// A symbol of the file's `lib_symbols`, with any `extends` resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibSymbol {
    /// The name placed symbols use, such as `Device:R`.
    pub name: String,
    /// Whether the symbol is marked as a power symbol.
    pub power: bool,
    /// The symbol's own fields: reference prefix, value, footprint and the rest.
    pub properties: Vec<Property>,
    /// Whether the symbol hides its pins' names.
    pub pin_names_hidden: bool,
    /// Whether the symbol hides its pins' numbers.
    pub pin_numbers_hidden: bool,
    /// The drawing, by unit and body style. Unit 0 is common to every unit. Symbol frame.
    pub units: Vec<SymbolUnit>,
}

impl LibSymbol {
    /// The value of the field named `key`.
    pub fn property(&self, key: &str) -> Option<&str> {
        property(&self.properties, key)
    }
}

/// One unit and body style of a library symbol, from a child symbol named `NAME_UNIT_STYLE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolUnit {
    /// 0 for what all units share, else the unit's number from 1.
    pub unit: u32,
    /// 1 for the normal body style, 2 for the alternate (De Morgan) one.
    pub style: u32,
    pub name: Option<String>,
    pub graphics: Vec<Graphic>,
    pub texts: Vec<Text>,
    pub pins: Vec<Pin>,
}

/// A pin of a library symbol. Symbol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub electrical: Electrical,
    /// How the pin is drawn, such as `line`, `inverted` or `clock`.
    pub style: String,
    /// The connection point: the pin's outer end, where wires attach.
    pub at: Point,
    /// The direction from the connection point toward the symbol body.
    pub rotation_mdeg: i32,
    /// The lead's length from the connection point toward the body; 0 draws no lead.
    pub length_nm: i64,
    /// The pin's name; empty when the file writes `~`.
    pub name: String,
    pub number: String,
    /// Whether the pin is hidden. A hidden pin still connects.
    pub hidden: bool,
}

/// A pin's electrical type, as the format names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Electrical {
    Input,
    Output,
    Bidirectional,
    TriState,
    Passive,
    Free,
    Unspecified,
    PowerIn,
    PowerOut,
    OpenCollector,
    OpenEmitter,
    NoConnect,
}

/// A placed symbol: one unit of a component, or a power symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedSymbol {
    /// The file's identity for this placement, lowercase.
    pub uuid: Option<String>,
    /// The name of the library symbol it draws with: `lib_name` when the file gives one, else
    /// `lib_id`.
    pub lib_symbol: String,
    /// The library link as written, such as `Device:R`.
    pub lib_id: String,
    /// The symbol's origin on the sheet. Sheet frame.
    pub at: Point,
    /// Counterclockwise turn, in millidegrees from 0 to 359 999.
    pub rotation_mdeg: i32,
    pub mirror: Mirror,
    /// The unit placed, from 1.
    pub unit: u32,
    /// The resolved reference, such as `R1`; `R?` when unannotated, `?` when the file gives none.
    pub reference: String,
    /// Where [`Self::reference`] and [`Self::unit`] came from.
    pub reference_from: ReferenceSource,
    /// Every instance record for this placed symbol: its own `instances` list (KiCad 7 and
    /// later), then the file's `symbol_instances` entry for it (KiCad 6). Paths are lowercase.
    pub instances: Vec<Instance>,
    /// Every field the placed symbol carries, including `Reference` and `Value`. Sheet frame.
    pub properties: Vec<Property>,
    /// Whether the part is listed in the bill of materials.
    pub in_bom: bool,
    /// Whether the part belongs on the board.
    pub on_board: bool,
    /// Whether the part is marked do-not-populate.
    pub dnp: bool,
}

impl PlacedSymbol {
    /// The value of the field named `key`.
    pub fn property(&self, key: &str) -> Option<&str> {
        property(&self.properties, key)
    }
}

/// One instance record: a placed symbol's reference and unit at one place in a hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// The place: the root file's identity and the sheet symbols' identities down to the
    /// symbol's sheet (KiCad 7 and later), or those identities and the symbol's own (KiCad 6).
    pub path: String,
    pub reference: String,
    pub unit: Option<u32>,
}

/// Which source gave a placed symbol's reference, in the order the reader tries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReferenceSource {
    /// Its instance record for this file's own place (KiCad 7 and later).
    OwnRecord,
    /// Its first instance record, which is for another place or project.
    OtherRecord,
    /// The file's `symbol_instances` list (KiCad 6).
    SymbolInstances,
    /// Its `Reference` field.
    Field,
    /// Nothing: the reference is `?`.
    #[default]
    None,
}

/// How a placed symbol is mirrored, applied after its rotation, about the symbol's own axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mirror {
    #[default]
    None,
    /// Mirrored about the x axis: top and bottom swap.
    X,
    /// Mirrored about the y axis: left and right swap.
    Y,
}

/// A field (property) and how it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub value: String,
    /// Where the text is anchored, in the frame of what carries it.
    pub at: Option<Point>,
    /// The text's counterclockwise turn, in millidegrees.
    pub rotation_mdeg: i32,
    /// Whether the text is hidden.
    pub hidden: bool,
    /// The character height.
    pub size_nm: i64,
    pub justify: Justify,
}

/// A text's horizontal anchoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Justify {
    Left,
    #[default]
    Center,
    Right,
}

/// A wire or bus: a run of points. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The file's identity for this item, lowercase.
    pub uuid: Option<String>,
    pub points: Vec<Point>,
}

/// A bus entry: a short diagonal from `at` to `at + size`. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusEntry {
    /// The file's identity for this item, lowercase.
    pub uuid: Option<String>,
    pub at: Point,
    pub size: Point,
}

/// A junction or a no-connect marker at a point. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The file's identity for this item, lowercase.
    pub uuid: Option<String>,
    pub at: Point,
}

/// A net label. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The file's identity for this item, lowercase.
    pub uuid: Option<String>,
    pub kind: LabelKind,
    pub name: String,
    /// The connection point.
    pub at: Point,
    /// The text's counterclockwise turn, in millidegrees.
    pub rotation_mdeg: i32,
    /// The shape of a global or hierarchical label, such as `input` or `passive`.
    pub shape: Option<String>,
    /// The character height.
    pub size_nm: i64,
}

/// What a label's name reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKind {
    /// The same name on the same sheet.
    Local,
    /// The same name anywhere in the schematic.
    Global,
    /// A sheet's interface to the sheet above.
    Hierarchical,
}

/// A sheet symbol: a box standing for another file. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetSymbol {
    /// The file's identity for this item, lowercase.
    pub uuid: Option<String>,
    pub name: String,
    /// The file it names, with `/` between folders.
    pub file: String,
    pub at: Point,
    pub size: Point,
    pub pins: Vec<SheetPin>,
}

/// A sheet symbol's pin. Sheet frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetPin {
    pub name: String,
    pub shape: Option<String>,
    pub at: Point,
}

/// Text on a sheet or in a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub at: Point,
    /// Counterclockwise turn, in millidegrees.
    pub rotation_mdeg: i32,
    /// The character height.
    pub size_nm: i64,
    pub justify: Justify,
    /// Whether the text is hidden.
    pub hidden: bool,
}

/// A drawing on a sheet or in a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graphic {
    pub shape: Shape,
    /// Stroke width; 0 means the default width.
    pub stroke_nm: i64,
    /// How the shape is filled; only closed shapes are filled.
    pub fill: Fill,
}

/// The geometry of a drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Polyline(Vec<Point>),
    Rectangle {
        start: Point,
        end: Point,
    },
    Circle {
        center: Point,
        radius_nm: i64,
    },
    /// A circular arc through three points.
    Arc {
        start: Point,
        mid: Point,
        end: Point,
    },
    /// A cubic Bézier curve by its four control points.
    Bezier([Point; 4]),
}

/// How a drawing is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fill {
    #[default]
    None,
    /// Filled with the stroke's colour.
    Outline,
    /// Filled with the body background colour.
    Background,
    /// Filled with a colour of its own.
    Color,
}

/// Read one schematic file. Fails only when the text is not a KiCad schematic, is malformed or
/// is larger than [`MAX_BYTES`] or [`MAX_ITEMS`]; anything else that can't be read is left out
/// and counted in [`Schematic::notes`].
pub fn read_schematic(text: &str) -> Result<Schematic, Error> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge(format!("the schematic is larger than {} MiB", MAX_BYTES >> 20)));
    }
    let (mut reader, head) = Reader::open(text)?;
    if head != "kicad_sch" {
        return Err(Error::WrongFormat("a KiCad schematic"));
    }
    let mut state = State::default();
    while let Some(item) = reader.next_item()? {
        state.item(&item)?;
    }
    state.finish()
}

/// A placed symbol's instance records, kept until the file's own identity and KiCad 6's
/// `symbol_instances` are known.
#[derive(Default)]
struct Instances {
    /// `(path, reference, unit)` from the symbol's own `instances` list, in file order.
    records: Vec<(String, String, Option<u32>)>,
}

/// The reader's state across one file.
#[derive(Default)]
struct State {
    schematic: Schematic,
    /// Library symbols as written, by name, until every `extends` can be resolved.
    raw_lib: BTreeMap<String, RawLibSymbol>,
    /// Each placed symbol's instance records, by its index in `schematic.symbols`.
    instances: Vec<Instances>,
    /// KiCad 6's `symbol_instances`: reference and unit by path, lowercase.
    legacy: BTreeMap<String, (String, Option<u32>)>,
    items: usize,
}

/// A library symbol as written, before its `extends` is resolved.
struct RawLibSymbol {
    extends: Option<String>,
    power: bool,
    properties: Vec<Property>,
    pin_names_hidden: bool,
    pin_numbers_hidden: bool,
    units: Vec<SymbolUnit>,
}

impl State {
    /// Count `n` more items, failing past [`MAX_ITEMS`].
    fn count(&mut self, n: usize) -> Result<(), Error> {
        self.items = self.items.saturating_add(n);
        if self.items > MAX_ITEMS {
            return Err(Error::TooLarge(format!("the schematic has more than {MAX_ITEMS} items")));
        }
        Ok(())
    }

    /// One top-level item of the file.
    fn item(&mut self, node: &Node) -> Result<(), Error> {
        match node.head().unwrap_or_default() {
            "version" => {
                let version = node.number(0);
                if version.is_some_and(|v| v > NEWEST_KNOWN) {
                    self.schematic.notes.add("file newer than KiCad 9: read as far as it goes");
                }
                self.schematic.version = version.filter(|v| *v >= 0.0 && v.fract() == 0.0 && *v <= u64::MAX as f64).map(|v| v as u64);
            }
            "uuid" => self.schematic.uuid = uuid(node),
            "paper" => self.schematic.paper = self.paper(node),
            "lib_symbols" => self.lib_symbols(node)?,
            kind @ ("junction" | "no_connect") => match point(node.child("at"), 0) {
                Some(at) => {
                    self.count(1)?;
                    let marker = Marker { uuid: node.child("uuid").and_then(uuid), at };
                    if kind == "junction" { self.schematic.junctions.push(marker) } else { self.schematic.no_connects.push(marker) }
                }
                None => self.schematic.lost.add(&format!("{} out of range left out", kind.replace('_', "-"))),
            },
            "bus_entry" => match (point(node.child("at"), 0), point(node.child("size"), 0)) {
                (Some(at), Some(size)) => {
                    self.count(1)?;
                    self.schematic.bus_entries.push(BusEntry { uuid: node.child("uuid").and_then(uuid), at, size });
                }
                _ => self.schematic.lost.add("bus entry out of range left out"),
            },
            kind @ ("wire" | "bus") => match points(node) {
                Some(points) if points.len() >= 2 => {
                    self.count(1)?;
                    let line = Line { uuid: node.child("uuid").and_then(uuid), points };
                    if kind == "wire" { self.schematic.wires.push(line) } else { self.schematic.buses.push(line) }
                }
                _ => self.schematic.lost.add(&format!("{kind} without two points in range left out")),
            },
            kind @ ("polyline" | "rectangle" | "circle" | "arc" | "bezier") => match graphic(node, kind) {
                Some(graphic) => {
                    self.count(1)?;
                    self.schematic.graphics.push(graphic);
                }
                None => self.schematic.notes.add("sheet drawing out of range left out"),
            },
            "text" => match text(node, false) {
                Some(text) => {
                    self.count(1)?;
                    self.schematic.texts.push(text);
                }
                None => self.schematic.notes.add("sheet text that is blank or out of range left out"),
            },
            kind @ ("label" | "global_label" | "hierarchical_label") => self.label(node, kind)?,
            "symbol" => self.placed_symbol(node)?,
            "sheet" => self.sheet(node)?,
            "symbol_instances" => self.legacy_instances(node),
            "text_box" => self.schematic.notes.add("text box left out"),
            "image" => self.schematic.notes.add("image left out"),
            "bus_alias" => self.schematic.notes.add("bus alias left out"),
            "netclass_flag" | "directive_label" => self.schematic.notes.add("net class directive left out"),
            "rule_area" => self.schematic.notes.add("rule area left out"),
            "table" => self.schematic.notes.add("table left out"),
            "generator" | "generator_version" | "title_block" | "sheet_instances" | "embedded_fonts" | "embedded_files" => {}
            other => self.schematic.lost.add(&format!("{} left out", other.chars().take(40).collect::<String>())),
        }
        Ok(())
    }

    /// The paper of a `paper` entry: a named size, or `User` with its width and height.
    fn paper(&mut self, node: &Node) -> Option<Paper> {
        // Sizes in millimetres, landscape.
        let name = node.text(0)?;
        let size = match name {
            "A5" => Some((210.0, 148.0)),
            "A4" => Some((297.0, 210.0)),
            "A3" => Some((420.0, 297.0)),
            "A2" => Some((594.0, 420.0)),
            "A1" => Some((841.0, 594.0)),
            "A0" => Some((1189.0, 841.0)),
            "A" | "USLetter" => Some((279.4, 215.9)),
            "B" | "USLedger" => Some((431.8, 279.4)),
            "USLegal" => Some((355.6, 215.9)),
            "C" => Some((558.8, 431.8)),
            "D" => Some((863.6, 558.8)),
            "E" => Some((1117.6, 863.6)),
            "User" => node.number(1).zip(node.number(2)),
            _ => None,
        };
        let portrait = node.flag("portrait");
        let size = size.and_then(|(w, h)| Some((positive(w)?, positive(h)?)));
        let Some((w, h)) = size else {
            self.schematic.notes.add("paper size not known left out");
            return None;
        };
        let (width_nm, height_nm) = if portrait { (h, w) } else { (w, h) };
        Some(Paper { name: name.to_owned(), width_nm, height_nm, portrait })
    }

    /// File the symbols of a `lib_symbols` list under their names; the first of a name counts.
    fn lib_symbols(&mut self, node: &Node) -> Result<(), Error> {
        for symbol in node.all("symbol") {
            let Some(name) = symbol.text(0) else {
                self.schematic.lost.add("library symbol without a name left out");
                continue;
            };
            if self.raw_lib.contains_key(name) {
                self.schematic.lost.add("library symbol named twice: the first kept");
                continue;
            }
            let raw = self.raw_lib_symbol(symbol)?;
            self.raw_lib.insert(name.to_owned(), raw);
        }
        Ok(())
    }

    /// A library symbol as written: its fields and flags, and its units by `NAME_UNIT_STYLE`.
    fn raw_lib_symbol(&mut self, node: &Node) -> Result<RawLibSymbol, Error> {
        let mut units = Vec::new();
        for unit in node.all("symbol") {
            let Some((number, style)) = unit.text(0).and_then(unit_and_style) else {
                self.schematic.lost.add("symbol unit whose name gives no unit left out");
                continue;
            };
            let mut out = SymbolUnit {
                unit: number,
                style,
                name: unit.child("unit_name").and_then(|n| n.text(0)).map(str::to_owned),
                graphics: Vec::new(),
                texts: Vec::new(),
                pins: Vec::new(),
            };
            for part in unit.args() {
                match part.head().unwrap_or_default() {
                    kind @ ("polyline" | "rectangle" | "circle" | "arc" | "bezier") => match graphic(part, kind) {
                        Some(g) => out.graphics.push(g),
                        None => self.schematic.notes.add("symbol drawing out of range left out"),
                    },
                    "text" => match text(part, true) {
                        Some(t) => out.texts.push(t),
                        None => self.schematic.notes.add("symbol text that is blank or out of range left out"),
                    },
                    "pin" => match self.pin(part) {
                        Some(p) => out.pins.push(p),
                        None => self.schematic.lost.add("pin without a number or out of range left out"),
                    },
                    "text_box" => self.schematic.notes.add("text box left out"),
                    _ => {}
                }
            }
            self.count(out.graphics.len().saturating_add(out.texts.len()).saturating_add(out.pins.len()).saturating_add(1))?;
            units.push(out);
        }
        let properties = properties(node);
        self.count(properties.len())?;
        Ok(RawLibSymbol {
            extends: node.child("extends").and_then(|e| e.text(0)).map(str::to_owned),
            power: node.child("power").is_some() || node.flag("power"),
            properties,
            pin_names_hidden: node.child("pin_names").is_some_and(hidden),
            pin_numbers_hidden: node.child("pin_numbers").is_some_and(hidden),
            units,
        })
    }

    /// A pin of a symbol unit; `None` without a number or a position in range.
    fn pin(&mut self, node: &Node) -> Option<Pin> {
        let electrical = match node.text(0)? {
            "input" => Electrical::Input,
            "output" => Electrical::Output,
            "bidirectional" => Electrical::Bidirectional,
            "tri_state" => Electrical::TriState,
            "passive" => Electrical::Passive,
            "free" => Electrical::Free,
            "unspecified" => Electrical::Unspecified,
            "power_in" => Electrical::PowerIn,
            "power_out" => Electrical::PowerOut,
            "open_collector" => Electrical::OpenCollector,
            "open_emitter" => Electrical::OpenEmitter,
            "no_connect" => Electrical::NoConnect,
            _ => {
                self.schematic.notes.add("pin of an unknown electrical type read as unspecified");
                Electrical::Unspecified
            }
        };
        let at = node.child("at")?;
        let number = node.child("number").and_then(|n| n.text(0))?;
        if number.trim().is_empty() {
            return None;
        }
        Some(Pin {
            electrical,
            style: node.text(1).unwrap_or("line").to_owned(),
            at: xy(at, 0)?,
            rotation_mdeg: millidegrees(at.number(2).unwrap_or(0.0)),
            length_nm: nm(node.child("length").and_then(|l| l.number(0)).unwrap_or(0.0)).filter(|l| *l >= 0)?,
            name: node.child("name").and_then(|n| n.text(0)).map(|n| if n == "~" { String::new() } else { n.to_owned() }).unwrap_or_default(),
            number: number.to_owned(),
            hidden: hidden(node),
        })
    }

    /// A net label, global label or hierarchical label; blank or out-of-range ones are noted.
    fn label(&mut self, node: &Node, kind: &str) -> Result<(), Error> {
        let name = node.text(0).unwrap_or_default();
        let at = node.child("at");
        let (Some(position), false) = (point(at, 0), name.trim().is_empty()) else {
            self.schematic.lost.add("label that is blank or out of range left out");
            return Ok(());
        };
        self.count(1)?;
        self.schematic.labels.push(Label {
            uuid: node.child("uuid").and_then(uuid),
            kind: match kind {
                "global_label" => LabelKind::Global,
                "hierarchical_label" => LabelKind::Hierarchical,
                _ => LabelKind::Local,
            },
            name: name.to_owned(),
            at: position,
            rotation_mdeg: millidegrees(at.and_then(|a| a.number(2)).unwrap_or(0.0)),
            shape: node.child("shape").and_then(|s| s.text(0)).map(str::to_owned),
            size_nm: text_size(node),
        });
        Ok(())
    }

    /// A placed symbol. Its reference and unit are settled in [`State::finish`].
    fn placed_symbol(&mut self, node: &Node) -> Result<(), Error> {
        let lib_id = node.child("lib_id").and_then(|n| n.text(0)).unwrap_or_default();
        let lib_symbol = node.child("lib_name").and_then(|n| n.text(0)).unwrap_or(lib_id);
        let at = node.child("at");
        let (Some(position), false) = (point(at, 0), lib_symbol.is_empty()) else {
            self.schematic.lost.add("placed symbol without a library symbol or out of range left out");
            return Ok(());
        };
        let records = node
            .child("instances")
            .into_iter()
            .flat_map(|i| i.all("project"))
            .flat_map(|p| p.all("path"))
            .filter_map(|r| Some((r.text(0)?.to_ascii_lowercase(), r.child("reference")?.text(0)?.to_owned(), unit_of(r))))
            .collect::<Vec<_>>();
        let properties = properties(node);
        self.count(properties.len().saturating_add(records.len()).saturating_add(1))?;
        let says = |key: &str, default: bool| match node.child(key).and_then(|n| n.text(0)) {
            Some("yes") => true,
            Some("no") => false,
            _ => default,
        };
        self.schematic.symbols.push(PlacedSymbol {
            uuid: node.child("uuid").and_then(uuid),
            lib_symbol: lib_symbol.to_owned(),
            lib_id: lib_id.to_owned(),
            at: position,
            rotation_mdeg: millidegrees(at.and_then(|a| a.number(2)).unwrap_or(0.0)),
            mirror: match node.child("mirror").and_then(|m| m.text(0)) {
                Some("x") => Mirror::X,
                Some("y") => Mirror::Y,
                _ => Mirror::None,
            },
            // Settled in `finish`; the symbol's own `(unit N)` until then.
            unit: unit_of(node).unwrap_or(1),
            reference: String::new(),
            reference_from: ReferenceSource::None,
            instances: Vec::new(),
            properties,
            in_bom: says("in_bom", true),
            on_board: says("on_board", true),
            dnp: says("dnp", false),
        });
        self.instances.push(Instances { records });
        Ok(())
    }

    /// A sheet symbol, read but not followed.
    fn sheet(&mut self, node: &Node) -> Result<(), Error> {
        let fields = properties(node);
        let field = |keys: [&str; 2]| keys.iter().find_map(|k| property(&fields, k)).unwrap_or_default().to_owned();
        let (Some(at), Some(size)) = (point(node.child("at"), 0), point(node.child("size"), 0)) else {
            self.schematic.lost.add("sheet symbol out of range left out");
            return Ok(());
        };
        let mut pins = Vec::new();
        for pin in node.all("pin") {
            match (pin.text(0), point(pin.child("at"), 0)) {
                (Some(name), Some(at)) if !name.trim().is_empty() => {
                    pins.push(SheetPin { name: name.to_owned(), shape: pin.text(1).map(str::to_owned), at })
                }
                _ => self.schematic.lost.add("sheet pin that is blank or out of range left out"),
            }
        }
        self.count(pins.len().saturating_add(1))?;
        self.schematic.sheets.push(SheetSymbol {
            uuid: node.child("uuid").and_then(uuid),
            name: field(["Sheetname", "Sheet name"]),
            file: file_name(&field(["Sheetfile", "Sheet file"])),
            at,
            size,
            pins,
        });
        Ok(())
    }

    /// KiCad 6's `symbol_instances`: each placed symbol's reference and unit by its path.
    fn legacy_instances(&mut self, node: &Node) {
        for record in node.all("path") {
            let (Some(path), Some(reference)) = (record.text(0), record.child("reference").and_then(|r| r.text(0))) else {
                continue;
            };
            if self.legacy.len() >= MAX_ITEMS {
                break;
            }
            self.legacy.entry(path.to_ascii_lowercase()).or_insert_with(|| (reference.to_owned(), unit_of(record)));
        }
    }

    /// Resolve every library symbol's `extends` and every placed symbol's reference and unit.
    fn finish(mut self) -> Result<Schematic, Error> {
        let names: Vec<String> = self.raw_lib.keys().cloned().collect();
        for name in names {
            if let Some(symbol) = self.resolved(&name) {
                self.schematic.lib_symbols.insert(name, symbol);
            }
        }
        // A file's own place in the hierarchy is its identity; KiCad 6 keys a top-level symbol
        // by its own identity alone.
        let path = self.schematic.uuid.as_ref().map(|id| format!("/{id}"));
        for (symbol, instances) in self.schematic.symbols.iter_mut().zip(&self.instances) {
            symbol.instances = instances
                .records
                .iter()
                .map(|(path, reference, unit)| Instance { path: path.clone(), reference: reference.clone(), unit: *unit })
                .collect();
            let legacy_path = symbol.uuid.as_ref().map(|id| format!("/{id}"));
            if let Some((path, (reference, unit))) = legacy_path.as_ref().and_then(|p| self.legacy.get(p).map(|record| (p, record))) {
                symbol.instances.push(Instance { path: path.clone(), reference: reference.clone(), unit: *unit });
            }
            let own = path.as_ref().and_then(|path| instances.records.iter().find(|(p, _, _)| p == path));
            let record = |(_, reference, unit): &(String, String, Option<u32>)| (reference.clone(), *unit);
            let (reference, unit, from) = if let Some(found) = own {
                let (reference, unit) = record(found);
                (reference, unit, ReferenceSource::OwnRecord)
            } else if let Some(found) = instances.records.first() {
                let (reference, unit) = record(found);
                (reference, unit, ReferenceSource::OtherRecord)
            } else if let Some((reference, unit)) = legacy_path.as_ref().and_then(|p| self.legacy.get(p)) {
                (reference.clone(), *unit, ReferenceSource::SymbolInstances)
            } else if let Some(reference) = symbol.property("Reference") {
                (reference.to_owned(), None, ReferenceSource::Field)
            } else {
                (String::new(), None, ReferenceSource::None)
            };
            let blank = reference.trim().is_empty();
            symbol.reference = if blank { "?".to_owned() } else { reference };
            symbol.reference_from = if blank { ReferenceSource::None } else { from };
            if let Some(unit) = unit {
                symbol.unit = unit;
            }
        }
        Ok(self.schematic)
    }

    /// A library symbol with its drawing taken from the end of its `extends` chain and its own
    /// fields kept. `None`, with a note, when the chain breaks or is longer than
    /// [`MAX_EXTENDS`].
    fn resolved(&mut self, name: &str) -> Option<LibSymbol> {
        let own = self.raw_lib.get(name)?;
        let mut drawn = own;
        for _ in 0..=MAX_EXTENDS {
            let Some(parent) = &drawn.extends else {
                return Some(LibSymbol {
                    name: name.to_owned(),
                    power: own.power || drawn.power,
                    properties: own.properties.clone(),
                    pin_names_hidden: drawn.pin_names_hidden,
                    pin_numbers_hidden: drawn.pin_numbers_hidden,
                    units: drawn.units.clone(),
                });
            };
            // A parent is named by its full name, or by its entry name within the same library.
            let library = name.rsplit_once(':').map(|(library, _)| library);
            let full = library.map(|library| format!("{library}:{parent}"));
            match self.raw_lib.get(parent.as_str()).or_else(|| full.and_then(|f| self.raw_lib.get(&f))) {
                Some(next) => drawn = next,
                None => break,
            }
        }
        self.schematic.notes.add("library symbol whose `extends` chain is broken left out");
        None
    }
}

/// The value of the field named `key` among `properties`.
fn property<'p>(properties: &'p [Property], key: &str) -> Option<&'p str> {
    properties.iter().find(|p| p.key == key).map(|p| p.value.as_str())
}

/// The fields of a node, in file order, with positions in the node's own frame.
fn properties(node: &Node) -> Vec<Property> {
    node.all("property")
        .filter_map(|p| {
            let at = p.child("at");
            Some(Property {
                key: p.text(0)?.to_owned(),
                value: p.text(1)?.to_owned(),
                at: at.and_then(|a| xy(a, 0)),
                rotation_mdeg: millidegrees(at.and_then(|a| a.number(2)).unwrap_or(0.0)),
                hidden: hidden(p),
                size_nm: text_size(p),
                justify: justify(p),
            })
        })
        .collect()
}

/// A unit's identity from its child symbol's name, `NAME_UNIT_STYLE`.
fn unit_and_style(name: &str) -> Option<(u32, u32)> {
    let mut parts = name.rsplitn(3, '_');
    let style = parts.next()?.parse().ok()?;
    let unit = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((unit, style))
}

/// The unit a node's `(unit N)` names, if it is a whole number from 1.
fn unit_of(node: &Node) -> Option<u32> {
    node.child("unit").and_then(|u| u.number(0)).filter(|u| *u >= 1.0 && u.fract() == 0.0 && *u <= f64::from(u32::MAX)).map(|u| u as u32)
}

/// A node's `uuid`, lowercase, when it is written as one; `None` otherwise.
fn uuid(node: &Node) -> Option<String> {
    let text = node.text(0)?;
    let well_formed =
        text.len() == 36 && text.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    well_formed.then(|| text.to_ascii_lowercase())
}

/// A file name as a sheet symbol gives it, with `/` between folders and no leading `./`.
fn file_name(name: &str) -> String {
    let name = name.replace('\\', "/");
    let mut rest = name.as_str();
    while let Some(r) = rest.strip_prefix("./") {
        rest = r;
    }
    rest.to_owned()
}

/// Whether a node, or the effects inside it, says hide: the bare word, or `(hide yes)`.
fn hidden(node: &Node) -> bool {
    let says = |n: &Node| n.flag("hide") || n.child("hide").is_some_and(|h| h.text(0) != Some("no"));
    says(node) || node.child("effects").is_some_and(says)
}

/// The two numbers from the `i`-th argument of `node` on, as a point in the node's own frame.
fn xy(node: &Node, i: usize) -> Option<Point> {
    Some(Point { x_nm: nm(node.number(i)?)?, y_nm: nm(node.number(i + 1)?)? })
}

/// [`xy`] of an optional node.
fn point(node: Option<&Node>, i: usize) -> Option<Point> {
    xy(node?, i)
}

/// A length in millimetres that must be positive.
fn positive(mm: f64) -> Option<i64> {
    nm(mm).filter(|n| *n > 0)
}

/// The points of a `pts` list; `None` when any is out of range.
fn points(node: &Node) -> Option<Vec<Point>> {
    node.child("pts")?.all("xy").map(|p| xy(p, 0)).collect()
}

/// The character height a text's effects give, or the usual 1.27 mm.
fn text_size(node: &Node) -> i64 {
    node.child("effects")
        .and_then(|e| e.child("font"))
        .and_then(|f| f.child("size"))
        .and_then(|s| s.number(0))
        .and_then(positive)
        .unwrap_or(1_270_000)
}

/// How a text's effects anchor it from side to side.
fn justify(node: &Node) -> Justify {
    match node.child("effects").and_then(|e| e.child("justify")) {
        Some(j) if j.flag("left") => Justify::Left,
        Some(j) if j.flag("right") => Justify::Right,
        _ => Justify::Center,
    }
}

/// A drawing; `None` when a point or size is missing or out of range.
fn graphic(node: &Node, kind: &str) -> Option<Graphic> {
    let at = |name: &str| node.child(name).and_then(|n| xy(n, 0));
    let shape = match kind {
        "polyline" => Shape::Polyline(points(node).filter(|p| p.len() >= 2)?),
        "bezier" => Shape::Bezier(<[Point; 4]>::try_from(points(node)?).ok()?),
        "rectangle" => Shape::Rectangle { start: at("start")?, end: at("end")? },
        "circle" => Shape::Circle { center: at("center")?, radius_nm: positive(node.child("radius")?.number(0)?)? },
        "arc" => Shape::Arc { start: at("start")?, mid: at("mid")?, end: at("end")? },
        _ => return None,
    };
    let stroke_nm = nm(node.child("stroke").and_then(|s| s.child("width")).and_then(|w| w.number(0)).unwrap_or(0.0)).filter(|w| *w >= 0)?;
    let fill = match node.child("fill").and_then(|f| f.child("type")).and_then(|t| t.text(0)) {
        Some("outline") => Fill::Outline,
        Some("background") => Fill::Background,
        Some("color") => Fill::Color,
        _ => Fill::None,
    };
    Some(Graphic { shape, stroke_nm, fill })
}

/// Text on a sheet or in a symbol; `None` when blank or out of range.
///
/// The format documents a symbol text's angle in tenths of a degree, while KiCad writes whole
/// degrees in the files observed. A value that is a quarter turn only when read as degrees is
/// read as degrees; anything else in a symbol is read as tenths.
fn text(node: &Node, in_symbol: bool) -> Option<Text> {
    let words = node.text(0)?;
    if words.trim().is_empty() {
        return None;
    }
    let at = node.child("at")?;
    let raw = at.number(2).unwrap_or(0.0);
    let degrees = if in_symbol && (raw / 10.0) % 90.0 != 0.0 && raw % 90.0 == 0.0 {
        raw
    } else if in_symbol {
        raw / 10.0
    } else {
        raw
    };
    Some(Text {
        text: words.to_owned(),
        at: xy(at, 0)?,
        rotation_mdeg: millidegrees(degrees),
        size_nm: text_size(node),
        justify: justify(node),
        hidden: hidden(node),
    })
}
