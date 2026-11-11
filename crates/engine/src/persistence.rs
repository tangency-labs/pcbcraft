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
mod documents;
pub use documents::{
    Documents, DocumentsEncoder, ENTRY_POINT, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS, MAX_DOCUMENTS_BYTES, convert_legacy_documents, decode_documents,
    encode_documents, is_document_path,
};
mod packed;
pub use packed::{PackedEncoder, PackedProject, decode_legacy_packed, decode_packed, encode_packed, encode_packed_documents};
#[cfg(test)]
mod split_tests;

/// Explain the intentional app 0.1.0 break when a legacy standalone document is opened.
pub const UNSUPPORTED_PROJECT_LAYOUT: &str = "Unsupported project layout. PCBcraft 0.1.0 opens project folders through project.json and packed .pc ZIP files; legacy standalone JSON projects and project.pc entry points are not accepted by normal Open. Convert previous split projects with pcbcraft-cli convert SOURCE DESTINATION.";

/// Maximum number of bytes in a supported project ZIP.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// JSON document or ZIP package encoding, detected independently of the filename.
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
            Self::Json => MAX_DOCUMENT_BYTES,
            Self::Zip => MAX_ARCHIVE_BYTES,
        }
    }
}

/// Read a diagnostic single-document fixture or current packed project, including legacy
/// inputs refused by normal app/CLI opens. Use [`decode_packed`] for current archives or
/// [`decode_documents`] after folder I/O; this compatibility helper is for tests and benchmarks.
pub fn decode_diagnostic_file(bytes: &[u8]) -> Result<(Arc<Project>, Encoding), String> {
    let encoding = Encoding::detect(bytes);
    if bytes.len() > encoding.input_limit() {
        return Err("project exceeds the native file size limit".to_owned());
    }
    let project = match encoding {
        Encoding::Json => decode(bytes)?,
        Encoding::Zip => {
            if let Ok(payload) = archive::unpack(bytes) {
                decode(&payload)?
            } else {
                decode_packed(bytes)?.project
            }
        }
    };
    Ok((project, encoding))
}

/// Encode a single-document project or container v1, retained for migration fixtures.
/// Configured boards are refused; normal saves use [`encode_documents`] or [`encode_packed`].
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

/// Format 4 keeps a full copy of every symbol definition the project uses, with the library it
/// came from, so a project opens without that library. It keeps format 3's wire geometry.
const FORMAT: &str = "pcbcraft-project/4";
/// Single-document watermark encoding; versions 5 through 7 identify split-document forms.
const WATERMARK_FORMAT: &str = "pcbcraft-project/8";
/// Explicit optional physical-board registration in the diagnostic single-document form.
const OPTIONAL_BOARD_FORMAT: &str = "pcbcraft-project/10";
/// Format 3 added free wire endpoints, junctions and authored bends; it opens through
/// [`migrate_to_4`].
const FORMAT_3: &str = "pcbcraft-project/3";
/// Format 2 separated component fields from unit geometry; it opens through [`migrate_to_4`].
const FORMAT_2: &str = "pcbcraft-project/2";
/// The first project format, which opens through [`migrate_from_1`] and then [`migrate_to_4`].
const FORMAT_1: &str = "pcbcraft-project/1";
/// Maximum number of bytes in a plain project document or ZIP project payload.
pub const MAX_PROJECT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    #[serde(default, deserialize_with = "present")]
    boards: Option<Vec<crate::board::identity::Registration>>,
    id: Uuid,
    name: String,
    #[serde(default, deserialize_with = "present")]
    numeric_id_high_water: Option<u64>,
    /// One saved copy per definition the components use: a symbol file plus its `library`.
    symbol_definitions: Vec<serde_json::Value>,
    components: Vec<ComponentRecord<'static>>,
    sheet: Sheet,
    board: Option<BoardRecords>,
}

/// A format 2 or 3 document, which named starter definitions without saving copies; read only
/// to migrate it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document2 {
    format: String,
    id: Uuid,
    name: String,
    components: Vec<ComponentRecord2>,
    sheet: Sheet,
    board: Option<BoardRecords>,
}

/// A format 2 or 3 component, which named a starter library definition without saving a copy.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRecord2 {
    id: u64,
    definition: String,
    definition_revision: u32,
    reference_number: usize,
    reference: String,
    value: String,
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
    #[serde(default, deserialize_with = "present")]
    baseline: Option<crate::board::update::Baseline>,
    footprints: Vec<Footprint>,
}

/// Component data has no schematic coordinates or footprint geometry.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ComponentRecord<'a> {
    id: u64,
    /// With `definition` and `definition_revision`, names one of the project's saved copies.
    library: std::borrow::Cow<'a, str>,
    definition: std::borrow::Cow<'a, str>,
    definition_revision: u32,
    reference_number: usize,
    reference: std::borrow::Cow<'a, str>,
    value: std::borrow::Cow<'a, str>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    metadata: Option<crate::schematic::fields::Metadata>,
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
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    field_layout: Option<crate::schematic::fields::Layouts>,
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
fn migrate_from_1(legacy: LegacyDocument) -> Result<Document2, String> {
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
        components.push(ComponentRecord2 {
            id: record.id,
            definition: record.definition,
            definition_revision: record.definition_revision,
            reference_number: record.reference_number,
            reference: format!("{}{}", definition.reference_prefix, record.reference_number),
            value: definition.default_value().to_owned(),
        });
        symbols.push(Placement {
            id: record.id,
            component: record.id,
            unit: definition.unit_id().to_owned().into(),
            x_nm: record.x_nm,
            y_nm: record.y_nm,
            rotation: 0,
            field_layout: None,
        });
    }
    let board = legacy.board.map(|board| BoardRecords {
        baseline: None,
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
                rotation: 0,
                library: None,
                mapping: None,
                bottom: false,
                reference_offset: None,
                reference_text: None,
            })
            .collect(),
    });
    Ok(Document2 {
        format: FORMAT_2.to_owned(),
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

/// Explicitly bring a format 2 or 3 document up to format 4: each component's named starter
/// library definition must exist at its saved revision, and the document gains one saved copy
/// per definition used, tagged with the starter library's ID. Format 2 may not carry format 3's
/// wire geometry. Everything else is carried over and then checked by the current validator.
fn migrate_to_4(document: Document2) -> Result<Document, String> {
    if document.format != FORMAT_2 && document.format != FORMAT_3 {
        return Err(format!("unsupported project format {}", document.format));
    }
    let sheet = &document.sheet;
    let geometry = sheet.junctions.is_some()
        || sheet
            .wires
            .iter()
            .any(|wire| wire.bends.is_some() || !matches!(wire.from, Endpoint::Pin { .. }) || !matches!(wire.to, Endpoint::Pin { .. }));
    if document.format == FORMAT_2 && geometry {
        return Err("format 2 cannot contain authored wire geometry".to_owned());
    }
    let mut used = BTreeMap::new();
    let mut components = Vec::with_capacity(document.components.len());
    for record in document.components {
        let definition = crate::library::starter()
            .symbol_revision(&record.definition, record.definition_revision)
            .cloned()
            .ok_or_else(|| format!("unavailable definition {} at revision {}", record.definition, record.definition_revision))?;
        components.push(ComponentRecord {
            id: record.id,
            library: definition.library.clone().into(),
            definition: record.definition.into(),
            definition_revision: record.definition_revision,
            reference_number: record.reference_number,
            reference: record.reference.into(),
            value: record.value.into(),
            metadata: None,
        });
        used.insert((definition.id.clone(), definition.revision()), definition);
    }
    let symbol_definitions = used
        .into_values()
        .map(|definition| serde_json::to_value(crate::library::saved_copy(&definition)).map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    Ok(Document {
        format: FORMAT.to_owned(),
        boards: None,
        id: document.id,
        name: document.name,
        numeric_id_high_water: None,
        symbol_definitions,
        components,
        sheet: document.sheet,
        board: document.board,
    })
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
    Label { label: u64 },
    NetLabel { net_label: u64 },
    SheetPin { sheet_pin: u64 },
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
            WireEndpoint::Terminal(terminal) => Self::Pin { symbol: terminal.symbol, pin: terminal.number().to_owned().into() },
            WireEndpoint::Junction(junction) => Self::Junction { junction },
            WireEndpoint::NetLabel(net_label) => Self::NetLabel { net_label },
            WireEndpoint::Label(label) => Self::Label { label },
            WireEndpoint::SheetPin(sheet_pin) => Self::SheetPin { sheet_pin },
        }
    }
}

/// Encode deterministic single-document JSON for migration fixtures and diagnostic tools.
/// Legacy snapshots retain format 4; adopted allocation metadata requires format 8.
/// Configured boards are refused. Normal saves use [`encode_documents`] or [`encode_packed`].
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
    /// The distinct definitions the components use, sorted by library, ID and revision. Filled
    /// on the first step so starting a save never walks the schematic.
    definitions: Option<Vec<Arc<crate::schematic::SymbolDefinition>>>,
    written_definitions: usize,
    writing_components: bool,
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
    /// Begin incremental single-document encoding after rejecting unsupported document features.
    /// Metadata, text appearance, setup, physical definitions, silkscreen, outlines, layer
    /// drawings and rotation require split documents or an archive; refuse lossy export.
    pub fn new(project: Arc<Project>) -> Result<Self, String> {
        if project.components.values().any(|c| !c.metadata.is_empty())
            || project.sheets.iter().any(|s| s.symbols.iter().any(|s| !s.field_layout.is_empty()))
        {
            return Err("component fields and text appearance require the split project format".into());
        }
        if project.sheets.len() != 1 || project.sheet_name() != "Main" {
            return Err("legacy single-file encoding cannot represent multiple or renamed sheets; use split documents or a packed archive".to_owned());
        }
        if !project.labels.is_empty() || !project.subsheets.is_empty() || !project.net_labels.is_empty() {
            return Err("legacy single-file encoding cannot represent hierarchy; use split documents or a packed archive".into());
        }
        if !project.schematic_packages.assignments.is_empty()
            || !project.board.assignments.is_empty()
            || project.board.baseline.is_some()
            || project.board.silkscreen.is_some()
            || project.board.outline.is_some()
            || project.board.drawings.is_some()
            || !project.board.tracks.is_empty()
            || !project.board.vias.is_empty()
            || project.board.setup.is_some()
            || !project.board.definitions.is_empty()
            || project.board.footprints().iter().any(|f| f.rotation != 0 || f.bottom || f.reference_offset.is_some() || f.reference_text.is_some())
        {
            return Err(
                "legacy single-file encoding cannot represent board setup, accepted board baselines, physical footprints, independent assignments, silkscreen settings, outlines, layer drawings, tracks, vias, footprint rotation, bottom-side footprints, reference offsets or print overrides; use split documents or a packed archive".to_owned()
            );
        }
        let name = serde_json::to_string(&project.name).map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let registrations = project.board.ownership.records();
        let format = if registrations.is_some() {
            OPTIONAL_BOARD_FORMAT
        } else if project.numeric_id_high_water.is_some() {
            WATERMARK_FORMAT
        } else {
            FORMAT
        };
        write!(bytes, "{{\n  \"format\": \"{format}\",\n  \"id\": \"{}\",\n  \"name\": {name},\n", project.id).map_err(|error| error.to_string())?;
        if let Some(high_water) = project.numeric_id_high_water {
            writeln!(bytes, "  \"numeric_id_high_water\": {high_water},").map_err(|error| error.to_string())?;
        }
        if let Some(registrations) = registrations {
            writeln!(bytes, "  \"boards\": {},", serde_json::to_string(&registrations).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        }
        bytes.extend_from_slice(b"  \"symbol_definitions\": [\n");
        Ok(Self {
            project,
            bytes,
            definitions: None,
            written_definitions: 0,
            writing_components: false,
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
        let project = &self.project;
        let definitions = self.definitions.get_or_insert_with(|| {
            let mut used: Vec<_> = project.components.values().map(|component| component.definition.clone()).collect();
            used.sort_unstable_by(|a, b| (&a.library, &a.id, a.revision()).cmp(&(&b.library, &b.id, b.revision())));
            used.dedup_by(|a, b| (&a.library, &a.id, a.revision()) == (&b.library, &b.id, b.revision()));
            used
        });
        for _ in 0..records {
            if let Some(definition) = definitions.get(self.written_definitions) {
                let more = self.written_definitions + 1 < definitions.len();
                crate::library::write_saved_copy(&mut self.bytes, definition)?;
                self.bytes.extend_from_slice(if more { b",\n\n" } else { b"\n" });
                self.written_definitions += 1;
                continue;
            }
            if !self.writing_components {
                self.bytes.extend_from_slice(b"    ],\n  \"components\": [\n");
                self.writing_components = true;
            }
            if let Some((&id, component)) =
                self.project.components.range((std::ops::Bound::Excluded(self.components), std::ops::Bound::Unbounded)).next()
            {
                let record = ComponentRecord {
                    id,
                    library: component.definition.library.as_str().into(),
                    definition: component.definition.id.as_str().into(),
                    definition_revision: component.definition.revision(),
                    reference_number: component.reference_number,
                    reference: component.reference.as_str().into(),
                    value: component.value.as_str().into(),
                    metadata: None,
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
                    unit: symbol.unit.as_str().into(),
                    x_nm: symbol.position.x_nm,
                    y_nm: symbol.position.y_nm,
                    rotation: u16::from(symbol.rotation) * 90,
                    field_layout: None,
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
        FORMAT | WATERMARK_FORMAT | OPTIONAL_BOARD_FORMAT => {
            (serde_json::from_slice::<Document>(bytes).map_err(|error| format!("invalid project JSON: {error}"))?, None)
        }
        FORMAT_2 | FORMAT_3 => {
            let version = if header.format == FORMAT_2 { 2 } else { 3 };
            let document = serde_json::from_slice(bytes).map_err(|error| format!("invalid format {version} project: {error}"))?;
            (migrate_to_4(document)?, Some(version))
        }
        FORMAT_1 => {
            let legacy = serde_json::from_slice(bytes).map_err(|error| format!("invalid format 1 project: {error}"))?;
            (migrate_to_4(migrate_from_1(legacy)?)?, Some(1))
        }
        format => return Err(format!("unsupported project format {format}")),
    };
    if document.components.iter().any(|c| c.metadata.is_some()) || document.sheet.symbols.iter().any(|s| s.field_layout.is_some()) {
        return Err("component fields and text appearance require split project/sheet formats 11/4".into());
    }
    if document.board.as_ref().is_some_and(|b| b.footprints.iter().any(|f| f.rotation != 0 || f.library.is_some())) {
        return Err("physical footprints and rotation require split board format 5 or 4 respectively".to_owned());
    }
    if document.board.as_ref().is_some_and(|b| b.footprints.iter().any(|f| f.reference_offset.is_some() || f.reference_text.is_some())) {
        return Err("footprint reference offsets and print overrides require split board format 15".into());
    }
    if document.board.as_ref().is_some_and(|board| board.baseline.is_some()) {
        return Err("accepted board baselines require split board format 10".into());
    }
    validate_document(document, upgraded_from, BTreeMap::new())
}

/// Validate the combined legacy or split records without imposing a single-file size limit.
fn validate_document(
    document: Document,
    upgraded_from: Option<u32>,
    definitions: BTreeMap<crate::board::definition::Key, Arc<crate::board::definition::FootprintDefinition>>,
) -> Result<Arc<Project>, String> {
    validate_with_hierarchy(document, upgraded_from, definitions, &BTreeSet::new(), &BTreeSet::new())
}

/// Validate hierarchy anchors, live objects and accepted historical package references
/// with one object-ID and wire validator before adopting any project snapshot.
fn validate_with_hierarchy(
    document: Document,
    upgraded_from: Option<u32>,
    definitions: BTreeMap<crate::board::definition::Key, Arc<crate::board::definition::FootprintDefinition>>,
    anchors: &BTreeSet<WireEndpoint>,
    hierarchy_ids: &BTreeSet<u64>,
) -> Result<Arc<Project>, String> {
    let Document { format, boards, id, name, numeric_id_high_water, symbol_definitions, components: records, sheet, board } = document;
    if !matches!(format.as_str(), FORMAT | WATERMARK_FORMAT | OPTIONAL_BOARD_FORMAT) || (format == OPTIONAL_BOARD_FORMAT) != boards.is_some() {
        return Err(format!("unsupported project format {format}"));
    }
    if matches!(format.as_str(), WATERMARK_FORMAT | OPTIONAL_BOARD_FORMAT) != numeric_id_high_water.is_some() {
        return Err("numeric ID watermark requires project format 8 or 10".into());
    }
    if boards.as_ref().is_some_and(Vec::is_empty) && board.is_some() {
        return Err("schematic-only project cannot contain physical board data".into());
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
    if symbol_definitions.len() > records.len() {
        return Err("project saves more symbol definitions than it has components".to_owned());
    }

    // Components resolve their definitions only from the project's own saved copies.
    let mut saved = BTreeMap::new();
    for value in symbol_definitions {
        let definition = crate::library::read_saved_copy(value).map_err(|error| format!("invalid saved symbol definition: {error}"))?;
        let key = (definition.library.clone(), definition.id.clone(), definition.revision());
        if saved.contains_key(&key) {
            return Err(format!("symbol {} revision {} from library {} is saved twice", key.1, key.2, key.0));
        }
        saved.insert(key, (definition, false));
    }

    // Components have their own ID namespace. Placement/wire/footprint IDs retain the old namespace.
    let mut component_ids = BTreeSet::new();
    let mut references = BTreeSet::new();
    let mut shown = BTreeSet::new();
    let mut components = BTreeMap::new();
    for record in records {
        validate_id(&mut component_ids, record.id)?;
        let (definition, used) =
            saved.get_mut(&(record.library.to_string(), record.definition.to_string(), record.definition_revision)).ok_or_else(|| {
                format!(
                    "component {} uses symbol {} revision {}, which the project doesn't save",
                    record.id, record.definition, record.definition_revision
                )
            })?;
        *used = true;
        let definition = definition.clone();
        if !(1..=MAX_REFERENCE_NUMBER).contains(&record.reference_number)
            || !references.insert((definition.reference_prefix().to_owned(), record.reference_number))
        {
            return Err("invalid or duplicate component reference number".to_owned());
        }
        crate::schematic::check_reference(&record.reference)?;
        crate::schematic::check_value(&record.value)?;
        if !shown.insert(record.reference.clone()) {
            return Err("two components show the same reference".to_owned());
        }
        let metadata = record.metadata.unwrap_or_default();
        metadata.validate()?;
        components.insert(
            record.id,
            Component {
                ordinal: record.id,
                definition,
                reference_number: record.reference_number,
                reference: record.reference.into_owned(),
                value: record.value.into_owned(),
                metadata,
            },
        );
    }
    if let Some(((library, id, revision), _)) = saved.iter().find(|(_, (_, used))| !used) {
        return Err(format!("symbol {id} revision {revision} from library {library} is saved but no component uses it"));
    }
    let mut ids = hierarchy_ids.clone();
    let mut symbols = Vec::with_capacity(sheet.symbols.len());
    let mut units = BTreeMap::new();
    for record in sheet.symbols {
        validate_id(&mut ids, record.id)?;
        let component = components.get(&record.component).ok_or_else(|| "symbol points to a missing component".to_owned())?;
        let unit = component.definition.unit_id();
        if record.unit != unit {
            return Err(format!("unavailable unit {} on component {}", record.unit, record.component));
        }
        if units.insert((record.component, unit.to_owned()), record.id).is_some() {
            return Err("component unit is placed more than once".to_owned());
        }
        if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&record.y_nm) {
            return Err("symbol position is outside the schematic bounds".to_owned());
        }
        let rotation = match record.rotation {
            degrees @ (0 | 90 | 180 | 270) => (degrees / 90) as u8,
            _ => return Err("symbol rotation must be 0, 90, 180 or 270 degrees".to_owned()),
        };
        let field_layout = record.field_layout.unwrap_or_default();
        crate::schematic::fields::validate_layouts(&field_layout, &component.metadata)?;
        symbols.push(SymbolPlacement {
            definition: component.definition.clone(),
            ordinal: record.id,
            component: record.component,
            unit: unit.to_owned(),
            position: Point { x_nm: record.x_nm, y_nm: record.y_nm },
            field_layout,
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
        Endpoint::NetLabel { net_label } => anchors
            .contains(&WireEndpoint::NetLabel(net_label))
            .then_some(WireEndpoint::NetLabel(net_label))
            .ok_or_else(|| "wire points to missing named label".to_owned()),
        Endpoint::Label { label } => anchors
            .contains(&WireEndpoint::Label(label))
            .then_some(WireEndpoint::Label(label))
            .ok_or_else(|| "wire points to missing hierarchical label".to_owned()),
        Endpoint::SheetPin { sheet_pin } => anchors
            .contains(&WireEndpoint::SheetPin(sheet_pin))
            .then_some(WireEndpoint::SheetPin(sheet_pin))
            .ok_or_else(|| "wire points to missing sheet pin".to_owned()),
        Endpoint::Pin { symbol, pin } => symbols
            .binary_search_by_key(&symbol, |placed| placed.ordinal)
            .ok()
            .and_then(|index| Terminal::from_number(symbol, &symbols[index].definition, &pin))
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
    layout.definitions = definitions;
    if let Some(mut board) = board {
        layout.baseline = board.baseline.take().map(Arc::new);
        if let Some(baseline) = &layout.baseline {
            baseline.validate()?;
            if baseline.project != id {
                return Err("accepted board baseline belongs to another project".into());
            }
        }
        if board.footprints.len() > layout.baseline.as_ref().map_or(components.len(), |baseline| baseline.components.len()) {
            return Err("board has more footprints than schematic components".to_owned());
        }
        board.footprints.sort_unstable_by_key(|footprint| footprint.id);
        for footprint in board.footprints {
            validate_id(&mut ids, footprint.id)?;
            if let Some(baseline) = &layout.baseline {
                baseline.validate_footprint(&footprint)?;
            } else {
                match components.get(&footprint.component) {
                    None => return Err("footprint points to a missing component".to_owned()),
                    Some(component) if component.metadata.attributes.exclude_from_board => {
                        return Err("a board footprint points to a component excluded from the board".into());
                    }
                    Some(component) if !layout.mapping_valid(&footprint, &component.definition) => {
                        return Err(
                        "the two-pad footprint only fits symbols with exactly pins 1 and 2; physical footprint mappings must cover every terminal"
                            .to_owned(),
                    );
                    }
                    Some(_) => {}
                }
            }
            layout.append(footprint).map_err(str::to_owned)?;
        }
    }

    let mut result = Project {
        id,
        numeric_id_high_water,
        sheets: Arc::new(vec![crate::sheets::SchematicSheet {
            sheet_id: sheet.id,
            sheet_name: "Main".to_owned(),
            labels: Arc::default(),
            net_labels: Arc::default(),
            subsheets: Arc::default(),
            symbols: Arc::new(symbols),
            wires: Arc::new(wires),
            junctions: Arc::new(junctions),
            connectivity: Arc::default(),
        }]),
        sheet_index: 0,
        name,
        components: Arc::new(components),
        units: Arc::new(units),
        schematic_packages: Arc::default(),
        board: Arc::new(layout),
        named_graph: Arc::default(),
        upgraded_from,
    };
    let ownership = crate::board::identity::Ownership::decode(boards, &result)?;
    Arc::make_mut(&mut result.board).ownership = ownership;
    validate_watermark(&result)?;
    Ok(Arc::new(result))
}

/// Validate allocation metadata after all current numeric identities have been resolved.
fn validate_watermark(project: &Project) -> Result<(), String> {
    if let Some(high_water) = project.numeric_id_high_water
        && (high_water > MAX_OBJECT_ID || high_water < project.highest_id())
    {
        return Err("numeric ID watermark is out of range or below a retained object ID".into());
    }
    Ok(())
}

/// Require unique positive object IDs with room for subsequent runtime revisions.
pub(crate) fn validate_id(ids: &mut BTreeSet<u64>, id: u64) -> Result<(), String> {
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
        session.apply("project.new", &json!({"name": "LED indicator", "create_board":true})).unwrap();
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
        session.apply("project.new", &json!({"name": "All parts", "create_board":true})).unwrap();
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
                ("capacitor", "C1", "C"),
                ("connector", "J1", "Input"),
                ("header-4", "J2", "Header"),
                ("led", "D1", "LED"),
                ("npn", "Q1", "NPN"),
                ("resistor", "R1", "R"),
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
        session.apply("project.new", &json!({"name": "Supply", "create_board":true})).unwrap();
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
    const FORMAT_2_FIXTURE: &[u8] = include_bytes!("../../app/tests/fixtures/two-resistors-format-2.pc");
    const CURRENT_FIXTURE: &[u8] = include_bytes!("../../app/tests/fixtures/two-resistors.pc");

    /// Format 1 and format 2 projects open through the explicit migrations, say which format
    /// they came from, and save as exactly the current fixture. An unchanged current project
    /// saves without a diff. A format 1 file can't carry later fields.
    #[test]
    fn older_formats_migrate_to_the_current_format() {
        for (bytes, version) in [(FORMAT_1_FIXTURE, 1), (FORMAT_2_FIXTURE, 2)] {
            let migrated = decode(bytes).unwrap();
            assert_eq!(migrated.upgraded_from(), Some(version));
            assert!(migrated.symbols().iter().all(|symbol| symbol.turns() == 0));
            assert_eq!(encode(migrated).unwrap(), CURRENT_FIXTURE, "format {version}");
        }
        let current = decode(CURRENT_FIXTURE).unwrap();
        assert_eq!(current.upgraded_from(), None);
        assert_eq!(encode(current).unwrap(), CURRENT_FIXTURE, "an unchanged current project saves without a diff");
        // A format 3 file keeps its authored wire geometry through the migration.
        let mut format_3: serde_json::Value = serde_json::from_slice(FORMAT_2_FIXTURE).unwrap();
        format_3["format"] = json!("pcbcraft-project/3");
        format_3["sheet"]["wires"][0]["bends"] = json!([{"x_nm": 12_700_000, "y_nm": 0}]);
        let migrated = decode(&serde_json::to_vec(&format_3).unwrap()).unwrap();
        assert_eq!(migrated.upgraded_from(), Some(3));
        let saved: serde_json::Value = serde_json::from_slice(&encode(migrated).unwrap()).unwrap();
        assert_eq!(saved["format"], "pcbcraft-project/4");
        assert_eq!(saved["sheet"]["wires"][0]["bends"], json!([{"x_nm": 12_700_000, "y_nm": 0}]));
        assert_eq!(saved["symbol_definitions"].as_array().unwrap().len(), 1);
        format_3["format"] = json!("pcbcraft-project/2");
        assert!(decode(&serde_json::to_vec(&format_3).unwrap()).unwrap_err().contains("format 2 cannot contain authored wire geometry"));

        let forged = std::str::from_utf8(FORMAT_2_FIXTURE).unwrap().replacen("\"id\":2,", "\"id\":2,\"library\":\"starter\",", 1);
        assert!(decode(forged.as_bytes()).unwrap_err().contains("unknown field `library`"), "format 2 has no library field");

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

    /// A project opens from its own saved copies: a copy that differs from the starter library,
    /// or comes from a library this build doesn't have, opens and saves back byte for byte.
    /// Placing another part with the same library, ID and revision reuses the project's copy.
    #[test]
    fn projects_open_from_their_saved_symbol_copies() {
        let mut document: serde_json::Value = serde_json::from_slice(CURRENT_FIXTURE).unwrap();
        // Save the copy at the starter library's current revision, so new placements match it.
        let current = crate::library::starter().symbol("resistor").unwrap().revision();
        document["symbol_definitions"][0]["revision"] = json!(current);
        for component in document["components"].as_array_mut().unwrap() {
            component["definition_revision"] = json!(current);
        }
        document["symbol_definitions"][0]["name"] = json!("Resistor, as saved");
        let project = decode(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(project.components().all(|component| component.definition.name == "Resistor, as saved"));
        let saved = encode(project).unwrap();
        assert!(std::str::from_utf8(&saved).unwrap().contains("Resistor, as saved"));
        assert_eq!(encode(decode(&saved).unwrap()).unwrap(), saved, "the saved copy round-trips byte for byte");

        let mut session = Session::default();
        session.open_snapshot(decode(&saved).unwrap());
        let place = json!({"definition": "resistor", "x_nm": 50_800_000, "y_nm": 0, "expected_revision": session.revision()});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let names: Vec<_> = session.project().unwrap().components().map(|component| component.definition.name.as_str()).collect();
        assert_eq!(names, ["Resistor, as saved"; 3]);
        let text = String::from_utf8(encode(session.snapshot().unwrap()).unwrap()).unwrap();
        assert_eq!(text.matches("\"format\":\"pcbcraft-symbol/1\"").count(), 1, "one saved copy for all three resistors");

        // An older saved revision stays as it is, and a new placement saves the current one beside it.
        let mut session = Session::default();
        session.open_snapshot(decode(CURRENT_FIXTURE).unwrap());
        let place = json!({"definition": "resistor", "x_nm": 50_800_000, "y_nm": 0, "expected_revision": session.revision()});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let revisions: Vec<_> = session.project().unwrap().components().map(|component| component.definition.revision()).collect();
        assert_eq!(revisions, [1, 1, current]);
        let text = String::from_utf8(encode(session.snapshot().unwrap()).unwrap()).unwrap();
        assert_eq!(text.matches("\"format\":\"pcbcraft-symbol/1\"").count(), 2, "both revisions are saved");

        document["symbol_definitions"][0]["library"] = json!("team-passives");
        for component in document["components"].as_array_mut().unwrap() {
            component["library"] = json!("team-passives");
        }
        let team = decode(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(team.components().all(|component| component.definition.library == "team-passives"));
        let team_bytes = encode(team.clone()).unwrap();
        assert_eq!(encode(decode(&team_bytes).unwrap()).unwrap(), team_bytes);

        // Closing a project frees copies only it used: nothing outlives the session and its history.
        let mut session = Session::default();
        session.open_snapshot(team);
        let place = json!({"definition": "resistor", "x_nm": 50_800_000, "y_nm": 0, "expected_revision": session.revision()});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let weak = std::sync::Arc::downgrade(&session.project().unwrap().components().next().unwrap().definition);
        drop(session);
        assert!(weak.upgrade().is_none(), "the team-passives copy is freed with the last project using it");
    }

    /// A saved copy spans lines like a library file, one per terminal, stroke and pin, so changing
    /// one stroke of it changes exactly one line of the project file.
    #[test]
    fn a_saved_copy_edit_changes_one_line() {
        let mut document: serde_json::Value = serde_json::from_slice(CURRENT_FIXTURE).unwrap();
        document["symbol_definitions"][0]["units"][0]["drawing"][0]["to_nm"] = json!([-3_810_000, 0]);
        let edited = encode(decode(&serde_json::to_vec(&document).unwrap()).unwrap()).unwrap();
        let (before, after) = (std::str::from_utf8(CURRENT_FIXTURE).unwrap(), std::str::from_utf8(&edited).unwrap());
        assert_eq!(before.lines().count(), after.lines().count());
        let changed: Vec<_> = before.lines().zip(after.lines()).filter(|(old, new)| old != new).map(|(_, new)| new.trim()).collect();
        assert_eq!(changed, [r#"{"type":"line","from_nm":[-6350000,0],"to_nm":[-3810000,0]},"#]);
    }

    /// Saved copies must match the components exactly: each component's copy is saved once,
    /// every saved copy is used, and each copy passes the symbol file rules with its library ID.
    #[test]
    fn saved_symbol_copies_are_checked() {
        let valid: serde_json::Value = serde_json::from_slice(CURRENT_FIXTURE).unwrap();
        let error = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut document = valid.clone();
            edit(&mut document);
            decode(&serde_json::to_vec(&document).unwrap()).unwrap_err()
        };
        assert!(error(&|d| d["components"][0]["definition_revision"] = json!(2)).contains("doesn't save"));
        assert!(error(&|d| d["components"][0]["library"] = json!("other")).contains("doesn't save"));
        let copies = |d: &mut serde_json::Value| d["symbol_definitions"].as_array_mut().unwrap().clone();
        assert!(
            error(&|d| {
                let copy = copies(d)[0].clone();
                d["symbol_definitions"].as_array_mut().unwrap().push(copy);
            })
            .contains("saved twice")
        );
        assert!(
            error(&|d| {
                let mut copy = copies(d)[0].clone();
                copy["id"] = json!("spare");
                d["symbol_definitions"].as_array_mut().unwrap().push(copy);
            })
            .contains("no component uses it")
        );
        assert!(error(&|d| d["symbol_definitions"][0]["terminals"] = json!([])).contains("invalid saved symbol definition"));
        assert!(error(&|d| d["symbol_definitions"][0]["footprint"] = json!("0805")).contains("unknown field"));
        assert!(
            error(&|d| {
                d["symbol_definitions"][0].as_object_mut().unwrap().remove("library");
            })
            .contains("`library`")
        );
        assert!(error(&|d| d["symbol_definitions"][0]["library"] = json!("Bad Library")).contains("library ID"));
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
        assert_eq!(pad.inspect(), json!({"board":decoded.board_target().unwrap().board,"footprint":"footprint-5", "pad":"2"}));
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
        let mut document: serde_json::Value = serde_json::from_slice(CURRENT_FIXTURE).unwrap();
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
        assert_eq!(pad.inspect(), json!({"board":migrated.board_target().unwrap().board,"footprint": "footprint-5", "pad": "2"}));
        for encoding in [Encoding::Json, Encoding::Zip] {
            let bytes = encode_file(migrated.clone(), encoding).unwrap();
            let (project, detected) = decode_diagnostic_file(&bytes).unwrap();
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
        let valid: serde_json::Value = serde_json::from_slice(CURRENT_FIXTURE).unwrap();
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
