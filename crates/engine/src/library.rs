//! Schematic symbol libraries. A library is a folder with a `library.json` manifest and one
//! versioned JSON file per symbol in `symbols/`; see `docs/symbol-library.md` for the format.
//! Symbols describe only what the schematic draws and connects. Layout footprints are separate
//! definitions in their own libraries, linked to a symbol by assignment, so nothing here names
//! a footprint or a pad.
//!
//! The starter library in `libraries/starter` is embedded at build time, so the desktop and
//! browser builds offer the same parts without reading the file system.

use std::{collections::BTreeSet, sync::LazyLock};

use serde::Deserialize;
use serde_json::Value;

use crate::schematic::{ElectricalType, GRID_NM, PIN_OFFSET_NM, PinDefinition, Point, Shape, SymbolDefinition, check_value};

mod starter_files {
    include!(concat!(env!("OUT_DIR"), "/starter_library.rs"));
}

/// The symbol file format this build reads.
pub const SYMBOL_FORMAT: &str = "pcbcraft-symbol/1";
/// The library manifest format this build reads.
pub const LIBRARY_FORMAT: &str = "pcbcraft-library/1";

/// Most strokes in one symbol drawing.
const MAX_SHAPES: usize = 256;
/// Most points in one polyline.
const MAX_POINTS: usize = 64;
/// How far a drawing may reach from the symbol's center on either axis, in nanometers.
const MAX_DRAWING_NM: i64 = 2 * PIN_OFFSET_NM;

/// A loaded symbol library: its identity and license from the manifest, the symbols that passed
/// every check in file-name order, and a message for each file that didn't.
#[derive(Debug)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub license: String,
    symbols: Vec<SymbolDefinition>,
    problems: Vec<String>,
}

static STARTER: LazyLock<Library> = LazyLock::new(|| Library::load(starter_files::MANIFEST, starter_files::SYMBOLS));

/// The starter library shipped with PCBcraft.
pub fn starter() -> &'static Library {
    &STARTER
}

impl Library {
    /// Read a manifest and its symbol files, given as (file name, contents) pairs. A file that
    /// fails a check, or repeats an earlier symbol's ID, is left out and named in
    /// [`Self::problems`], so one bad file never hides the rest of the library.
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
        let mut symbols = Vec::with_capacity(files.len());
        for (file, text) in files {
            match parse_symbol(text) {
                Ok(symbol) if ids.insert(symbol.id.clone()) => symbols.push(symbol),
                Ok(symbol) => problems.push(format!("{file}: another symbol already has the ID {}", symbol.id)),
                Err(error) => problems.push(format!("{file}: {error}")),
            }
        }
        Self { id, name, license, symbols, problems }
    }

    /// Every symbol that loaded, in file-name order.
    pub fn symbols(&self) -> &[SymbolDefinition] {
        &self.symbols
    }

    /// The symbol with exactly this ID, if it loaded.
    pub fn symbol(&self, id: &str) -> Option<&SymbolDefinition> {
        self.symbols.iter().find(|symbol| symbol.id == id)
    }

    /// One message per manifest or symbol file that was left out, naming the file and the reason.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    #[allow(dead_code, reason = "checked before the rest of the manifest is read")]
    format: String,
    id: String,
    name: String,
    license: String,
}

/// Read a library manifest after checking its format, ID, name and license text.
fn parse_manifest(text: &str) -> Result<ManifestFile, String> {
    let manifest: ManifestFile = serde_json::from_value(versioned(text, LIBRARY_FORMAT, "library")?).map_err(|error| error.to_string())?;
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
    Line { from_nm: [i64; 2], to_nm: [i64; 2] },
    Polyline { points_nm: Vec<[i64; 2]> },
    Rect { min_nm: [i64; 2], max_nm: [i64; 2] },
    Circle { center_nm: [i64; 2], radius_nm: i64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    terminal: String,
    at_nm: [i64; 2],
}

/// Parse and check one symbol file.
///
/// The file must be [`SYMBOL_FORMAT`] with no unknown fields. This build places symbols with
/// exactly two terminals, `"1"` then `"2"`, and one unit with one pin per terminal. Pins sit on
/// the 1.27 mm grid, at most 6.35 mm from the center on each axis and apart from each other.
/// Drawings have at most 256 strokes within 12.7 mm of the center. Errors name the first
/// problem found.
pub fn parse_symbol(text: &str) -> Result<SymbolDefinition, String> {
    let file: SymbolFile = serde_json::from_value(versioned(text, SYMBOL_FORMAT, "symbol")?).map_err(|error| error.to_string())?;
    check_id(&file.id)?;
    if file.revision == 0 {
        return Err("`revision` must be 1 or more".to_owned());
    }
    check_text("name", &file.name, 64)?;
    if !(1..=4).contains(&file.reference_prefix.len()) || !file.reference_prefix.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("`reference_prefix` must be 1 to 4 capital letters".to_owned());
    }
    check_value(&file.default_value).map_err(|error| error.replace("`value`", "`default_value`"))?;
    let [first, second] =
        <[TerminalFile; 2]>::try_from(file.terminals).map_err(|_| "this build places symbols with exactly two terminals".to_owned())?;
    if first.id != "1" || second.id != "2" {
        return Err("the two terminals must be \"1\" then \"2\"".to_owned());
    }
    for name in [&first.name, &second.name].into_iter().flatten() {
        check_text("terminal name", name, 32)?;
    }
    let [unit] = <[UnitFile; 1]>::try_from(file.units).map_err(|_| "this build places symbols with exactly one unit".to_owned())?;
    check_id(&unit.id).map_err(|error| format!("unit {error}"))?;
    let offset = |terminal: &str| -> Result<Point, String> {
        let mut pins = unit.pins.iter().filter(|pin| pin.terminal == terminal);
        let (Some(pin), None) = (pins.next(), pins.next()) else {
            return Err(format!("terminal {terminal} needs exactly one pin"));
        };
        let [x_nm, y_nm] = pin.at_nm;
        if x_nm % GRID_NM != 0 || y_nm % GRID_NM != 0 || x_nm.abs() > PIN_OFFSET_NM || y_nm.abs() > PIN_OFFSET_NM {
            return Err(format!("pin {terminal} must sit on the 1.27 mm grid within 6.35 mm of the center"));
        }
        Ok(Point { x_nm, y_nm })
    };
    let offsets = [offset("1")?, offset("2")?];
    if unit.pins.len() != 2 {
        return Err("every pin must belong to terminal 1 or 2".to_owned());
    }
    if offsets[0] == offsets[1] {
        return Err("the two pins can't share a position".to_owned());
    }
    if unit.drawing.len() > MAX_SHAPES {
        return Err(format!("a drawing has at most {MAX_SHAPES} strokes"));
    }
    let drawing = unit.drawing.into_iter().map(shape).collect::<Result<Vec<_>, _>>()?;
    let pin = |terminal: TerminalFile, offset| PinDefinition { offset, name: terminal.name, electrical_type: terminal.electrical_type };
    Ok(SymbolDefinition {
        id: file.id,
        name: file.name,
        revision: file.revision,
        reference_prefix: file.reference_prefix,
        value: file.default_value,
        unit: unit.id,
        pins: [pin(first, offsets[0]), pin(second, offsets[1])],
        drawing,
    })
}

/// Parse JSON and check its `format` before the strict read, so an older or newer file reports
/// its version instead of a field error.
fn versioned(text: &str, format: &str, what: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("invalid {what} JSON: {error}"))?;
    match value.get("format").and_then(Value::as_str) {
        Some(found) if found == format => Ok(value),
        Some(found) => Err(format!("unsupported {what} format {found}")),
        None => Err(format!("missing {what} format")),
    }
}

/// Convert one stroke, checking that it stays near the center and that polylines and circles
/// have something to draw.
fn shape(file: ShapeFile) -> Result<Shape, String> {
    let point = |[x_nm, y_nm]: [i64; 2]| -> Result<Point, String> {
        if x_nm.abs() > MAX_DRAWING_NM || y_nm.abs() > MAX_DRAWING_NM {
            return Err("a drawing must stay within 12.7 mm of the symbol's center".to_owned());
        }
        Ok(Point { x_nm, y_nm })
    };
    Ok(match file {
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
                return Err("a circle's radius must be positive and at most 12.7 mm".to_owned());
            }
            Shape::Circle(point(center_nm)?, radius_nm)
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
        assert_eq!(ids, ["capacitor", "connector", "led", "resistor"]);
        let led = library.symbol("led").unwrap();
        assert_eq!(led.pins().each_ref().map(|pin| pin.name.as_deref()), [Some("Anode"), Some("Cathode")]);
        assert_eq!(led.reference_prefix(), "D");
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
        assert!(error(|file| file["format"] = json!("pcbcraft-symbol/2")).contains("unsupported symbol format"));
        assert!(error(|file| file["footprint"] = json!("0805")).contains("unknown field"), "symbols carry no footprint");
        assert!(error(|file| file["id"] = json!("Resistor")).contains("lowercase"));
        assert!(error(|file| file["revision"] = json!(0)).contains("revision"));
        assert!(error(|file| file["reference_prefix"] = json!("r")).contains("capital"));
        assert!(error(|file| file["terminals"].as_array_mut().unwrap().truncate(1)).contains("two terminals"));
        assert!(error(|file| file["terminals"][1]["electrical_type"] = json!("magic")).contains("unknown variant"));
        assert!(error(|file| file["units"] = json!([])).contains("one unit"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([6_350_001, 0])).contains("grid"));
        assert!(error(|file| file["units"][0]["pins"][1]["at_nm"] = json!([7_620_000, 0])).contains("within 6.35 mm"));
        assert!(error(|file| file["units"][0]["pins"][1]["terminal"] = json!("1")).contains("exactly one pin"));
        assert!(error(|file| file["units"][0]["drawing"][0]["to_nm"] = json!([12_700_001, 0])).contains("12.7 mm"));
        assert!(error(|file| file["units"][0]["drawing"][0] = json!({"type": "circle", "center_nm": [0, 0], "radius_nm": 0})).contains("radius"));
        assert!(error(|file| file["units"][0]["drawing"][0] = json!({"type": "arc"})).contains("unknown variant"));
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
