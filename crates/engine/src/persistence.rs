//! Versioned native project data. The live session owns history and derived indexes.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    MAX_NAME, MAX_OBJECT_ID, MAX_REFERENCE_NUMBER, Project,
    board::{Board, Footprint},
    schematic::{Component, MAX_CENTER_NM, MAX_SYMBOLS, Point, SymbolPlacement, Terminal, Wire, WireEndpoint, wiring::MAX_WIRES},
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

/// Format 2 separates component fields from unit geometry and links footprints to components.
const FORMAT: &str = "pcbcraft-project/2";
/// Format 3 adds explicit free endpoint identities and authored wire bends.
const FORMAT_3: &str = "pcbcraft-project/3";
/// The first project format, which opens through [`migrate_from_1`].
const FORMAT_1: &str = "pcbcraft-project/1";
/// Maximum number of bytes in a plain project document or ZIP project payload.
pub const MAX_PROJECT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    id: Uuid,
    name: String,
    components: Vec<ComponentRecord<'static>>,
    sheet: Sheet,
    board: Option<BoardRecords>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sheet {
    id: Uuid,
    symbols: Vec<Placement<'static>>,
    #[serde(default, deserialize_with = "present")]
    junctions: Option<Vec<Junction>>,
    wires: Vec<Connection<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardRecords {
    footprints: Vec<Footprint>,
}

/// Component data has no schematic coordinates or footprint geometry.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ComponentRecord<'a> {
    id: u64,
    definition: std::borrow::Cow<'a, str>,
    definition_revision: u32,
    reference_number: usize,
    reference: std::borrow::Cow<'a, str>,
    value: std::borrow::Cow<'a, str>,
}

/// A schematic unit stores only its identity, owner and geometry.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Placement<'a> {
    id: u64,
    component: u64,
    unit: std::borrow::Cow<'a, str>,
    x_nm: i64,
    y_nm: i64,
    rotation: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDocument {
    format: String,
    id: Uuid,
    name: String,
    sheet: LegacySheet,
    board: Option<LegacyBoard>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySheet {
    id: Uuid,
    symbols: Vec<LegacyPlacement>,
    wires: Vec<Connection<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPlacement {
    id: u64,
    definition: String,
    definition_revision: u32,
    reference_number: usize,
    x_nm: i64,
    y_nm: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyBoard {
    footprints: Vec<LegacyFootprint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyFootprint {
    id: u64,
    symbol: u64,
    definition: String,
    definition_revision: u32,
    pin_1_pad: u8,
    x_nm: i64,
    y_nm: i64,
}

/// Explicitly split each format 1 placement into a component and its unrotated unit.
/// The strict legacy schema refuses newer fields, and all converted records undergo the
/// current validator before adoption. IDs, wires and footprint associations are retained.
fn migrate_from_1(legacy: LegacyDocument) -> Result<Document, String> {
    if legacy.format != FORMAT_1 {
        return Err(format!("unsupported project format {}", legacy.format));
    }
    if legacy
        .sheet
        .wires
        .iter()
        .any(|wire| wire.bends.is_some() || !matches!(wire.from, Endpoint::Pin { .. }) || !matches!(wire.to, Endpoint::Pin { .. }))
    {
        return Err("format 1 cannot contain authored wire geometry".to_owned());
    }
    let mut components = Vec::with_capacity(legacy.sheet.symbols.len());
    let mut symbols = Vec::with_capacity(legacy.sheet.symbols.len());
    for record in legacy.sheet.symbols {
        let definition = crate::library::starter()
            .symbol_revision(&record.definition, record.definition_revision)
            .ok_or_else(|| format!("unavailable definition {} at revision {}", record.definition, record.definition_revision))?;
        components.push(ComponentRecord {
            id: record.id,
            definition: record.definition.into(),
            definition_revision: record.definition_revision,
            reference_number: record.reference_number,
            reference: format!("{}{}", definition.reference_prefix, record.reference_number).into(),
            value: definition.default_value().into(),
        });
        symbols.push(Placement {
            id: record.id,
            component: record.id,
            unit: definition.unit_id().into(),
            x_nm: record.x_nm,
            y_nm: record.y_nm,
            rotation: 0,
        });
    }
    let board = legacy.board.map(|board| BoardRecords {
        footprints: board
            .footprints
            .into_iter()
            .map(|record| Footprint {
                id: record.id,
                component: record.symbol,
                definition: record.definition,
                definition_revision: record.definition_revision,
                pin_1_pad: record.pin_1_pad,
                x_nm: record.x_nm,
                y_nm: record.y_nm,
            })
            .collect(),
    });
    Ok(Document {
        format: FORMAT.to_owned(),
        id: legacy.id,
        name: legacy.name,
        components,
        sheet: Sheet { id: legacy.sheet.id, symbols, junctions: None, wires: legacy.sheet.wires },
        board,
    })
}

/// Optional geometry may be omitted, but an explicit null is malformed rather than absent.
/// This keeps old format tags from accepting newer fields by spelling their value as null.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Stored wire geometry is absent on legacy auto-routed pin links.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Connection<'a> {
    id: u64,
    from: Endpoint<'a>,
    to: Endpoint<'a>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    bends: Option<Vec<Point>>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
enum Endpoint<'a> {
    Pin { symbol: u64, pin: std::borrow::Cow<'a, str> },
    Junction { junction: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Junction {
    id: u64,
    x_nm: i64,
    y_nm: i64,
}

impl From<WireEndpoint> for Endpoint<'static> {
    /// Preserve electrical identity independently of route geometry.
    fn from(endpoint: WireEndpoint) -> Self {
        match endpoint {
            WireEndpoint::Terminal(terminal) => Self::Pin { symbol: terminal.symbol, pin: terminal.number().into() },
            WireEndpoint::Junction(junction) => Self::Junction { junction },
        }
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
    components: u64,
    writing_symbols: bool,
    symbols: usize,
    junctions: u64,
    writing_junctions: bool,
    wires: usize,
    writing_wires: bool,
    footprints: usize,
    writing_board: bool,
}

impl Encoder {
    /// Start a native document, using format 3 when authored routes or junctions require it.
    pub fn new(project: Arc<Project>) -> Result<Self, String> {
        let name = serde_json::to_string(&project.name).map_err(|error| error.to_string())?;
        let format = if !project.junctions.is_empty() || project.wires.iter().any(|wire| wire.bends.is_some()) { FORMAT_3 } else { FORMAT };
        let mut bytes = Vec::new();
        write!(bytes, "{{\n  \"format\": \"{format}\",\n  \"id\": \"{}\",\n  \"name\": {name},\n  \"components\": [\n", project.id)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            project,
            bytes,
            components: 0,
            writing_symbols: false,
            symbols: 0,
            junctions: 0,
            writing_junctions: false,
            wires: 0,
            writing_wires: false,
            footprints: 0,
            writing_board: false,
        })
    }

    /// Write up to the given number of records. Callers stop on the first completed document.
    pub fn step(&mut self, records: usize) -> Result<Option<Vec<u8>>, String> {
        for _ in 0..records {
            if let Some((&id, component)) =
                self.project.components.range((std::ops::Bound::Excluded(self.components), std::ops::Bound::Unbounded)).next()
            {
                let record = ComponentRecord {
                    id,
                    definition: component.definition.id.as_str().into(),
                    definition_revision: component.definition.revision(),
                    reference_number: component.reference_number,
                    reference: component.reference.as_str().into(),
                    value: component.value.as_str().into(),
                };
                let more = self.project.components.last_key_value().is_some_and(|(&last, _)| last != id);
                write_record(&mut self.bytes, &record, more)?;
                self.components = id;
                continue;
            }
            if !self.writing_symbols {
                write!(self.bytes, "    ],\n  \"sheet\": {{\n    \"id\": \"{}\",\n    \"symbols\": [\n", self.project.sheet_id)
                    .map_err(|error| error.to_string())?;
                self.writing_symbols = true;
            }
            if let Some(symbol) = self.project.symbols.get(self.symbols) {
                let record = Placement {
                    id: symbol.ordinal,
                    component: symbol.component,
                    unit: symbol.unit.into(),
                    x_nm: symbol.position.x_nm,
                    y_nm: symbol.position.y_nm,
                    rotation: u16::from(symbol.rotation) * 90,
                };
                write_record(&mut self.bytes, &record, self.symbols + 1 < self.project.symbols.len())?;
                self.symbols += 1;
                continue;
            }
            if !self.project.junctions.is_empty() {
                if !self.writing_junctions {
                    self.bytes.extend_from_slice(b"    ],\n    \"junctions\": [\n");
                    self.writing_junctions = true;
                }
                if let Some((&id, &point)) =
                    self.project.junctions.range((std::ops::Bound::Excluded(self.junctions), std::ops::Bound::Unbounded)).next()
                {
                    let record = Junction { id, x_nm: point.x_nm, y_nm: point.y_nm };
                    let more = self.project.junctions.last_key_value().is_some_and(|(&last, _)| last != id);
                    write_record(&mut self.bytes, &record, more)?;
                    self.junctions = id;
                    continue;
                }
            }
            if !self.writing_wires {
                self.bytes.extend_from_slice(b"    ],\n    \"wires\": [\n");
                self.writing_wires = true;
            }
            if let Some(wire) = self.project.wires.get(self.wires) {
                let [from, to] = wire.endpoints();
                let record = Connection { id: wire.ordinal, from: from.into(), to: to.into(), bends: wire.bends.clone() };
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
    #[derive(Deserialize)]
    struct Header {
        format: String,
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|error| format!("invalid project JSON: {error}"))?;
    let (document, upgraded_from) = match header.format.as_str() {
        FORMAT | FORMAT_3 => (serde_json::from_slice::<Document>(bytes).map_err(|error| format!("invalid project JSON: {error}"))?, None),
        FORMAT_1 => {
            let legacy = serde_json::from_slice(bytes).map_err(|error| format!("invalid format 1 project: {error}"))?;
            (migrate_from_1(legacy)?, Some(1))
        }
        format => return Err(format!("unsupported project format {format}")),
    };
    let Document { format, id, name, components: records, sheet, board } = document;
    if format == FORMAT
        && (sheet.junctions.is_some()
            || sheet
                .wires
                .iter()
                .any(|wire| wire.bends.is_some() || !matches!(wire.from, Endpoint::Pin { .. }) || !matches!(wire.to, Endpoint::Pin { .. })))
    {
        return Err("format 2 cannot contain authored wire geometry".to_owned());
    }
    if id.is_nil() || sheet.id.is_nil() || id == sheet.id {
        return Err("invalid project or sheet identity".to_owned());
    }
    if name.is_empty() || name.trim() != name || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("invalid project name".to_owned());
    }
    if records.len() > MAX_SYMBOLS || sheet.symbols.len() > MAX_SYMBOLS || sheet.wires.len() > MAX_WIRES {
        return Err("project exceeds the symbol or wire limit".to_owned());
    }

    // Components have their own ID namespace. Placement/wire/footprint IDs retain the old namespace.
    let mut component_ids = BTreeSet::new();
    let mut references = BTreeSet::new();
    let mut shown = BTreeSet::new();
    let mut components = BTreeMap::new();
    for record in records {
        validate_id(&mut component_ids, record.id)?;
        let definition = crate::library::starter()
            .symbol_revision(&record.definition, record.definition_revision)
            .ok_or_else(|| format!("unavailable definition {} at revision {}", record.definition, record.definition_revision))?;
        if !(1..=MAX_REFERENCE_NUMBER).contains(&record.reference_number)
            || !references.insert((definition.reference_prefix(), record.reference_number))
        {
            return Err("invalid or duplicate component reference number".to_owned());
        }
        crate::schematic::check_reference(&record.reference)?;
        crate::schematic::check_value(&record.value)?;
        if !shown.insert(record.reference.clone()) {
            return Err("two components show the same reference".to_owned());
        }
        components.insert(
            record.id,
            Component {
                ordinal: record.id,
                definition,
                reference_number: record.reference_number,
                reference: record.reference.into_owned(),
                value: record.value.into_owned(),
            },
        );
    }
    let mut ids = BTreeSet::new();
    let mut symbols = Vec::with_capacity(sheet.symbols.len());
    let mut units = BTreeMap::new();
    for record in sheet.symbols {
        validate_id(&mut ids, record.id)?;
        let component = components.get(&record.component).ok_or_else(|| "symbol points to a missing component".to_owned())?;
        let unit = component.definition.unit_id();
        if record.unit != unit {
            return Err(format!("unavailable unit {} on component {}", record.unit, record.component));
        }
        if units.insert((record.component, unit), record.id).is_some() {
            return Err("component unit is placed more than once".to_owned());
        }
        if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.y_nm) {
            return Err("symbol position is outside the schematic bounds".to_owned());
        }
        let rotation = match record.rotation {
            degrees @ (0 | 90 | 180 | 270) => (degrees / 90) as u8,
            _ => return Err("symbol rotation must be 0, 90, 180 or 270 degrees".to_owned()),
        };
        symbols.push(SymbolPlacement {
            definition: component.definition,
            ordinal: record.id,
            component: record.component,
            unit,
            position: Point { x_nm: record.x_nm, y_nm: record.y_nm },
            rotation,
        });
    }
    if components.len() != units.len() {
        return Err("component has no placed unit".to_owned());
    }
    symbols.sort_unstable_by_key(|symbol| symbol.ordinal);
    let mut junctions = BTreeMap::new();
    let junction_records = sheet.junctions.unwrap_or_default();
    if junction_records.len() > MAX_WIRES * 2 {
        return Err("project exceeds the junction limit".to_owned());
    }
    for record in junction_records {
        validate_id(&mut ids, record.id)?;
        let point = Point { x_nm: record.x_nm, y_nm: record.y_nm };
        if !crate::wire::valid_point(&point) {
            return Err("junction is outside the schematic bounds".to_owned());
        }
        junctions.insert(record.id, point);
    }
    let resolve = |endpoint: Endpoint<'_>| match endpoint {
        Endpoint::Pin { symbol, pin } => symbols
            .binary_search_by_key(&symbol, |placed| placed.ordinal)
            .ok()
            .and_then(|index| Terminal::from_number(symbol, symbols[index].definition, &pin))
            .map(WireEndpoint::Terminal)
            .ok_or_else(|| format!("wire points to missing pin {pin} on symbol {symbol}")),
        Endpoint::Junction { junction } => junctions
            .contains_key(&junction)
            .then_some(WireEndpoint::Junction(junction))
            .ok_or_else(|| format!("wire points to missing junction {junction}")),
    };
    let mut pairs = BTreeSet::new();
    let mut used_junctions = BTreeSet::new();
    let mut wires = Vec::with_capacity(sheet.wires.len());
    for record in sheet.wires {
        validate_id(&mut ids, record.id)?;
        let mut endpoints = [resolve(record.from)?, resolve(record.to)?];
        let mut sorted = endpoints;
        sorted.sort_unstable();
        if sorted[0] == sorted[1] || !pairs.insert(sorted) {
            return Err("wire has identical endpoints or duplicates another wire".to_owned());
        }
        if let Some(bends) = &record.bends {
            if bends.len() > crate::wire::MAX_BENDS || bends.iter().any(|point| !crate::wire::valid_point(point)) {
                return Err("invalid wire bends".to_owned());
            }
        } else {
            endpoints = sorted;
        }
        for endpoint in endpoints {
            if let WireEndpoint::Junction(id) = endpoint {
                used_junctions.insert(id);
            }
        }
        wires.push(Wire { ordinal: record.id, endpoints, bends: record.bends });
    }
    if used_junctions.len() != junctions.len() {
        return Err("junction has no connected wire".to_owned());
    }
    wires.sort_unstable_by_key(|wire| wire.ordinal);
    let mut layout = Board::default();
    if let Some(mut board) = board {
        if board.footprints.len() > components.len() {
            return Err("board has more footprints than schematic components".to_owned());
        }
        board.footprints.sort_unstable_by_key(|footprint| footprint.id);
        for footprint in board.footprints {
            validate_id(&mut ids, footprint.id)?;
            match components.get(&footprint.component) {
                None => return Err("footprint points to a missing component".to_owned()),
                Some(component) if !component.definition.fits_two_pad_footprint() => {
                    return Err("the two-pad footprint only fits symbols with exactly pins 1 and 2".to_owned());
                }
                Some(_) => {}
            }
            layout.append(footprint).map_err(str::to_owned)?;
        }
    }

    Ok(Arc::new(Project {
        id,
        sheet_id: sheet.id,
        name,
        symbols: Arc::new(symbols),
        components: Arc::new(components),
        units: Arc::new(units),
        wires: Arc::new(wires),
        junctions: Arc::new(junctions),
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

    /// Every starter library symbol places through the shared command, numbers its references
    /// by prefix and survives a save and reopen with its kind, reference and value.
    #[test]
    fn every_library_symbol_places_and_round_trips() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"name": "All parts"})).unwrap();
        for (index, definition) in crate::library::starter().symbols().iter().enumerate() {
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
            [
                ("capacitor", "C1", "100nF"),
                ("connector", "J1", "Input"),
                ("header-4", "J2", "Header"),
                ("led", "D1", "LED"),
                ("resistor", "R1", "1k"),
            ]
            .map(|(kind, reference, value)| (json!(kind), json!(reference), json!(value)))
        );
        let loaded = decode(&encode(session.snapshot().unwrap()).unwrap()).unwrap();
        let mut reopened = Session::default();
        reopened.open_snapshot(loaded);
        assert_eq!(reopened.inspect()["project"]["schematic"]["symbols"], symbols);
    }

    /// Multi-pin library symbols wire by pin number through the shared command, report every
    /// pin with its name and electrical type, and survive a save and reopen. A pin the symbol
    /// doesn't have is refused, and the two-pad footprint refuses a four-pin header both as a
    /// command and in a forged file.
    #[test]
    fn multi_pin_symbols_wire_by_number_and_refuse_missing_pins() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"name": "Supply"})).unwrap();
        for (definition, x_nm) in [("led", 0), ("header-4", 50_800_000)] {
            session
                .apply(PLACE_SYMBOL, &json!({"definition": definition, "x_nm": x_nm, "y_nm": 0, "expected_revision": session.revision()}))
                .unwrap();
        }
        let wire = |session: &mut Session, from: &str, to: &str| {
            session.apply(
                CONNECT_PINS,
                &json!({
                    "from": {"symbol": "symbol-2", "terminal": from},
                    "to": {"symbol": "symbol-3", "terminal": to},
                    "expected_revision": session.revision()
                }),
            )
        };
        wire(&mut session, "2", "4").unwrap();
        assert!(wire(&mut session, "4", "1").is_err(), "the LED has no pin 4");
        assert!(wire(&mut session, "9", "1").is_err(), "no library symbol has a pin 9");
        let state = session.inspect();
        let header = &state["project"]["schematic"]["symbols"][1];
        assert_eq!(header["pins"].as_array().unwrap().len(), 4);
        assert_eq!(header["pins"][3], json!({"number": "4", "electrical_type": "passive", "x_nm": 44_450_000, "y_nm": 3_810_000}));
        assert_eq!(state["project"]["schematic"]["symbols"][0]["pins"][1]["name"], "Cathode");
        let net = &state["project"]["schematic"]["netlist"]["nets"][0]["terminals"];
        assert_eq!(net, &json!([{"symbol": "symbol-2", "terminal": "2"}, {"symbol": "symbol-3", "terminal": "4"}]));

        let bytes = encode(session.snapshot().unwrap()).unwrap();
        let mut reopened = Session::default();
        reopened.open_snapshot(decode(&bytes).unwrap());
        assert_eq!(reopened.inspect()["project"], state["project"]);

        let footprint = json!({
            "symbol": "symbol-3", "definition": "generic-through-hole-2",
            "pin_1_pad": 1, "x_nm": 0, "y_nm": 0, "expected_revision": session.revision(),
        });
        assert!(session.apply(crate::PLACE_FOOTPRINT, &footprint).unwrap_err().to_string().contains("exactly pins 1 and 2"));
        let mut forged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        forged["board"] = json!({"footprints": [{
            "id": 9, "component": 3, "definition": "generic-through-hole-2", "definition_revision": 1, "pin_1_pad": 1, "x_nm": 0, "y_nm": 0
        }]});
        assert!(decode(&serde_json::to_vec(&forged).unwrap()).unwrap_err().contains("exactly pins 1 and 2"));
        let mut missing = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        missing["sheet"]["wires"][0]["from"]["pin"] = json!("4");
        assert!(decode(&serde_json::to_vec(&missing).unwrap()).unwrap_err().contains("missing pin 4"));
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
        assert!(decode(forged.as_bytes()).unwrap_err().contains("unknown field `rotation`"));
    }

    /// Explicit null geometry cannot bypass format 1/2's refusal of newer fields.
    #[test]
    fn old_formats_refuse_null_wire_geometry() {
        for fixture in [FORMAT_1_FIXTURE, FORMAT_2_FIXTURE] {
            let original: serde_json::Value = serde_json::from_slice(fixture).unwrap();
            let mut bends = original.clone();
            bends["sheet"]["wires"][0]["bends"] = serde_json::Value::Null;
            assert!(decode(&serde_json::to_vec(&bends).unwrap()).is_err());
            let mut junctions = original;
            junctions["sheet"]["junctions"] = serde_json::Value::Null;
            assert!(decode(&serde_json::to_vec(&junctions).unwrap()).is_err());
        }
    }

    /// Component fields and unit rotation survive a save, while invalid rotations and
    /// duplicate component references are refused.
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
        assert_eq!(text.matches("\"reference\":").count(), 2, "each component stores its reference");
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
        invalid["board"]["footprints"][0]["component"] = json!(99);
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    /// Component and symbol IDs may differ. Field edits only change the component record;
    /// wire/pad addresses, geometry, undo/redo and last-unit deletion follow explicit ownership.
    #[test]
    fn independent_component_identity_survives_edit_history_and_board_links() {
        let mut document: serde_json::Value = serde_json::from_slice(FORMAT_2_FIXTURE).unwrap();
        document["components"][0]["id"] = json!(1001);
        document["sheet"]["symbols"][0]["component"] = json!(1001);
        let mut session = Session::default();
        session.open_snapshot(decode(&serde_json::to_vec(&document).unwrap()).unwrap());
        assert!(session.revision() > 1001, "new object IDs advance past component IDs too");
        session
            .apply(
                crate::PLACE_FOOTPRINT,
                &json!({
                    "symbol": "symbol-2", "definition": "generic-through-hole-2", "pin_1_pad": 2,
                    "x_nm": 0, "y_nm": 0, "expected_revision": session.revision()
                }),
            )
            .unwrap();
        let before = session.snapshot().unwrap();
        let pin = before.symbols()[0].terminals()[0];
        let pad = before.pad_for_terminal(pin).unwrap();
        let bytes = encode(before.clone()).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["board"]["footprints"][0]["component"], 1001);
        assert!(stored["board"]["footprints"][0].get("symbol").is_none());
        assert_eq!(before.terminal_for_pad(pad), Some(pin));
        let fields = json!({"symbol": "symbol-2", "reference": "R42", "value": "4.7k", "expected_revision": session.revision()});
        session.apply(crate::SET_SYMBOL_FIELDS, &fields).unwrap();
        let edited = session.snapshot().unwrap();
        let after = encode(edited.clone()).unwrap();
        let old_text = std::str::from_utf8(&bytes).unwrap();
        let new_text = std::str::from_utf8(&after).unwrap();
        let changed: Vec<_> = old_text.lines().zip(new_text.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(old_text.lines().count(), new_text.lines().count());
        assert_eq!(changed.len(), 1);
        assert!(changed[0].1.contains("\"id\":1001"));
        assert!(changed[0].1.contains("\"reference\":\"R42\""));
        assert_eq!(edited.symbols(), before.symbols());
        assert!(edited.same_connectivity(&before));
        assert_eq!(edited.pad_for_terminal(pin), Some(pad));
        assert_eq!(edited.terminal_for_pad(pad), Some(pin));
        let reopened = decode(&after).unwrap();
        assert_eq!(reopened.reference(&reopened.symbols()[0]), "R42");
        assert_eq!(reopened.terminal_for_pad(pad), Some(pin));
        assert_eq!(encode(reopened).unwrap(), after);
        let state = session.inspect();
        session.apply(crate::SET_SYMBOL_FIELDS, &fields).unwrap_err();
        assert_eq!(session.inspect(), state, "stale field changes are atomic");
        let duplicate = json!({"symbol": "symbol-3", "reference": "R42", "value": "1k", "expected_revision": session.revision()});
        session.apply(crate::SET_SYMBOL_FIELDS, &duplicate).unwrap_err();
        assert_eq!(session.inspect(), state, "duplicate designators do not partially change fields");
        session.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        session.apply("edit.redo", &serde_json::Value::Null).unwrap();
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &edited));
        session.apply(crate::DELETE_SYMBOL, &json!({"symbol": "symbol-2", "expected_revision": session.revision()})).unwrap();
        let deleted = session.snapshot().unwrap();
        assert_eq!(deleted.components().count(), 1);
        assert!(deleted.wires().is_empty());
        assert!(deleted.terminal_for_pad(pad).is_none());
        assert!(deleted.board.footprints().is_empty());
        session.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &edited));
    }

    /// Old board links are migrated from symbol IDs to component IDs without altering pads or wires.
    #[test]
    fn format_1_footprint_migration_preserves_mapping_in_both_encodings() {
        let mut legacy: serde_json::Value = serde_json::from_slice(FORMAT_1_FIXTURE).unwrap();
        legacy["board"] = json!({"footprints": [{
            "id": 5, "symbol": 2, "definition": "generic-through-hole-2", "definition_revision": 1,
            "pin_1_pad": 2, "x_nm": 0, "y_nm": 0
        }]});
        let migrated = decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        let terminal = migrated.symbols()[0].terminals()[0];
        let pad = migrated.pad_for_terminal(terminal).unwrap();
        assert_eq!(pad.inspect(), json!({"footprint": "footprint-5", "pad": "2"}));
        for encoding in [Encoding::Json, Encoding::Zip] {
            let bytes = encode_file(migrated.clone(), encoding).unwrap();
            let (project, detected) = decode_file(&bytes).unwrap();
            assert_eq!(detected, encoding);
            assert_eq!(project.terminal_for_pad(pad), Some(terminal));
            assert_eq!(project.wires()[0].terminals(), migrated.wires()[0].terminals());
            assert_eq!(encode_file(project, encoding).unwrap(), bytes);
        }
        legacy["board"]["footprints"][0]["symbol"] = json!(999);
        assert!(decode(&serde_json::to_vec(&legacy).unwrap()).unwrap_err().contains("missing component"));
    }

    /// Reject dangling ownership, duplicate unit placements and mixed schemas before adoption.
    #[test]
    fn rejects_invalid_component_unit_relationships() {
        let valid: serde_json::Value = serde_json::from_slice(FORMAT_2_FIXTURE).unwrap();
        for (path, value) in [
            ("/components/1/id", json!(2)),
            ("/components/1/reference", json!("R1")),
            ("/sheet/symbols/0/component", json!(999)),
            ("/sheet/symbols/1/component", json!(2)),
            ("/sheet/symbols/0/unit", json!("missing")),
        ] {
            let mut bad = valid.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(decode(&serde_json::to_vec(&bad).unwrap()).is_err(), "{path}");
        }
        for (path, field, value) in [
            ("/sheet/symbols/0", "reference", json!("R4")),
            ("/sheet/symbols/0", "definition", json!("resistor")),
            ("/components/0", "x_nm", json!(0)),
        ] {
            let mut bad = valid.clone();
            bad.pointer_mut(path).unwrap()[field] = value;
            assert!(decode(&serde_json::to_vec(&bad).unwrap()).is_err(), "{field}");
        }
        let mut bad = valid.clone();
        bad["sheet"]["symbols"][0].as_object_mut().unwrap().remove("rotation");
        assert!(decode(&serde_json::to_vec(&bad).unwrap()).is_err());
        let mut bad = valid;
        bad["sheet"]["symbols"] = json!([]);
        bad["sheet"]["wires"] = json!([]);
        assert!(decode(&serde_json::to_vec(&bad).unwrap()).unwrap_err().contains("no placed unit"));
    }

    /// Reject malformed topology, future schema, unpinned definitions, and oversized input.
    #[test]
    fn rejects_bad_files_before_adoption() {
        let bytes = encode(fixture().snapshot().unwrap()).unwrap();
        let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (path, invalid) in [
            ("/format", json!("pcbcraft-project/999")),
            ("/components/0/definition_revision", json!(99)),
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
