//! Versioned native project data. The live session owns history and derived indexes.

use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    MAX_NAME, MAX_OBJECT_ID, MAX_REFERENCE_NUMBER, Project,
    board::{Board, Footprint},
    schematic::{MAX_CENTER_NM, MAX_SYMBOLS, Point, SymbolPlacement, Terminal, Wire, builtin_symbol, wiring::MAX_WIRES},
};

mod archive;
pub use archive::ArchiveEncoder;

/// Maximum number of bytes in a supported project ZIP.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// The encoding of a native .pc file, independent of its filename.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Json,
    Zip,
}

impl Encoding {
    /// Detect a ZIP by its header. Both encodings still require format validation.
    pub fn detect(bytes: &[u8]) -> Self {
        if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08") { Self::Zip } else { Self::Json }
    }

    /// Return the input size bound before allocating or decoding the file.
    pub fn input_limit(self) -> usize {
        match self {
            Self::Json => MAX_PROJECT_BYTES,
            Self::Zip => MAX_ARCHIVE_BYTES,
        }
    }
}

/// Read and validate a native project using the encoding detected from its content.
pub fn decode_file(bytes: &[u8]) -> Result<(Arc<Project>, Encoding), String> {
    let encoding = Encoding::detect(bytes);
    if bytes.len() > encoding.input_limit() {
        return Err("project exceeds the native file size limit".to_owned());
    }
    let project = match encoding {
        Encoding::Json => decode(bytes)?,
        Encoding::Zip => decode(&archive::unpack(bytes)?)?,
    };
    Ok((project, encoding))
}

/// Encode a project in the requested native file encoding.
pub fn encode_file(project: Arc<Project>, encoding: Encoding) -> Result<Vec<u8>, String> {
    let bytes = encode(project)?;
    match encoding {
        Encoding::Json => Ok(bytes),
        Encoding::Zip => {
            let mut encoder = ArchiveEncoder::new(bytes)?;
            loop {
                if let Some(bytes) = encoder.step()? {
                    return Ok(bytes);
                }
            }
        }
    }
}

/// The project format this build writes. Format 2 adds symbol rotation and edited
/// references and values.
const FORMAT: &str = "pcbcraft-project/2";
/// The first project format, which opens through [`migrate_from_1`].
const FORMAT_1: &str = "pcbcraft-project/1";
const BUILTIN_REVISION: u32 = 1;
/// Maximum number of bytes in a plain project document or ZIP project payload.
pub const MAX_PROJECT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    id: Uuid,
    name: String,
    sheet: Sheet,
    board: Option<BoardRecords>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sheet {
    id: Uuid,
    symbols: Vec<Placement<'static>>,
    wires: Vec<Connection<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardRecords {
    footprints: Vec<Footprint>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Placement<'a> {
    id: u64,
    definition: std::borrow::Cow<'a, str>,
    definition_revision: u32,
    reference_number: usize,
    x_nm: i64,
    y_nm: i64,
    /// Degrees counterclockwise: 0, 90, 180 or 270. Required from format 2; absent in format 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<u16>,
    /// Edited reference and value, written only when a person changed them (format 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<std::borrow::Cow<'a, str>>,
}

/// Bring a format 1 symbol record up to format 2: unrotated, with its automatic reference and
/// its definition's value. Format 1 had none of format 2's fields, so finding one is an error.
fn migrate_from_1(record: &mut Placement<'_>) -> Result<(), String> {
    if record.rotation.is_some() || record.reference.is_some() || record.value.is_some() {
        return Err(format!("{FORMAT_1} has no symbol rotation, reference or value fields"));
    }
    record.rotation = Some(0);
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Connection<'a> {
    id: u64,
    from: Endpoint<'a>,
    to: Endpoint<'a>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Endpoint<'a> {
    symbol: u64,
    pin: std::borrow::Cow<'a, str>,
}

impl From<Terminal> for Endpoint<'static> {
    fn from(terminal: Terminal) -> Self {
        Self { symbol: terminal.symbol, pin: terminal.number().into() }
    }
}

/// Encode a complete snapshot as deterministic JSON with one line per editable record.
pub fn encode(project: Arc<Project>) -> Result<Vec<u8>, String> {
    let mut encoder = Encoder::new(project)?;
    loop {
        if let Some(bytes) = encoder.step(256)? {
            return Ok(bytes);
        }
    }
}

/// A bounded serializer that shares its source snapshot and can yield between batches.
pub struct Encoder {
    project: Arc<Project>,
    bytes: Vec<u8>,
    symbols: usize,
    wires: usize,
    writing_wires: bool,
    footprints: usize,
    writing_board: bool,
}

impl Encoder {
    /// Start a native document without walking the schematic.
    pub fn new(project: Arc<Project>) -> Result<Self, String> {
        let name = serde_json::to_string(&project.name).map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        write!(
            bytes,
            "{{\n  \"format\": \"{FORMAT}\",\n  \"id\": \"{}\",\n  \"name\": {name},\n  \"sheet\": {{\n    \"id\": \"{}\",\n    \"symbols\": [\n",
            project.id, project.sheet_id
        )
        .map_err(|error| error.to_string())?;
        Ok(Self { project, bytes, symbols: 0, wires: 0, writing_wires: false, footprints: 0, writing_board: false })
    }

    /// Write up to the given number of records. Callers stop on the first completed document.
    pub fn step(&mut self, records: usize) -> Result<Option<Vec<u8>>, String> {
        for _ in 0..records {
            if let Some(symbol) = self.project.symbols.get(self.symbols) {
                let labels = self.project.labels.get(&symbol.ordinal);
                let record = Placement {
                    id: symbol.ordinal,
                    definition: symbol.definition.id.into(),
                    definition_revision: BUILTIN_REVISION,
                    reference_number: symbol.reference_number,
                    x_nm: symbol.position.x_nm,
                    y_nm: symbol.position.y_nm,
                    rotation: Some(u16::from(symbol.rotation) * 90),
                    reference: labels.and_then(|labels| labels.reference.as_deref()).map(Into::into),
                    value: labels.and_then(|labels| labels.value.as_deref()).map(Into::into),
                };
                write_record(&mut self.bytes, &record, self.symbols + 1 < self.project.symbols.len())?;
                self.symbols += 1;
                continue;
            }
            if !self.writing_wires {
                self.bytes.extend_from_slice(b"    ],\n    \"wires\": [\n");
                self.writing_wires = true;
            }
            if let Some(wire) = self.project.wires.get(self.wires) {
                let [from, to] = wire.terminals();
                let record = Connection { id: wire.ordinal, from: from.into(), to: to.into() };
                write_record(&mut self.bytes, &record, self.wires + 1 < self.project.wires.len())?;
                self.wires += 1;
                continue;
            }
            let footprints = self.project.board.footprints();
            if !self.writing_board {
                self.bytes.extend_from_slice(b"    ]\n  }");
                if !footprints.is_empty() {
                    self.bytes.extend_from_slice(b",\n  \"board\": {\n    \"footprints\": [\n");
                }
                self.writing_board = true;
            }
            if let Some(footprint) = footprints.get(self.footprints) {
                write_record(&mut self.bytes, footprint, self.footprints + 1 < footprints.len())?;
                self.footprints += 1;
                continue;
            }
            if !footprints.is_empty() {
                self.bytes.extend_from_slice(b"    ]\n  }");
            }
            self.bytes.extend_from_slice(b"\n}\n");
            return Ok(Some(std::mem::take(&mut self.bytes)));
        }
        Ok(None)
    }
}

/// Write a single independently diffable JSON record.
fn write_record(bytes: &mut Vec<u8>, record: &impl Serialize, more: bool) -> Result<(), String> {
    bytes.extend_from_slice(b"      ");
    serde_json::to_writer(&mut *bytes, record).map_err(|error| error.to_string())?;
    if more {
        bytes.extend_from_slice(b",\n\n");
    } else {
        bytes.push(b'\n');
    }
    Ok(())
}

/// Read at most one byte past the native file limit, even if the source keeps growing.
pub fn read_bytes(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut input).take(4).read_to_end(&mut bytes)?;
    let limit = Encoding::detect(&bytes).input_limit();
    input.take((limit + 1 - bytes.len()) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "project exceeds the native file size limit"));
    }
    Ok(bytes)
}

/// Read bounded project data and validate all identities and connections before adoption.
pub fn decode(bytes: &[u8]) -> Result<Arc<Project>, String> {
    if bytes.len() > MAX_PROJECT_BYTES {
        return Err("project exceeds the native file size limit".to_owned());
    }
    let Document { format, id, name, sheet, board } = serde_json::from_slice(bytes).map_err(|error| format!("invalid project JSON: {error}"))?;
    let upgraded_from = match format.as_str() {
        FORMAT => None,
        FORMAT_1 => Some(1),
        _ => return Err(format!("unsupported project format {format}")),
    };
    if id.is_nil() || sheet.id.is_nil() || id == sheet.id {
        return Err("invalid project or sheet identity".to_owned());
    }
    if name.is_empty() || name.trim() != name || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("invalid project name".to_owned());
    }
    if sheet.symbols.len() > MAX_SYMBOLS || sheet.wires.len() > MAX_WIRES {
        return Err("project exceeds the symbol or wire limit".to_owned());
    }

    let mut ids = BTreeSet::new();
    let mut references = BTreeSet::new();
    let mut symbols = Vec::with_capacity(sheet.symbols.len());
    let mut labels = std::collections::BTreeMap::new();
    let mut shown = BTreeSet::new();
    for mut record in sheet.symbols {
        if upgraded_from == Some(1) {
            migrate_from_1(&mut record)?;
        }
        let definition = builtin_symbol(&record.definition)
            .filter(|_| record.definition_revision == BUILTIN_REVISION)
            .ok_or_else(|| format!("unavailable definition {} at revision {}", record.definition, record.definition_revision))?;
        validate_id(&mut ids, record.id)?;
        if !(1..=MAX_REFERENCE_NUMBER).contains(&record.reference_number)
            || !references.insert((definition.reference_prefix, record.reference_number))
        {
            return Err("invalid or duplicate symbol reference".to_owned());
        }
        if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.y_nm) {
            return Err("symbol position is outside the schematic bounds".to_owned());
        }
        let rotation = match record.rotation {
            Some(degrees @ (0 | 90 | 180 | 270)) => (degrees / 90) as u8,
            Some(_) => return Err("symbol rotation must be 0, 90, 180 or 270 degrees".to_owned()),
            None => return Err("symbol is missing its rotation".to_owned()),
        };
        let symbol = SymbolPlacement {
            definition,
            ordinal: record.id,
            reference_number: record.reference_number,
            position: Point { x_nm: record.x_nm, y_nm: record.y_nm },
            rotation,
        };
        // Text equal to the automatic reference or default value is the same as no edit.
        let reference = record.reference.map(String::from).filter(|text| *text != symbol.reference());
        let value = record.value.map(String::from).filter(|text| text != symbol.value());
        if let Some(text) = &reference {
            crate::schematic::check_reference(text)?;
        }
        if let Some(text) = &value {
            crate::schematic::check_value(text)?;
        }
        if !shown.insert(reference.clone().unwrap_or_else(|| symbol.reference())) {
            return Err("two symbols show the same reference".to_owned());
        }
        if reference.is_some() || value.is_some() {
            labels.insert(record.id, crate::schematic::Labels { reference, value });
        }
        symbols.push(symbol);
    }
    symbols.sort_unstable_by_key(|symbol| symbol.ordinal);
    let resolve = |endpoint: Endpoint<'_>| {
        symbols
            .binary_search_by_key(&endpoint.symbol, |symbol| symbol.ordinal)
            .ok()
            .and_then(|_| Terminal::from_number(endpoint.symbol, &endpoint.pin))
            .ok_or_else(|| format!("wire points to missing pin {} on symbol {}", endpoint.pin, endpoint.symbol))
    };

    let mut pairs = BTreeSet::new();
    let mut wires = Vec::with_capacity(sheet.wires.len());
    for record in sheet.wires {
        validate_id(&mut ids, record.id)?;
        let mut terminals = [resolve(record.from)?, resolve(record.to)?];
        terminals.sort_unstable();
        if terminals[0] == terminals[1] || !pairs.insert(terminals) {
            return Err("wire has identical endpoints or duplicates another wire".to_owned());
        }
        wires.push(Wire { ordinal: record.id, terminals });
    }
    wires.sort_unstable_by_key(|wire| wire.ordinal);

    let mut layout = Board::default();
    if let Some(mut board) = board {
        if board.footprints.len() > symbols.len() {
            return Err("board has more footprints than schematic components".to_owned());
        }
        board.footprints.sort_unstable_by_key(|footprint| footprint.id);
        for footprint in board.footprints {
            validate_id(&mut ids, footprint.id)?;
            if symbols.binary_search_by_key(&footprint.symbol, |symbol| symbol.ordinal).is_err() {
                return Err("footprint points to a missing component".to_owned());
            }
            layout.append(footprint).map_err(str::to_owned)?;
        }
    }

    Ok(Arc::new(Project {
        id,
        sheet_id: sheet.id,
        name,
        symbols: Arc::new(symbols),
        labels: Arc::new(labels),
        wires: Arc::new(wires),
        connectivity: Arc::default(),
        board: Arc::new(layout),
        upgraded_from,
    }))
}

/// Require unique positive object IDs with room for subsequent runtime revisions.
fn validate_id(ids: &mut BTreeSet<u64>, id: u64) -> Result<(), String> {
    if !(1..=MAX_OBJECT_ID).contains(&id) || !ids.insert(id) {
        return Err(format!("invalid or duplicate object ID {id}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{CONNECT_PINS, MOVE_SYMBOL, PLACE_SYMBOL, Session};

    fn fixture() -> Session {
        let mut session = Session::default();
        session.apply("project.new", &json!({"name": "LED indicator"})).unwrap();
        for (definition, x_nm) in [("resistor", 0), ("led", 25_400_000)] {
            session
                .apply(
                    PLACE_SYMBOL,
                    &json!({
                        "definition": definition, "x_nm": x_nm, "y_nm": 0, "expected_revision": session.revision()
                    }),
                )
                .unwrap();
        }
        session
            .apply(
                CONNECT_PINS,
                &json!({
                    "from": {"symbol": "symbol-2", "terminal": "2"},
                    "to": {"symbol": "symbol-3", "terminal": "1"},
                    "expected_revision": session.revision()
                }),
            )
            .unwrap();
        session
    }

    /// Every built-in definition places through the shared command, numbers its references by
    /// prefix and survives a save and reopen with its kind, reference and value.
    #[test]
    fn every_builtin_symbol_places_and_round_trips() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"name": "All parts"})).unwrap();
        for (index, definition) in crate::schematic::BUILTIN_SYMBOLS.iter().enumerate() {
            let x_nm = index as i64 * 25_400_000;
            session
                .apply(PLACE_SYMBOL, &json!({"definition": definition.id, "x_nm": x_nm, "y_nm": 0, "expected_revision": session.revision()}))
                .unwrap();
        }
        let symbols = session.inspect()["project"]["schematic"]["symbols"].clone();
        let summary: Vec<_> =
            symbols.as_array().unwrap().iter().map(|symbol| (symbol["kind"].clone(), symbol["reference"].clone(), symbol["value"].clone())).collect();
        assert_eq!(
            summary,
            [("resistor", "R1", "1k"), ("led", "D1", "LED"), ("capacitor", "C1", "100nF"), ("connector", "J1", "Input")]
                .map(|(kind, reference, value)| (json!(kind), json!(reference), json!(value)))
        );
        let loaded = decode(&encode(session.snapshot().unwrap()).unwrap()).unwrap();
        let mut reopened = Session::default();
        reopened.open_snapshot(loaded);
        assert_eq!(reopened.inspect()["project"]["schematic"]["symbols"], symbols);
    }

    const FORMAT_1_FIXTURE: &[u8] = include_bytes!("../../app/tests/fixtures/two-resistors-format-1.pc");
    const FORMAT_2_FIXTURE: &[u8] = include_bytes!("../../app/tests/fixtures/two-resistors.pc");

    /// A format 1 project opens through the explicit migration, says it was upgraded, and saves
    /// as exactly the format 2 fixture. A format 1 file can't carry format 2's fields.
    #[test]
    fn format_1_migrates_to_format_2() {
        let migrated = decode(FORMAT_1_FIXTURE).unwrap();
        assert_eq!(migrated.upgraded_from(), Some(1));
        assert!(migrated.symbols().iter().all(|symbol| symbol.turns() == 0));
        assert_eq!(encode(migrated).unwrap(), FORMAT_2_FIXTURE);
        let current = decode(FORMAT_2_FIXTURE).unwrap();
        assert_eq!(current.upgraded_from(), None);
        assert_eq!(encode(current).unwrap(), FORMAT_2_FIXTURE, "an unchanged format 2 project saves without a diff");

        let original = std::str::from_utf8(FORMAT_1_FIXTURE).unwrap();
        let forged = original.replacen("\"y_nm\":0}", "\"y_nm\":0,\"rotation\":90}", 1);
        assert_ne!(forged, original);
        assert!(decode(forged.as_bytes()).unwrap_err().contains("has no symbol rotation"));
    }

    /// Rotation and edited text survive a save. Unedited symbols write no reference or value,
    /// and a file with a bad rotation or two symbols showing one reference is refused.
    #[test]
    fn rotation_and_edited_fields_round_trip() {
        let mut session = fixture();
        for _ in 0..2 {
            session.apply(crate::ROTATE_SYMBOL, &json!({"symbol": "symbol-2", "expected_revision": session.revision()})).unwrap();
        }
        let fields = json!({"symbol": "symbol-3", "reference": "D7", "value": "Red", "expected_revision": session.revision()});
        session.apply(crate::SET_SYMBOL_FIELDS, &fields).unwrap();
        let bytes = encode(session.snapshot().unwrap()).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(text.matches("\"reference\":").count(), 1, "only the edited symbol stores a reference");
        assert!(text.contains("\"rotation\":180"));
        let mut reopened = Session::default();
        reopened.open_snapshot(decode(&bytes).unwrap());
        assert_eq!(reopened.inspect()["project"], session.inspect()["project"]);
        let led = &reopened.inspect()["project"]["schematic"]["symbols"][1];
        assert_eq!((led["reference"].clone(), led["value"].clone()), (json!("D7"), json!("Red")));

        let bad_rotation = text.replace("\"rotation\":180", "\"rotation\":45");
        assert!(decode(bad_rotation.as_bytes()).unwrap_err().contains("rotation"));
        let duplicate = text.replace("\"reference\":\"D7\"", "\"reference\":\"R1\"");
        assert!(decode(duplicate.as_bytes()).unwrap_err().contains("same reference"));
    }

    /// Reopen retains IDs and wiring. Moving a symbol edits exactly one record and shares connectivity.
    #[test]
    fn round_trip_and_one_record_move() {
        let session = fixture();
        let before = session.snapshot().unwrap();
        let bytes = encode(before.clone()).unwrap();
        let loaded = decode(&bytes).unwrap();
        assert_eq!(encode(loaded.clone()).unwrap(), bytes);
        assert!(before.same_document(&loaded));
        let mut reopened = Session::default();
        reopened.open_snapshot(loaded.clone());
        assert_eq!(session.inspect()["project"], reopened.inspect()["project"]);
        reopened
            .apply(
                MOVE_SYMBOL,
                &json!({
                    "symbol": "symbol-2", "x_nm": 0, "y_nm": 1_270_000, "expected_revision": reopened.revision()
                }),
            )
            .unwrap();
        let moved = reopened.snapshot().unwrap();
        assert!(std::ptr::eq(loaded.connectivity(), moved.connectivity()));
        let after = encode(moved.clone()).unwrap();
        let old = std::str::from_utf8(&bytes).unwrap().lines().collect::<Vec<_>>();
        let new = std::str::from_utf8(&after).unwrap().lines().collect::<Vec<_>>();
        let changed = old.iter().zip(&new).filter(|(a, b)| a != b).collect::<Vec<_>>();
        assert_eq!(old.len(), new.len());
        assert_eq!(changed.len(), 1);
        assert!(changed[0].1.contains("\"y_nm\":1270000"));
        let pin = moved.symbols()[0].terminals()[1];
        assert_eq!(moved.connectivity().net_for(pin).unwrap().len(), 2);
        reopened.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert!(Arc::ptr_eq(&reopened.snapshot().unwrap(), &loaded));
    }

    /// A swapped pin mapping survives storage, movement, and reopening in both directions.
    #[test]
    fn footprint_links_round_trip_and_survive_schematic_moves() {
        let mut session = fixture();
        session
            .apply(
                crate::PLACE_FOOTPRINT,
                &json!({
                    "symbol": "symbol-3", "definition": "generic-through-hole-2",
                    "pin_1_pad": 2, "x_nm": 12_700_000, "y_nm": 0, "expected_revision": session.revision(),
                }),
            )
            .unwrap();
        let bytes = encode(session.snapshot().unwrap()).unwrap();
        let decoded = decode(&bytes).unwrap();
        let pin = decoded.symbols()[1].terminals()[0];
        let pad = decoded.pad_for_terminal(pin).unwrap();
        assert_eq!(pad.inspect(), json!({"footprint":"footprint-5", "pad":"2"}));
        assert_eq!(decoded.terminal_for_pad(pad), Some(pin));
        assert_eq!(decoded.pad_position(pad), Some(Point { x_nm: 13_970_000, y_nm: 0 }));
        assert_eq!(encode(decoded.clone()).unwrap(), bytes);
        session.open_snapshot(decoded);
        session
            .apply(
                MOVE_SYMBOL,
                &json!({
                    "symbol":"symbol-3", "x_nm": 2_540_000, "y_nm":0, "expected_revision": session.revision(),
                }),
            )
            .unwrap();
        let project = session.project().unwrap();
        assert_eq!(project.pad_for_terminal(pin), Some(pad));
        assert_eq!(project.terminal_for_pad(pad), Some(pin));
        assert!(project.pad_for_terminal(project.symbols()[0].terminals()[0]).is_none());
        let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        invalid["board"]["footprints"][0]["symbol"] = json!(99);
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    /// Reject malformed topology, future schema, unpinned definitions, and oversized input.
    #[test]
    fn rejects_bad_files_before_adoption() {
        let bytes = encode(fixture().snapshot().unwrap()).unwrap();
        let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (path, invalid) in [
            ("/format", json!("pcbcraft-project/999")),
            ("/sheet/symbols/0/definition_revision", json!(99)),
            ("/sheet/symbols/1/id", json!(2)),
            ("/sheet/wires/0/from/pin", json!("9")),
            ("/sheet/wires/0/to/symbol", json!(9999)),
        ] {
            let mut value = valid.clone();
            *value.pointer_mut(path).unwrap() = invalid;
            assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err(), "{path}");
        }
        assert!(decode(&vec![b' '; MAX_PROJECT_BYTES + 1]).is_err());
    }
}
