//! Schematic symbol libraries. A library is a folder with a `library.json` manifest and one
//! versioned JSON file per symbol in `symbols/`; see `docs/symbol-library.md` for the format.
//! Symbols describe only what the schematic draws and connects. Layout footprints are separate
//! definitions in their own libraries, linked to a symbol by assignment, so nothing here names
//! a footprint or a pad.
//!
//! The starter library in `libraries/starter` is embedded at build time, so the desktop and
//! browser builds offer the same parts without reading the file system.

use std::{
    collections::BTreeSet,
    sync::{Arc, LazyLock},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schematic::{ElectricalType, GRID_NM, MAX_PIN_OFFSET_NM, PinDefinition, Point, Shape, SymbolDefinition, TextAlign, check_value};

#[cfg(test)]
mod bundled_tests;
pub mod parts;
pub mod sources;
#[cfg(test)]
use crate::schematic::PIN_OFFSET_NM;

#[cfg(test)]
mod starter_files {
    use std::sync::LazyLock;
    pub const MANIFEST: &str = include_str!("../../../libraries/starter/library.json");
    pub static SYMBOLS: LazyLock<Vec<(&str, &str)>> = LazyLock::new(|| {
        super::bundled_files::LIBRARIES
            .iter()
            .find(|(id, _)| *id == "starter")
            .into_iter()
            .flat_map(|(_, files)| files.iter())
            .filter_map(|(path, text)| path.strip_prefix("symbols/").map(|name| (name, *text)))
            .collect()
    });
}

mod bundled_files {
    include!(concat!(env!("OUT_DIR"), "/bundled_libraries.rs"));
}

/// Embedded delivery only: each registration uses the same documents as a native folder.
pub fn bundled_documents() -> impl Iterator<Item = (&'static str, sources::Documents)> {
    bundled_files::LIBRARIES.iter().map(|(id, files)| (*id, files.iter().map(|(path, text)| ((*path).into(), (*text).into())).collect()))
}

/// The symbol file format this build reads.
pub const SYMBOL_FORMAT: &str = "pcbcraft-symbol/1";
/// Native definitions carrying explicit electrical semantics or instance defaults.
pub const SYMBOL_FORMAT_V2: &str = "pcbcraft-symbol/2";
/// Native symbols with an explicit default typed category/primary-value binding.
pub const SYMBOL_FORMAT_V3: &str = "pcbcraft-symbol/3";

/// Electrical roles survive copying and never follow from an item or library name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Behavior {
    /// Name this terminal globally using the placed component's Value.
    Power { terminal: String },
}

/// Initial fields and appearance for any native symbol; placed instances remain editable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub metadata: crate::schematic::fields::Metadata,
    pub field_layout: crate::schematic::fields::Layouts,
}

/// The library manifest format this build reads.
pub const LIBRARY_FORMAT: &str = "pcbcraft-library/1";

/// Most strokes in one symbol drawing.
const MAX_SHAPES: usize = 256;
/// Most points in one polyline.
const MAX_POINTS: usize = 64;
/// How far a drawing may reach from the symbol's center on either axis, in nanometers.
const MAX_DRAWING_NM: i64 = 100_000_000;
/// Most terminals in one symbol.
const MAX_TERMINALS: usize = 128;
/// Longest text label in a drawing, in characters.
const MAX_LABEL: usize = 16;

/// A loaded symbol library: its identity and license from the manifest, the symbols that passed
/// every check in file-name order, and a message for each file that didn't.
#[derive(Debug)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub license: String,
    symbols: Vec<Arc<SymbolDefinition>>,
    revisions: Vec<Arc<SymbolDefinition>>,
    problems: Vec<String>,
}

static STARTER: LazyLock<Library> = LazyLock::new(|| {
    let files = bundled_files::LIBRARIES.iter().find(|(id, _)| *id == "starter").map(|(_, files)| *files).unwrap_or_default();
    let manifest = files.iter().find(|(path, _)| *path == "library.json").map(|(_, text)| *text).unwrap_or_default();
    let symbols: Vec<_> = files.iter().filter_map(|(path, text)| path.strip_prefix("symbols/").map(|path| (path, *text))).collect();
    Library::load(manifest, &symbols)
});

/// The starter library shipped with PCBcraft.
pub fn starter() -> &'static Library {
    &STARTER
}

impl Library {
    /// Read a manifest and its symbol files, given as (file name, contents) pairs. A file that
    /// fails a check, or repeats an earlier symbol's ID and revision, is left out and named in
    /// [`Self::problems`], so one bad file never hides the rest of the library.
    /// The newest revision of each ID is offered for placement; older revisions remain available
    /// for reopening projects without changing their drawings or terminal positions.
    pub fn load(manifest: &str, files: &[(&str, &str)]) -> Self {
        let mut problems = Vec::new();
        let (id, name, license) = match parse_manifest(manifest) {
            Ok(manifest) => (manifest.id, manifest.name, manifest.license),
            Err(error) => {
                problems.push(format!("library.json: {error}"));
                (String::new(), String::new(), String::new())
            }
        };
        let mut ids = BTreeSet::new();
        let mut symbols: Vec<Arc<SymbolDefinition>> = Vec::with_capacity(files.len());
        let mut revisions = Vec::new();
        for (file, text) in files {
            match parse_symbol(text) {
                Ok(mut symbol) if ids.insert((symbol.id.clone(), symbol.revision)) => {
                    symbol.library.clone_from(&id);
                    let mut symbol = Arc::new(symbol);
                    if let Some(current) = symbols.iter_mut().find(|current| current.id == symbol.id) {
                        if symbol.revision > current.revision {
                            std::mem::swap(current, &mut symbol);
                        }
                        revisions.push(symbol);
                    } else {
                        symbols.push(symbol);
                    }
                }
                Ok(symbol) => problems.push(format!("{file}: another symbol already has the ID {} at revision {}", symbol.id, symbol.revision)),
                Err(error) => problems.push(format!("{file}: {error}")),
            }
        }
        Self { id, name, license, symbols, revisions, problems }
    }

    /// The latest revision of each loaded symbol, ordered by the first file for that ID.
    pub fn symbols(&self) -> &[Arc<SymbolDefinition>] {
        &self.symbols
    }

    /// The latest loaded revision of exactly this ID, for new placements.
    pub fn symbol(&self, id: &str) -> Option<&Arc<SymbolDefinition>> {
        self.symbols.iter().find(|symbol| symbol.id == id)
    }

    /// Resolve the exact revision pinned in a project without upgrading its geometry.
    pub fn symbol_revision(&self, id: &str, revision: u32) -> Option<&Arc<SymbolDefinition>> {
        self.symbols.iter().chain(&self.revisions).find(|symbol| symbol.id == id && symbol.revision == revision)
    }

    /// One message per manifest or symbol file that was left out, naming the file and the reason.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestFile {
    #[allow(dead_code, reason = "checked before the rest of the manifest is read")]
    format: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) license: String,
}

/// Read a library manifest after checking its format, ID, name and license text.
pub(super) fn parse_manifest(text: &str) -> Result<ManifestFile, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("invalid library JSON: {error}"))?;
    let manifest: ManifestFile = serde_json::from_value(versioned(value, LIBRARY_FORMAT, "library")?).map_err(|error| error.to_string())?;
    check_id(&manifest.id)?;
    check_text("name", &manifest.name, 64)?;
    check_text("license", &manifest.license, 128)?;
    Ok(manifest)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SymbolFile {
    #[allow(dead_code, reason = "checked before the rest of the file is read")]
    format: String,
    id: String,
    revision: u32,
    name: String,
    reference_prefix: String,
    default_value: String,
    #[serde(default)]
    behavior: Option<Behavior>,
    #[serde(default)]
    defaults: Option<Defaults>,
    #[serde(default)]
    category: Option<parts::CategoryBinding>,
    terminals: Vec<TerminalFile>,
    units: Vec<UnitFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalFile {
    id: String,
    #[serde(default)]
    name: Option<String>,
    electrical_type: ElectricalType,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitFile {
    id: String,
    drawing: Vec<ShapeFile>,
    pins: Vec<PinFile>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ShapeFile {
    Graphic { graphic: crate::library_graphics::Graphic },
    Line { from_nm: [i64; 2], to_nm: [i64; 2] },
    Polyline { points_nm: Vec<[i64; 2]> },
    Rect { min_nm: [i64; 2], max_nm: [i64; 2] },
    Circle { center_nm: [i64; 2], radius_nm: i64 },
    Text { at_nm: [i64; 2], text: String, align: TextAlign },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    terminal: String,
    at_nm: [i64; 2],
    #[serde(default)]
    appearance: Option<crate::schematic::PinAppearance>,
}

/// Parse and check one symbol file.
///
/// The file must be [`SYMBOL_FORMAT`], [`SYMBOL_FORMAT_V2`] or [`SYMBOL_FORMAT_V3`] with no unknown fields.
/// Version 2 validates explicit electrical roles and instance defaults; version 3 adds an explicit category binding. Terminals, 1 to 128 of them, have
/// unique IDs of 1 to 8 letters, digits, `+`, `-` or `_`. This build places symbols with one unit
/// that has exactly one pin per terminal. Pins sit on the 1.27 mm grid, at most 12.7 mm from the
/// center on each axis and apart from each other. Drawings have at most 256 strokes within
/// 100 mm of the center. Errors name the first problem found.
pub fn parse_symbol(text: &str) -> Result<SymbolDefinition, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("invalid symbol JSON: {error}"))?;
    parse_symbol_value(value)
}

/// [`parse_symbol`] for a symbol already read as JSON, such as a project's saved copy. The
/// definition's `library` is left empty for the caller to fill in.
fn parse_symbol_value(value: Value) -> Result<SymbolDefinition, String> {
    parse_symbol_mode(value, false)
}

/// Draft documents may temporarily have no pins; published symbols retain the strict minimum.
fn parse_symbol_mode(value: Value, draft: bool) -> Result<SymbolDefinition, String> {
    let format_version = match value["format"].as_str() {
        Some(SYMBOL_FORMAT) => 1,
        Some(SYMBOL_FORMAT_V2) => 2,
        Some(SYMBOL_FORMAT_V3) => 3,
        _ => return Err("unsupported symbol format".into()),
    };
    if format_version == 1 && (value.get("behavior").is_some() || value.get("defaults").is_some()) {
        return Err("electrical behavior and defaults require pcbcraft-symbol/2".into());
    }
    if format_version < 3 && value.get("category").is_some() {
        return Err("default category bindings require pcbcraft-symbol/3".into());
    }
    let file: SymbolFile = serde_json::from_value(value).map_err(|error| error.to_string())?;
    if format_version == 3 && file.category.is_none() {
        return Err("pcbcraft-symbol/3 requires an explicit category binding".into());
    }
    if let Some(category) = &file.category {
        category.validate()?;
    }
    if let Some(defaults) = &file.defaults {
        defaults.metadata.validate()?;
        crate::schematic::fields::validate_layouts(&defaults.field_layout, &defaults.metadata)?;
    }
    if let Some(Behavior::Power { terminal }) = &file.behavior
        && !file.terminals.iter().any(|pin| pin.id == *terminal)
    {
        return Err("power behavior must name an existing terminal".into());
    }
    check_id(&file.id)?;
    if file.revision == 0 {
        return Err("`revision` must be 1 or more".to_owned());
    }
    check_text("name", &file.name, 64)?;
    if !(1..=4).contains(&file.reference_prefix.len()) || !file.reference_prefix.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("`reference_prefix` must be 1 to 4 capital letters".to_owned());
    }
    check_value(&file.default_value).map_err(|error| error.replace("`value`", "`default_value`"))?;
    if file.terminals.len() > MAX_TERMINALS || (!draft && file.terminals.is_empty()) {
        return Err(format!("a symbol has 1 to {MAX_TERMINALS} terminals"));
    }
    let mut numbers = BTreeSet::new();
    for terminal in &file.terminals {
        check_pin_number(&terminal.id)?;
        if !numbers.insert(terminal.id.as_str()) {
            return Err(format!("terminal {} appears twice", terminal.id));
        }
        if let Some(name) = &terminal.name {
            check_text("terminal name", name, 32)?;
        }
    }
    let [unit] = <[UnitFile; 1]>::try_from(file.units).map_err(|_| "this build places symbols with exactly one unit".to_owned())?;
    check_id(&unit.id).map_err(|error| format!("unit {error}"))?;
    if let Some(pin) = unit.pins.iter().find(|pin| !numbers.contains(pin.terminal.as_str())) {
        return Err(format!("pin {} names no terminal", pin.terminal));
    }
    let mut positions = BTreeSet::new();
    let mut pins = Vec::with_capacity(file.terminals.len());
    for terminal in file.terminals {
        let mut placed = unit.pins.iter().filter(|pin| pin.terminal == terminal.id);
        let (Some(pin), None) = (placed.next(), placed.next()) else {
            return Err(format!("terminal {} needs exactly one pin", terminal.id));
        };
        let [x_nm, y_nm] = pin.at_nm;
        let reach = -MAX_PIN_OFFSET_NM..=MAX_PIN_OFFSET_NM;
        if !reach.contains(&x_nm) || !reach.contains(&y_nm) || x_nm % GRID_NM != 0 || y_nm % GRID_NM != 0 {
            return Err(format!("pin {} must sit on the 1.27 mm grid within 12.7 mm of the center", terminal.id));
        }
        if !positions.insert((x_nm, y_nm)) {
            return Err(format!("pin {} shares a position with another pin", terminal.id));
        }
        if let Some(appearance) = &pin.appearance {
            appearance.validate(Point { x_nm, y_nm })?;
        }
        pins.push(PinDefinition {
            appearance: pin.appearance.clone(),
            number: terminal.id,
            offset: Point { x_nm, y_nm },
            name: terminal.name,
            electrical_type: terminal.electrical_type,
        });
    }
    if unit.drawing.len() > MAX_SHAPES {
        return Err(format!("a drawing has at most {MAX_SHAPES} strokes"));
    }
    let drawing = unit.drawing.into_iter().map(shape).collect::<Result<Vec<_>, _>>()?;
    Ok(SymbolDefinition {
        format_version,
        behavior: file.behavior,
        defaults: file.defaults,
        category: file.category,
        provenance: None,
        library: String::new(),
        id: file.id,
        name: file.name,
        revision: file.revision,
        reference_prefix: file.reference_prefix,
        value: file.default_value,
        unit: unit.id,
        pins,
        drawing,
    })
}

/// Pin numbers are 1 to 8 ASCII letters, digits, `+`, `-` or `_`, such as `1`, `A3` or `VCC`.
fn check_pin_number(number: &str) -> Result<(), String> {
    match crate::schematic::PinNumber::new(number) {
        Some(_) => Ok(()),
        None => Err(format!("terminal ID {number:?} must be 1 to 8 letters, digits, +, - or _")),
    }
}

/// Read a project's saved copy of a symbol: the symbol file's fields plus the `library` it came
/// from. A copy identical to any retained starter revision shares that definition; any other copy
/// lives only as long as the projects and edits that use it. Fails on any symbol file rule or a
/// bad library ID.
pub(crate) fn read_saved_copy(value: Value) -> Result<Arc<SymbolDefinition>, String> {
    read_saved_mode(value, false)
}

/// Validate an editable draft with the same structural/geometry rules while permitting zero pins.
pub(crate) fn read_saved_draft(value: Value) -> Result<Arc<SymbolDefinition>, String> {
    read_saved_mode(value, true)
}

/// Retain identity/provenance and share known immutable definitions after mode-specific validation.
fn read_saved_mode(value: Value, draft: bool) -> Result<Arc<SymbolDefinition>, String> {
    let Value::Object(mut fields) = value else {
        return Err("a saved symbol definition must be a JSON object".to_owned());
    };
    let library = match fields.remove("library") {
        Some(Value::String(library)) => library,
        _ => return Err("a saved symbol definition needs its `library` ID".to_owned()),
    };
    check_id(&library).map_err(|error| format!("library {error}"))?;
    let provenance = fields.remove("provenance").map(serde_json::from_value::<sources::Provenance>).transpose().map_err(|e| e.to_string())?;
    if let Some(provenance) = &provenance {
        provenance.validate()?;
    }
    let mut definition = parse_symbol_mode(Value::Object(fields), draft)?;
    definition.provenance = provenance;
    definition.library = library;
    let starter = starter();
    Ok(starter.symbols.iter().chain(&starter.revisions).find(|known| ***known == definition).cloned().unwrap_or_else(|| Arc::new(definition)))
}

/// Write a symbol's saved form into a project file, indented as one project record: the header
/// fields on the first line, then one line per terminal, drawing stroke and pin, so editing one
/// stroke or pin changes one line. The JSON is the same as [`saved_copy`] serializes, and the
/// output depends only on the definition, so an unchanged project saves without a diff.
pub(crate) fn write_saved_copy(bytes: &mut Vec<u8>, definition: &SymbolDefinition) -> Result<(), String> {
    let SavedSymbol {
        library,
        format,
        id,
        revision,
        name,
        reference_prefix,
        default_value,
        provenance,
        behavior,
        defaults,
        category,
        terminals,
        units,
    } = saved_copy(definition);
    let header =
        compact(&SavedHeader { library, format, id, revision, name, reference_prefix, default_value, provenance, behavior, defaults, category })?;
    // The header object stays open so the lists below continue it.
    let header = header.strip_suffix('}').unwrap_or(&header);
    let list = |bytes: &mut Vec<u8>, indent: &str, items: Vec<String>| {
        for (index, item) in items.iter().enumerate() {
            bytes.extend_from_slice(indent.as_bytes());
            bytes.extend_from_slice(item.as_bytes());
            bytes.extend_from_slice(if index + 1 < items.len() { b",\n" } else { b"\n" });
        }
    };
    bytes.extend_from_slice(b"      ");
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(b",\n        \"terminals\":[\n");
    list(bytes, "          ", terminals.iter().map(compact).collect::<Result<_, _>>()?);
    bytes.extend_from_slice(b"        ],\n        \"units\":[\n");
    for (index, unit) in units.iter().enumerate() {
        bytes.extend_from_slice(format!("          {{\"id\":{},\n            \"drawing\":[\n", compact(&unit.id)?).as_bytes());
        list(bytes, "              ", unit.drawing.iter().map(compact).collect::<Result<_, _>>()?);
        bytes.extend_from_slice(b"            ],\n            \"pins\":[\n");
        list(bytes, "              ", unit.pins.iter().map(compact).collect::<Result<_, _>>()?);
        bytes.extend_from_slice(if index + 1 < units.len() { b"            ]},\n" } else { b"            ]}\n" });
    }
    bytes.extend_from_slice(b"        ]}");
    Ok(())
}

/// Serialize one saved part of a symbol as compact JSON, with its fields in declaration order.
fn compact(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

#[derive(Serialize)]
struct SavedHeader<'a> {
    library: &'a str,
    format: &'static str,
    id: &'a str,
    revision: u32,
    name: &'a str,
    reference_prefix: &'a str,
    default_value: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<&'a sources::Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    behavior: Option<&'a Behavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    defaults: Option<&'a Defaults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a parts::CategoryBinding>,
}

/// The saved form of a symbol in a project: its library ID, then the same fields as its
/// symbol file, so [`read_saved_copy`] reads it back to an equal definition.
pub(crate) fn saved_copy(definition: &SymbolDefinition) -> SavedSymbol<'_> {
    let point = |point: &Point| [point.x_nm, point.y_nm];
    SavedSymbol {
        behavior: definition.behavior.as_ref(),
        defaults: definition.defaults.as_ref(),
        category: definition.category.as_ref(),
        provenance: definition.provenance.as_ref(),
        library: &definition.library,
        format: definition.format(),
        id: &definition.id,
        revision: definition.revision,
        name: &definition.name,
        reference_prefix: &definition.reference_prefix,
        default_value: &definition.value,
        terminals: definition
            .pins
            .iter()
            .map(|pin| SavedTerminal { id: &pin.number, name: pin.name.as_deref(), electrical_type: pin.electrical_type })
            .collect(),
        units: [SavedUnit {
            id: &definition.unit,
            drawing: definition
                .drawing
                .iter()
                .map(|shape| match shape {
                    Shape::Graphic(graphic) => SavedShape::Graphic { graphic: graphic.clone() },
                    Shape::Line(from, to) => SavedShape::Line { from_nm: point(from), to_nm: point(to) },
                    Shape::Polyline(points) => SavedShape::Polyline { points_nm: points.iter().map(point).collect() },
                    Shape::Rect(min, max) => SavedShape::Rect { min_nm: point(min), max_nm: point(max) },
                    Shape::Circle(center, radius) => SavedShape::Circle { center_nm: point(center), radius_nm: *radius },
                    Shape::Text(at, text, align) => SavedShape::Text { at_nm: point(at), text, align: *align },
                })
                .collect(),
            pins: definition
                .pins
                .iter()
                .map(|pin| SavedPin { terminal: &pin.number, at_nm: point(&pin.offset), appearance: pin.appearance.as_ref() })
                .collect(),
        }],
    }
}

#[derive(Serialize)]
pub(crate) struct SavedSymbol<'a> {
    library: &'a str,
    format: &'static str,
    id: &'a str,
    revision: u32,
    name: &'a str,
    reference_prefix: &'a str,
    default_value: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<&'a sources::Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    behavior: Option<&'a Behavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    defaults: Option<&'a Defaults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a parts::CategoryBinding>,
    terminals: Vec<SavedTerminal<'a>>,
    units: [SavedUnit<'a>; 1],
}

#[derive(Serialize)]
struct SavedTerminal<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    electrical_type: ElectricalType,
}

#[derive(Serialize)]
struct SavedUnit<'a> {
    id: &'a str,
    drawing: Vec<SavedShape<'a>>,
    pins: Vec<SavedPin<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SavedShape<'a> {
    Graphic { graphic: crate::library_graphics::Graphic },
    Line { from_nm: [i64; 2], to_nm: [i64; 2] },
    Polyline { points_nm: Vec<[i64; 2]> },
    Rect { min_nm: [i64; 2], max_nm: [i64; 2] },
    Circle { center_nm: [i64; 2], radius_nm: i64 },
    Text { at_nm: [i64; 2], text: &'a str, align: TextAlign },
}

#[derive(Serialize)]
struct SavedPin<'a> {
    terminal: &'a str,
    at_nm: [i64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    appearance: Option<&'a crate::schematic::PinAppearance>,
}

/// Parse JSON and check its `format` before the strict read, so an older or newer file reports
/// its version instead of a field error.
fn versioned(value: Value, format: &str, what: &str) -> Result<Value, String> {
    match value.get("format").and_then(Value::as_str) {
        Some(found) if found == format => Ok(value),
        Some(found) => Err(format!("unsupported {what} format {found}")),
        None => Err(format!("missing {what} format")),
    }
}

/// Convert one stroke, checking that it stays near the center and that polylines and circles
/// have something to draw. Coordinates outside the inclusive drawing bounds, including integer
/// extremes, return an error without overflowing.
fn shape(file: ShapeFile) -> Result<Shape, String> {
    let point = |[x_nm, y_nm]: [i64; 2]| -> Result<Point, String> {
        if !(-MAX_DRAWING_NM..=MAX_DRAWING_NM).contains(&x_nm) || !(-MAX_DRAWING_NM..=MAX_DRAWING_NM).contains(&y_nm) {
            return Err("a drawing must stay within 100 mm of the symbol's center".to_owned());
        }
        Ok(Point { x_nm, y_nm })
    };
    Ok(match file {
        ShapeFile::Graphic { graphic } => {
            graphic.validate(MAX_DRAWING_NM)?;
            Shape::Graphic(graphic)
        }
        ShapeFile::Line { from_nm, to_nm } => Shape::Line(point(from_nm)?, point(to_nm)?),
        ShapeFile::Polyline { points_nm } => {
            if !(2..=MAX_POINTS).contains(&points_nm.len()) {
                return Err(format!("a polyline has 2 to {MAX_POINTS} points"));
            }
            Shape::Polyline(points_nm.into_iter().map(point).collect::<Result<_, _>>()?)
        }
        ShapeFile::Rect { min_nm, max_nm } => Shape::Rect(point(min_nm)?, point(max_nm)?),
        ShapeFile::Circle { center_nm, radius_nm } => {
            if !(1..=MAX_DRAWING_NM).contains(&radius_nm) {
                return Err("a circle's radius must be positive and at most 100 mm".to_owned());
            }
            Shape::Circle(point(center_nm)?, radius_nm)
        }
        ShapeFile::Text { at_nm, text, align } => {
            check_text("text", &text, MAX_LABEL)?;
            Shape::Text(point(at_nm)?, text, align)
        }
    })
}

/// IDs are 1 to 64 lowercase letters, digits and inner hyphens, so they read well in project
/// files and command parameters.
fn check_id(id: &str) -> Result<(), String> {
    let valid = (1..=64).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if valid { Ok(()) } else { Err(format!("ID {id:?} must be 1 to 64 lowercase letters, digits and inner hyphens")) }
}

/// Non-empty, trimmed, at most `limit` characters and free of control characters.
fn check_text(field: &str, text: &str, limit: usize) -> Result<(), String> {
    if text.is_empty() || text.trim() != text || text.chars().count() > limit || text.chars().any(char::is_control) {
        return Err(format!("`{field}` must be 1 to {limit} characters on one line"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every starter file loads, in file-name order, with the manifest's identity and license.
    #[test]
    fn the_starter_library_loads_every_file() {
        let library = starter();
        assert_eq!(library.problems(), [] as [String; 0]);
        assert_eq!((library.id.as_str(), library.license.as_str()), ("starter", "MIT OR Apache-2.0"));
        let ids: Vec<_> = library.symbols().iter().map(|symbol| symbol.id.as_str()).collect();
        assert_eq!(ids, ["capacitor", "connector", "header-4", "led", "npn", "resistor"]);
        let led = library.symbol("led").unwrap();
        let names: Vec<_> = led.pins().iter().map(|pin| pin.name.as_deref()).collect();
        assert_eq!(names, [Some("Anode"), Some("Cathode")]);
        assert_eq!(led.reference_prefix(), "D");
    }

    /// New placements use the latest revision; older pinned revisions retain their original pins and values.
    #[test]
    fn compact_revisions_preserve_old_projects_and_choose_newest_in_any_order() {
        for symbol in starter().symbols().iter().filter(|symbol| ["capacitor", "connector", "led", "resistor"].contains(&symbol.id.as_str())) {
            assert_eq!(symbol.revision(), if ["resistor", "capacitor"].contains(&symbol.id.as_str()) { 3 } else { 2 });
            let old = starter().symbol_revision(&symbol.id, 1).unwrap();
            for (new, old) in symbol.pins().iter().zip(old.pins()) {
                assert_eq!(old.offset.x_nm.abs(), PIN_OFFSET_NM);
                assert!(new.offset.x_nm.abs() < old.offset.x_nm.abs());
                assert_eq!(new.offset.x_nm % GRID_NM, 0);
            }
        }
        let files = starter_files::SYMBOLS.iter().rev().copied().collect::<Vec<_>>();
        let reversed = Library::load(starter_files::MANIFEST, &files);
        assert!(reversed.problems().is_empty());
        assert_eq!(reversed.symbol("resistor").unwrap().revision(), 3);
        assert_eq!(reversed.symbol_revision("resistor", 1).unwrap().pins()[1].offset.x_nm, PIN_OFFSET_NM);
        assert!(reversed.symbol_revision("resistor", 4).is_none());
        let old = crate::persistence::decode(include_bytes!("../../app/tests/fixtures/two-resistors.pc")).unwrap();
        assert_eq!(old.symbols()[0].pins()[1].x_nm, PIN_OFFSET_NM);
    }

    fn resistor() -> Value {
        serde_json::from_str(starter_files::SYMBOLS.iter().find(|(file, _)| *file == "resistor.json").unwrap().1).unwrap()
    }

    fn error(edit: impl FnOnce(&mut Value)) -> String {
        let mut file = resistor();
        edit(&mut file);
        parse_symbol(&file.to_string()).unwrap_err()
    }

    /// Each rule the format promises refuses a file that breaks it, and says which rule.
    #[test]
    fn symbol_files_that_break_a_rule_are_refused() {
        assert!(parse_symbol(&resistor().to_string()).is_ok());
        assert!(error(|file| file["format"] = json!("pcbcraft-symbol/99")).contains("unsupported symbol format"));
        assert!(error(|file| file["footprint"] = json!("0805")).contains("unknown field"), "symbols carry no footprint");
        assert!(error(|file| file["id"] = json!("Resistor")).contains("lowercase"));
        assert!(error(|file| file["revision"] = json!(0)).contains("revision"));
        assert!(error(|file| file["reference_prefix"] = json!("r")).contains("capital"));
        assert!(error(|file| file["terminals"].as_array_mut().unwrap().truncate(1)).contains("pin 2 names no terminal"));
        assert!(error(|file| file["terminals"] = json!([])).contains("1 to 128 terminals"));
        assert!(error(|file| file["terminals"][1]["id"] = json!("1")).contains("terminal 1 appears twice"));
        assert!(error(|file| file["terminals"][1]["id"] = json!("pin two")).contains("1 to 8 letters"));
        assert!(error(|file| file["terminals"][1]["electrical_type"] = json!("magic")).contains("unknown variant"));
        assert!(error(|file| file["units"] = json!([])).contains("one unit"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([6_350_001, 0])).contains("grid"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([13_970_000, 0])).contains("within 12.7 mm"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([i64::MIN, 0])).contains("within 12.7 mm"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([-5_080_000, 0])).contains("shares a position"));
        assert!(error(|file| file["units"][0]["pins"][1]["terminal"] = json!("1")).contains("exactly one pin"));
        assert!(error(|file| file["units"][0]["drawing"][0]["to_nm"] = json!([100_000_001, 0])).contains("100 mm"));
        assert!(error(|file| file["units"][0]["drawing"][0]["to_nm"] = json!([i64::MIN, 0])).contains("100 mm"));
        assert!(error(|file| file["units"][0]["drawing"][0]["to_nm"] = json!([0, i64::MIN])).contains("100 mm"));
        assert!(error(|file| file["units"][0]["drawing"][0] = json!({"type": "circle", "center_nm": [0, 0], "radius_nm": 0})).contains("radius"));
        assert!(error(|file| file["units"][0]["drawing"][0] = json!({"type": "arc"})).contains("unknown variant"));
        let label = |text: &str| json!({"type": "text", "at_nm": [0, 0], "text": text, "align": "middle"});
        assert!(error(|file| file["units"][0]["drawing"][0] = label("")).contains("`text`"));
        assert!(error(|file| file["units"][0]["drawing"][0] = label("seventeen letters")).contains("`text`"));
    }

    /// Multi-pin symbols keep their terminals in file order with names, numbers and positions,
    /// and text labels load as upright strokes.
    #[test]
    fn multi_pin_symbols_load_in_terminal_order() {
        let header = starter().symbol("header-4").unwrap();
        let pins: Vec<_> = header.pins().iter().map(|pin| (pin.number.as_str(), pin.offset.y_nm, pin.electrical_type)).collect();
        let row = |number, y_nm| (number, y_nm, ElectricalType::Passive);
        assert_eq!(pins, [row("1", -3_810_000), row("2", -1_270_000), row("3", 1_270_000), row("4", 3_810_000)]);
        assert!(header.drawing.iter().any(|shape| matches!(shape, Shape::Text(_, text, TextAlign::Middle) if text == "4")));
        assert!(!header.fits_two_pad_footprint());
        assert!(starter().symbol("resistor").unwrap().fits_two_pad_footprint());
    }

    /// Every starter symbol saves as a copy that reads back to the same shared starter
    /// definition. A copy that differs is its own definition, freed once nothing uses it.
    #[test]
    fn saved_copies_read_back_to_the_same_definition() {
        for symbol in starter().symbols() {
            let copy = read_saved_copy(serde_json::to_value(saved_copy(symbol)).unwrap()).unwrap();
            assert!(Arc::ptr_eq(&copy, symbol), "{}", symbol.id);
        }
        let mut changed = serde_json::to_value(saved_copy(starter().symbol("led").unwrap())).unwrap();
        changed["name"] = json!("LED, as saved");
        let copy = read_saved_copy(changed).unwrap();
        assert_eq!((copy.library.as_str(), copy.name.as_str()), ("starter", "LED, as saved"));
        let weak = Arc::downgrade(&copy);
        drop(copy);
        assert!(weak.upgrade().is_none(), "nothing keeps a copy alive once it is dropped");
    }

    /// Extreme coordinates report a file error without preventing later valid files from loading.
    #[test]
    fn a_drawing_overflow_leaves_the_rest_of_the_library() {
        let mut bad = resistor();
        bad["units"][0]["drawing"][0]["to_nm"] = json!([i64::MIN, 0]);
        let good = resistor().to_string();
        let library = Library::load(starter_files::MANIFEST, &[("bad.json", &bad.to_string()), ("good.json", &good)]);
        assert_eq!(library.symbols().len(), 1);
        assert!(library.symbol("resistor").is_some());
        assert_eq!(library.problems(), ["bad.json: a drawing must stay within 100 mm of the symbol's center"]);
    }

    /// A bad or repeated file is left out with a message, and the rest of the library still loads.
    #[test]
    fn a_bad_file_leaves_the_rest_of_the_library() {
        let good = resistor().to_string();
        let library = Library::load(starter_files::MANIFEST, &[("a.json", &good), ("b.json", "{"), ("c.json", &good)]);
        assert_eq!(library.symbols().len(), 1);
        assert_eq!(library.problems().len(), 2);
        assert!(library.problems()[0].starts_with("b.json: invalid symbol JSON"));
        assert!(library.problems()[1].contains("already has the ID resistor"));
        let broken = Library::load("{\"format\": \"pcbcraft-library/9\"}", &[("a.json", &good)]);
        assert!(broken.problems()[0].contains("unsupported library format"));
        assert_eq!(broken.symbols().len(), 1);
    }
}
