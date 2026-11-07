//! The Git working format: an entry point and independently editable documents.

use super::*;

/// The fixed, plain-text entry point inside a project folder.
pub const ENTRY_POINT: &str = "project.pc";
/// Each JSON document is bounded independently; large projects need not fit in one legacy file.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
/// Bound total decoded documents below the packed-file limit, leaving room for ZIP metadata.
pub const MAX_DOCUMENTS_BYTES: usize = 48 * 1024 * 1024;
/// Bound directory metadata as well as payload allocation.
pub const MAX_DOCUMENTS: usize = 4096;
const PROJECT_FORMAT: &str = "pcbcraft-project/6";
const PREVIOUS_PROJECT_FORMAT: &str = "pcbcraft-project/5";
const SHEET_FORMAT: &str = "pcbcraft-sheet/1";
const HIERARCHY_FORMAT: &str = "pcbcraft-sheet/2";
const BOARD_FORMAT: &str = "pcbcraft-board/1";
const CONFIGURED_BOARD_FORMAT: &str = "pcbcraft-board/2";
const MATERIAL_BOARD_FORMAT: &str = "pcbcraft-board/3";
const ROTATED_BOARD_FORMAT: &str = "pcbcraft-board/4";
const PHYSICAL_BOARD_FORMAT: &str = "pcbcraft-board/5";
const SILKSCREEN_BOARD_FORMAT: &str = "pcbcraft-board/6";
const ASSIGNED_BOARD_FORMAT: &str = "pcbcraft-board/7";
const OUTLINE_BOARD_FORMAT: &str = "pcbcraft-board/8";

/// Relative portable paths and exact document bytes; folders carry no hash manifest.
pub type Documents = BTreeMap<String, Vec<u8>>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SheetReference {
    id: Uuid,
    name: String,
    file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectDocument {
    format: String,
    id: Uuid,
    name: String,
    sheets: Vec<SheetReference>,
    components: Vec<ComponentRecord<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetDocument {
    format: String,
    id: Uuid,
    #[serde(default, deserialize_with = "present")]
    labels: Option<Vec<crate::hierarchy::Label>>,
    #[serde(default, deserialize_with = "present")]
    subsheets: Option<Vec<crate::hierarchy::SheetSymbol>>,
    symbols: Vec<Placement<'static>>,
    junctions: Vec<Junction>,
    wires: Vec<Connection<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardDocument {
    format: String,
    #[serde(default, deserialize_with = "present")]
    setup: Option<Box<serde_json::value::RawValue>>,
    #[serde(default, deserialize_with = "present")]
    silkscreen: Option<crate::board::silkscreen::Settings>,
    #[serde(default, deserialize_with = "present")]
    assignments: Option<Vec<crate::board::assignment::Assignment>>,
    #[serde(default, deserialize_with = "present")]
    outline: Option<crate::board::outline::Outline>,
    footprints: Vec<Footprint>,
}

/// Accept only paths owned by this format. Pinned item identities must match their files on decode.
pub fn is_document_path(path: &str) -> bool {
    if path == ENTRY_POINT || path == "board.json" {
        return true;
    }
    let pieces: Vec<_> = path.split('/').collect();
    match pieces.as_slice() {
        ["sheets", name] => name.strip_suffix(".json").and_then(|name| Uuid::parse_str(name).ok()).is_some(),
        ["library", "symbols" | "footprints", library, name] => {
            let id = |text: &str| {
                (1..=64).contains(&text.len())
                    && text.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                    && !text.starts_with('-')
                    && !text.ends_with('-')
            };
            id(library)
                && name
                    .strip_suffix(".json")
                    .and_then(|name| name.rsplit_once(".r"))
                    .is_some_and(|(symbol, revision)| id(symbol) && revision.parse::<u32>().is_ok_and(|revision| revision > 0))
        }
        _ => false,
    }
}

/// Bound and validate every part before constructing a project. No partial state is returned.
/// Parse each sheet once, collecting interfaces before resolving wire endpoint ownership.
pub fn decode_documents(documents: &Documents) -> Result<Arc<Project>, String> {
    check_documents(documents)?;
    let project: ProjectDocument = parse(documents, ENTRY_POINT)?;
    if project.format != PROJECT_FORMAT && project.format != PREVIOUS_PROJECT_FORMAT {
        return Err(format!("{ENTRY_POINT}: unsupported project format {}", project.format));
    }
    if project.sheets.is_empty()
        || project.sheets.len() > crate::sheets::MAX_SHEETS
        || (project.format == PREVIOUS_PROJECT_FORMAT && project.sheets.len() != 1)
    {
        return Err(format!("{ENTRY_POINT}: invalid sheet count for {}", project.format));
    }
    let mut sheet_ids = BTreeSet::from([project.id]);
    let mut sheet_names = BTreeSet::new();
    let mut used = BTreeSet::from([ENTRY_POINT.to_owned(), "board.json".to_owned()]);
    let mut layouts = Vec::new();
    let mut hierarchy_ids = BTreeSet::new();
    let mut anchors = BTreeSet::new();
    let mut hierarchy_sheets = Vec::new();
    let mut parsed_sheets = Vec::with_capacity(project.sheets.len());
    // Read all interfaces before resolving parent-side endpoint ownership.
    for reference in &project.sheets {
        let mut sheet: SheetDocument = parse(documents, &reference.file)?;
        match (sheet.format.as_str(), sheet.labels.is_some(), sheet.subsheets.is_some()) {
            (SHEET_FORMAT, false, false) | (HIERARCHY_FORMAT, true, true) => {}
            _ => return Err(format!("{}: incompatible sheet format and hierarchy fields", reference.file)),
        }
        let mut metadata = crate::sheets::SchematicSheet::empty(reference.id, reference.name.clone());
        metadata.labels = Arc::new(sheet.labels.take().unwrap_or_default());
        metadata.subsheets = Arc::new(sheet.subsheets.take().unwrap_or_default());
        for label in metadata.labels.iter() {
            validate_id(&mut hierarchy_ids, label.id)?;
            anchors.insert(WireEndpoint::Label(label.id));
        }
        for block in metadata.subsheets.iter() {
            validate_id(&mut hierarchy_ids, block.id)?;
        }
        hierarchy_sheets.push(metadata);
        parsed_sheets.push(sheet);
    }
    let metadata = Project {
        name: project.name.clone(),
        id: project.id,
        sheets: Arc::new(hierarchy_sheets.clone()),
        sheet_index: 0,
        components: Arc::default(),
        units: Arc::default(),
        board: Arc::default(),
        upgraded_from: None,
    };
    metadata.validate_hierarchy()?;
    for sheet in metadata.sheets() {
        let projection = metadata.for_sheet(sheet.target).ok_or("missing sheet")?;
        anchors.extend(projection.hierarchy_endpoints().map(|(end, _)| end));
    }
    let mut combined = Sheet { id: project.sheets[0].id, symbols: Vec::new(), junctions: Some(Vec::new()), wires: Vec::new() };
    for (reference, sheet) in project.sheets.iter().zip(parsed_sheets) {
        let sheet_path = format!("sheets/{}.json", reference.id);
        if reference.id.is_nil()
            || !sheet_ids.insert(reference.id)
            || reference.file != sheet_path
            || reference.name.is_empty()
            || reference.name.trim() != reference.name
            || reference.name.chars().count() > MAX_NAME
            || reference.name.chars().any(char::is_control)
            || !sheet_names.insert(reference.name.to_lowercase())
        {
            return Err(format!("{ENTRY_POINT}: invalid or duplicate sheet identity, name or file"));
        }
        if !matches!(sheet.format.as_str(), SHEET_FORMAT | HIERARCHY_FORMAT) || sheet.id != reference.id {
            return Err(format!("{sheet_path}: sheet format or identity does not match {ENTRY_POINT}"));
        }
        let symbols: BTreeSet<_> = sheet.symbols.iter().map(|s| s.id).collect();
        let junctions: BTreeSet<_> = sheet.junctions.iter().map(|j| j.id).collect();
        let wires: BTreeSet<_> = sheet.wires.iter().map(|w| w.id).collect();
        let own = metadata.for_sheet(crate::SheetTarget { project: project.id, sheet: reference.id }).ok_or("missing sheet")?;
        let interface: BTreeSet<_> = own.hierarchy_endpoints().map(|(end, _)| end).collect();
        // Validate ownership before global identity/geometry validation. A wire must never
        // resolve a terminal or junction belonging to another sheet.
        for wire in &sheet.wires {
            for endpoint in [&wire.from, &wire.to] {
                let local = match endpoint {
                    Endpoint::Pin { symbol, .. } => symbols.contains(symbol),
                    Endpoint::Junction { junction } => junctions.contains(junction),
                    Endpoint::Label { label } => interface.contains(&WireEndpoint::Label(*label)),
                    Endpoint::SheetPin { sheet_pin } => interface.contains(&WireEndpoint::SheetPin(*sheet_pin)),
                };
                if !local {
                    return Err(format!("{sheet_path}: wire endpoint belongs to a missing or different sheet"));
                }
            }
        }
        used.insert(sheet_path);
        layouts.push((reference, symbols, junctions, wires));
        combined.symbols.extend(sheet.symbols);
        combined.junctions.get_or_insert_with(Vec::new).extend(sheet.junctions);
        combined.wires.extend(sheet.wires);
    }
    let board: BoardDocument = parse(documents, "board.json")?;
    if board.format != ROTATED_BOARD_FORMAT
        && board.format != PHYSICAL_BOARD_FORMAT
        && board.format != SILKSCREEN_BOARD_FORMAT
        && board.format != ASSIGNED_BOARD_FORMAT
        && board.format != OUTLINE_BOARD_FORMAT
        && board.footprints.iter().any(|f| f.rotation != 0)
    {
        return Err("board.json: footprint rotation requires board format 4".to_owned());
    }
    if board.format != PHYSICAL_BOARD_FORMAT
        && board.format != SILKSCREEN_BOARD_FORMAT
        && board.format != ASSIGNED_BOARD_FORMAT
        && board.format != OUTLINE_BOARD_FORMAT
        && board.footprints.iter().any(|f| f.library.is_some())
    {
        return Err("board.json: pinned physical footprints require board format 5".into());
    }
    if !matches!(board.format.as_str(), ASSIGNED_BOARD_FORMAT | OUTLINE_BOARD_FORMAT)
        && (board.format == SILKSCREEN_BOARD_FORMAT) != board.silkscreen.is_some()
    {
        return Err("board.json: explicit silkscreen settings require board format 6".into());
    }
    if (board.format == ASSIGNED_BOARD_FORMAT && board.assignments.is_none())
        || (!matches!(board.format.as_str(), ASSIGNED_BOARD_FORMAT | OUTLINE_BOARD_FORMAT) && board.assignments.is_some())
        || board.assignments.as_ref().is_some_and(|a| a.is_empty() || a.len() > project.components.len())
    {
        return Err("board.json: nonempty assignments require board format 7".into());
    }
    if (board.format == OUTLINE_BOARD_FORMAT) != board.outline.is_some() {
        return Err("board.json: an outline requires board format 8".into());
    }
    let setup = match (board.format.as_str(), board.setup) {
        (OUTLINE_BOARD_FORMAT, None) => None,
        (OUTLINE_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (ASSIGNED_BOARD_FORMAT, None) => None,
        (ASSIGNED_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (SILKSCREEN_BOARD_FORMAT, None) => None,
        (SILKSCREEN_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (PHYSICAL_BOARD_FORMAT, None) => None,
        (PHYSICAL_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (ROTATED_BOARD_FORMAT, None) => None,
        (ROTATED_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (BOARD_FORMAT, None) => None,
        (CONFIGURED_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, true)?),
        (MATERIAL_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        _ => return Err(format!("board.json: unsupported board format or incompatible setup: {}", board.format)),
    };
    let mut definitions = Vec::new();
    for component in &project.components {
        let path = format!("library/symbols/{}/{}.r{}.json", component.library, component.definition, component.definition_revision);
        if !is_document_path(&path) {
            return Err(format!("{ENTRY_POINT}: component {} has an invalid definition identity", component.id));
        }
        if used.insert(path.clone()) {
            let value: serde_json::Value = parse(documents, &path)?;
            let definition = crate::library::read_saved_copy(value.clone()).map_err(|error| format!("{path}: {error}"))?;
            if definition.library != component.library
                || definition.id != component.definition
                || definition.revision() != component.definition_revision
            {
                return Err(format!("{path}: saved symbol identity does not match its file name"));
            }
            definitions.push(value);
        }
    }
    let mut footprint_definitions = BTreeMap::new();
    for key in
        board.footprints.iter().filter_map(Footprint::key).chain(board.assignments.iter().flatten().map(crate::board::assignment::Assignment::key))
    {
        {
            let (library, id, revision) = &key;
            let path = format!("library/footprints/{library}/{id}.r{revision}.json");
            if !is_document_path(&path) {
                return Err(format!("board.json: invalid footprint identity {id}"));
            }
            if used.insert(path.clone()) {
                let bytes = documents.get(&path).ok_or_else(|| format!("{path}: missing pinned footprint"))?;
                let definition = crate::board::definition::read_saved(bytes).map_err(|e| format!("{path}: {e}"))?;
                if definition.key() != key {
                    return Err(format!("{path}: footprint identity does not match its file name"));
                }
                footprint_definitions.insert(key, definition);
            }
        }
    }
    if let Some(path) = documents.keys().find(|path| !used.contains(*path)) {
        return Err(format!("{path}: unexpected or unused project document"));
    }
    let previous_format = project.format == PREVIOUS_PROJECT_FORMAT;
    let document = Document {
        format: super::FORMAT.to_owned(),
        id: project.id,
        name: project.name,
        symbol_definitions: definitions,
        components: project.components,
        sheet: combined,
        board: Some(BoardRecords { footprints: board.footprints }),
    };
    let mut project = validate_with_hierarchy(document, None, footprint_definitions, &anchors, &hierarchy_ids)
        .map_err(|error| format!("{ENTRY_POINT}, sheets, board.json: {error}"))?;
    let project_data = Arc::make_mut(&mut project);
    let sheets = layouts
        .into_iter()
        .map(|(reference, symbols, junctions, wires)| {
            Ok(crate::sheets::SchematicSheet {
                sheet_id: reference.id,
                sheet_name: if previous_format && reference.name == "Schematic" { "Main".into() } else { reference.name.clone() },
                symbols: Arc::new(project_data.symbols.iter().filter(|s| symbols.contains(&s.ordinal)).cloned().collect()),
                wires: Arc::new(project_data.wires.iter().filter(|w| wires.contains(&w.ordinal)).cloned().collect()),
                junctions: Arc::new(project_data.junctions.iter().filter(|(id, _)| junctions.contains(id)).map(|(&id, &at)| (id, at)).collect()),
                labels: hierarchy_sheets.iter().find(|s| s.sheet_id == reference.id).ok_or("missing hierarchy sheet")?.labels.clone(),
                subsheets: hierarchy_sheets.iter().find(|s| s.sheet_id == reference.id).ok_or("missing hierarchy sheet")?.subsheets.clone(),
                connectivity: Arc::default(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    project_data.sheets = Arc::new(sheets);
    Arc::make_mut(&mut project_data.board).setup = setup;
    Arc::make_mut(&mut project_data.board).silkscreen = board.silkscreen;
    Arc::make_mut(&mut project_data.board).outline = board.outline;
    for assignment in board.assignments.into_iter().flatten() {
        if !assignment.valid(project_data) || project_data.board.assignments.contains_key(&assignment.component) {
            return Err("board.json: duplicate, unresolved or incompatible footprint assignment".into());
        }
        Arc::make_mut(&mut project_data.board).assignments.insert(assignment.component, assignment);
    }
    Ok(project)
}

/// Version 2 retains its original field vocabulary; migration preserves labels and dimensions
/// without inferring material definitions or construction. Version 3 uses the extended model.
fn decode_setup(raw: &serde_json::value::RawValue, legacy: bool) -> Result<crate::board::setup::BoardSetup, String> {
    if legacy {
        let value: serde_json::Value = serde_json::from_str(raw.get()).map_err(|error| format!("board.json: {error}"))?;
        legacy_fields(&value, &["layers", "stackup"])?;
        legacy_fields(&value["stackup"], &["id", "strata"])?;
        if let Some(layers) = value["layers"].as_array() {
            for layer in layers {
                legacy_fields(layer, &["id", "name", "purpose", "side"])?;
            }
        }
        if let Some(strata) = value["stackup"]["strata"].as_array() {
            for stratum in strata {
                let fields = if stratum["kind"] == "copper" {
                    &["kind", "layer", "thickness_nm"][..]
                } else {
                    &["kind", "id", "material", "thickness_nm"][..]
                };
                legacy_fields(stratum, fields)?;
            }
        }
    }
    // Decode the original JSON into typed records so duplicate keys cannot be lost in Value.
    serde_json::from_str(raw.get()).map_err(|error| format!("board.json: {error}"))
}

/// Do not accept a newer field under an older version, even if it is empty or null.
fn legacy_fields(value: &serde_json::Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("board.json: version 2 setup records must be objects")?;
    if let Some(field) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("board.json: field {field} is not supported by version 2"));
    }
    Ok(())
}

/// Reject unsafe names, excess entries and oversized input before parsing any JSON.
pub(super) fn check_documents(documents: &Documents) -> Result<(), String> {
    if documents.len() > MAX_DOCUMENTS || documents.values().map(Vec::len).sum::<usize>() > MAX_DOCUMENTS_BYTES {
        return Err("project exceeds the document count or total size limit".to_owned());
    }
    for (path, bytes) in documents {
        if !is_document_path(path) {
            return Err(format!("{path}: unexpected project path"));
        }
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(format!("{path}: document exceeds its size limit"));
        }
    }
    Ok(())
}

/// Parse a required document and keep its filename in syntax and schema errors.
fn parse<T: serde::de::DeserializeOwned>(documents: &Documents, path: &str) -> Result<T, String> {
    let bytes = documents.get(path).ok_or_else(|| format!("{path}: missing project document"))?;
    serde_json::from_slice(bytes).map_err(|error| format!("{path}: {error}"))
}

/// Encode the folder's complete logical contents; filesystem publication is a separate operation.
pub fn encode_documents(project: Arc<Project>) -> Result<Documents, String> {
    let mut encoder = DocumentsEncoder::new(project);
    loop {
        if let Some(documents) = encoder.step(256)? {
            return Ok(documents);
        }
    }
}

/// Incremental split-document writer. Each step bounds work by editable records so browser
/// downloads can yield between frames, without constructing a legacy monolithic document.
pub struct DocumentsEncoder {
    project: Arc<Project>,
    documents: Documents,
    bytes: Vec<u8>,
    phase: u8,
    sheet_cursor: usize,
    cursor: u64,
    first: bool,
    definitions: BTreeMap<(String, String, u32), Arc<crate::schematic::SymbolDefinition>>,
    footprint_definitions: BTreeMap<crate::board::definition::Key, Arc<crate::board::definition::FootprintDefinition>>,
}

impl DocumentsEncoder {
    /// Retain the immutable save snapshot without walking its contents in the input handler.
    pub fn new(project: Arc<Project>) -> Self {
        Self {
            project,
            documents: Documents::new(),
            bytes: Vec::new(),
            phase: 0,
            sheet_cursor: 0,
            cursor: 0,
            first: true,
            definitions: BTreeMap::new(),
            footprint_definitions: BTreeMap::new(),
        }
    }

    /// Write at most `records` records or phase transitions. The board-header transition also
    /// writes its bounded setup (at most 64 material revisions, 128 layers, 127 strata and
    /// 256 nested prepreg plies). Stop on completed documents.
    pub fn step(&mut self, records: usize) -> Result<Option<Documents>, String> {
        for _ in 0..records {
            match self.phase {
                0 => {
                    let p = &self.project;
                    let sheets = p
                        .sheets
                        .iter()
                        .map(|sheet| SheetReference {
                            id: sheet.sheet_id,
                            name: sheet.sheet_name.clone(),
                            file: format!("sheets/{}.json", sheet.sheet_id),
                        })
                        .collect::<Vec<_>>();
                    write!(
                        self.bytes,
                        "{{\n  \"format\": \"{PROJECT_FORMAT}\",\n  \"id\": \"{}\",\n  \"name\": {},\n  \"sheets\": {},\n  \"components\": [\n",
                        p.id,
                        json(&p.name)?,
                        json(&sheets)?
                    )
                    .map_err(|e| e.to_string())?;
                    self.next();
                }
                1 => {
                    if let Some((&id, c)) = self.project.components.range((std::ops::Bound::Excluded(self.cursor), std::ops::Bound::Unbounded)).next()
                    {
                        record(
                            &mut self.bytes,
                            &mut self.first,
                            &ComponentRecord {
                                id,
                                library: c.definition.library.as_str().into(),
                                definition: c.definition.id.as_str().into(),
                                definition_revision: c.definition.revision(),
                                reference_number: c.reference_number,
                                reference: c.reference.as_str().into(),
                                value: c.value.as_str().into(),
                            },
                        )?;
                        self.definitions
                            .insert((c.definition.library.clone(), c.definition.id.clone(), c.definition.revision()), c.definition.clone());
                        self.cursor = id;
                    } else {
                        self.finish_document(ENTRY_POINT.to_owned())?;
                        self.next();
                        self.phase = sheet_header(&mut self.bytes, &self.project.sheets[self.sheet_cursor])?;
                    }
                }
                2 => {
                    if let Some(label) = self.project.sheets[self.sheet_cursor].labels.get(self.cursor as usize) {
                        record(&mut self.bytes, &mut self.first, label)?;
                        self.cursor += 1;
                    } else {
                        self.bytes.extend_from_slice(b"  ],\n  \"subsheets\": [\n");
                        self.next();
                    }
                }
                3 => {
                    if let Some(block) = self.project.sheets[self.sheet_cursor].subsheets.get(self.cursor as usize) {
                        record(&mut self.bytes, &mut self.first, block)?;
                        self.cursor += 1;
                    } else {
                        self.bytes.extend_from_slice(b"  ],\n  \"symbols\": [\n");
                        self.next();
                    }
                }
                4 => {
                    if let Some(s) = self.project.sheets[self.sheet_cursor].symbols.get(self.cursor as usize) {
                        record(
                            &mut self.bytes,
                            &mut self.first,
                            &Placement {
                                id: s.ordinal,
                                component: s.component,
                                unit: s.unit.as_str().into(),
                                x_nm: s.position.x_nm,
                                y_nm: s.position.y_nm,
                                rotation: u16::from(s.rotation) * 90,
                            },
                        )?;
                        self.cursor += 1;
                    } else {
                        self.bytes.extend_from_slice(b"  ],\n  \"junctions\": [\n");
                        self.next();
                    }
                }
                5 => {
                    if let Some((&id, p)) = self.project.sheets[self.sheet_cursor]
                        .junctions
                        .range((std::ops::Bound::Excluded(self.cursor), std::ops::Bound::Unbounded))
                        .next()
                    {
                        record(&mut self.bytes, &mut self.first, &Junction { id, x_nm: p.x_nm, y_nm: p.y_nm })?;
                        self.cursor = id;
                    } else {
                        self.bytes.extend_from_slice(b"  ],\n  \"wires\": [\n");
                        self.next();
                    }
                }
                6 => {
                    if let Some(w) = self.project.sheets[self.sheet_cursor].wires.get(self.cursor as usize) {
                        let [from, to] = w.endpoints();
                        record(
                            &mut self.bytes,
                            &mut self.first,
                            &Connection { id: w.ordinal, from: from.into(), to: to.into(), bends: w.bends.clone() },
                        )?;
                        self.cursor += 1;
                    } else {
                        self.finish_document(format!("sheets/{}.json", self.project.sheets[self.sheet_cursor].sheet_id))?;
                        self.sheet_cursor += 1;
                        if self.sheet_cursor < self.project.sheets.len() {
                            self.cursor = 0;
                            self.first = true;
                            self.phase = sheet_header(&mut self.bytes, &self.project.sheets[self.sheet_cursor])?;
                        } else {
                            self.next();
                            board_header(
                                &mut self.bytes,
                                self.project.board.setup.as_ref(),
                                self.project.board.footprints().iter().any(|f| f.rotation != 0),
                                !self.project.board.definitions.is_empty(),
                                self.project.board.silkscreen.as_ref(),
                                !self.project.board.assignments.is_empty(),
                                self.project.board.outline.as_ref(),
                            )?;
                        }
                    }
                }
                7 => {
                    if let Some((&id, assignment)) =
                        self.project.board.assignments.range((std::ops::Bound::Excluded(self.cursor), std::ops::Bound::Unbounded)).next()
                    {
                        record(&mut self.bytes, &mut self.first, assignment)?;
                        let key = assignment.key();
                        let definition = self.project.board.definitions.get(&key).ok_or("unresolved assigned footprint during save")?;
                        self.footprint_definitions.insert(key, Arc::clone(definition));
                        self.cursor = id;
                    } else {
                        if !self.project.board.assignments.is_empty() {
                            self.bytes.extend_from_slice(b"  ],\n  \"footprints\": [\n");
                        }
                        self.next();
                    }
                }
                8 => {
                    if let Some(f) = self.project.board.footprints().get(self.cursor as usize) {
                        if let Some(key) = f.key() {
                            let definition = self.project.board.definitions.get(&key).ok_or("unresolved pinned footprint during save")?;
                            self.footprint_definitions.insert(key, Arc::clone(definition));
                        }
                        record(&mut self.bytes, &mut self.first, f)?;
                        self.cursor += 1;
                    } else {
                        self.finish_document("board.json".to_owned())?;
                        self.next();
                    }
                }
                9 => {
                    if let Some(((library, id, revision), definition)) = self.definitions.pop_first() {
                        let mut bytes = Vec::new();
                        saved_document(&mut bytes, &definition)?;
                        self.documents.insert(format!("library/symbols/{library}/{id}.r{revision}.json"), bytes);
                    } else {
                        self.next();
                    }
                }
                10 => {
                    if let Some(((library, id, revision), definition)) = self.footprint_definitions.pop_first() {
                        let mut bytes = serde_json::to_vec_pretty(definition.as_ref()).map_err(|e| e.to_string())?;
                        bytes.push(b'\n');
                        self.documents.insert(format!("library/footprints/{library}/{id}.r{revision}.json"), bytes);
                    } else {
                        check_documents(&self.documents)?;
                        self.next();
                        return Ok(Some(std::mem::take(&mut self.documents)));
                    }
                }
                _ => return Err("document encoding is already complete".to_owned()),
            }
            if self.bytes.len() > MAX_DOCUMENT_BYTES {
                return Err("encoded document exceeds its size limit".to_owned());
            }
        }
        Ok(None)
    }

    /// Advance to the next list and reset its ordering cursor and separator state.
    fn next(&mut self) {
        self.phase += 1;
        self.cursor = 0;
        self.first = true;
    }

    /// Close the last array and object and retain the exact bytes under the document's path.
    fn finish_document(&mut self, path: String) -> Result<(), String> {
        self.bytes.extend_from_slice(b"  ]\n}\n");
        if self.bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(format!("{path}: encoded document exceeds its size limit"));
        }
        self.documents.insert(path, std::mem::take(&mut self.bytes));
        Ok(())
    }
}

/// Keep legacy bytes until material fields, rotation, physical definitions or authored silk
/// require versions 3–8. Versions 7/8 add independent assignments/rectangular outlines respectively; older bytes stay unchanged.
fn board_header(
    bytes: &mut Vec<u8>,
    setup: Option<&crate::board::setup::BoardSetup>,
    rotated: bool,
    physical: bool,
    silkscreen: Option<&crate::board::silkscreen::Settings>,
    assignments: bool,
    outline: Option<&crate::board::outline::Outline>,
) -> Result<(), String> {
    if let Some(setup) = setup {
        let format = if outline.is_some() {
            OUTLINE_BOARD_FORMAT
        } else if assignments {
            ASSIGNED_BOARD_FORMAT
        } else if silkscreen.is_some() {
            SILKSCREEN_BOARD_FORMAT
        } else if physical {
            PHYSICAL_BOARD_FORMAT
        } else if rotated {
            ROTATED_BOARD_FORMAT
        } else if setup.requires_material_format() {
            MATERIAL_BOARD_FORMAT
        } else {
            CONFIGURED_BOARD_FORMAT
        };
        bytes.extend_from_slice(format!("{{\n  \"format\": \"{format}\",\n  \"setup\": {{\n").as_bytes());
        if !setup.materials().is_empty() {
            bytes.extend_from_slice(b"    \"materials\": [\n");
            let mut first = true;
            for material in setup.materials() {
                record(bytes, &mut first, material)?;
            }
            bytes.extend_from_slice(b"    ],\n");
        }
        bytes.extend_from_slice(b"    \"layers\": [\n");
        let mut first = true;
        for layer in setup.layers() {
            record(bytes, &mut first, layer)?;
        }
        write!(bytes, "    ],\n    \"stackup\": {{\n      \"id\": \"{}\",\n      \"strata\": [\n", setup.stackup().id).map_err(|e| e.to_string())?;
        first = true;
        for stratum in &setup.stackup().strata {
            record(bytes, &mut first, stratum)?;
        }
        bytes.extend_from_slice(b"      ]\n    }\n  },\n");
    } else {
        let format = if outline.is_some() {
            OUTLINE_BOARD_FORMAT
        } else if assignments {
            ASSIGNED_BOARD_FORMAT
        } else if silkscreen.is_some() {
            SILKSCREEN_BOARD_FORMAT
        } else if physical {
            PHYSICAL_BOARD_FORMAT
        } else if rotated {
            ROTATED_BOARD_FORMAT
        } else {
            BOARD_FORMAT
        };
        bytes.extend_from_slice(format!("{{\n  \"format\": \"{format}\",\n").as_bytes());
    }
    if let Some(settings) = silkscreen {
        writeln!(bytes, "  \"silkscreen\": {},", serde_json::to_string(settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    if let Some(outline) = outline {
        writeln!(bytes, "  \"outline\": {},", serde_json::to_string(outline).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    bytes.extend_from_slice(if assignments { b"  \"assignments\": [\n" } else { b"  \"footprints\": [\n" });
    Ok(())
}

/// Keep commas on their own lines so adding a record never rewrites the previous record.
fn record(bytes: &mut Vec<u8>, first: &mut bool, value: &impl Serialize) -> Result<(), String> {
    if !*first {
        bytes.extend_from_slice(b"\n    ,\n");
    }
    *first = false;
    bytes.extend_from_slice(b"    ");
    serde_json::to_writer(&mut *bytes, value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(())
}

/// Serialize a small header field without introducing unstable object ordering.
fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

/// Reuse the saved-symbol field order while making each array separator independent of its
/// previous record. Removing the legacy embedding indentation makes this a standalone file.
fn saved_document(bytes: &mut Vec<u8>, definition: &crate::schematic::SymbolDefinition) -> Result<(), String> {
    let mut embedded = Vec::new();
    crate::library::write_saved_copy(&mut embedded, definition)?;
    let text = std::str::from_utf8(&embedded).map_err(|e| e.to_string())?;
    for line in text.lines() {
        let line = line.strip_prefix("      ").unwrap_or(line);
        let record = line.strip_suffix(',').unwrap_or(line);
        bytes.extend_from_slice(record.as_bytes());
        bytes.push(b'\n');
        if line.ends_with(',') {
            let indent = &line[..line.len() - line.trim_start().len()];
            bytes.extend_from_slice(indent.as_bytes());
            bytes.extend_from_slice(b",\n");
        }
    }
    Ok(())
}

/// Write the bounded sheet header and return its first incremental record phase. Flat
/// sheets retain version 1 and bypass the version-2 label and child-symbol lists.
fn sheet_header(bytes: &mut Vec<u8>, sheet: &crate::sheets::SchematicSheet) -> Result<u8, String> {
    let hierarchy = !sheet.labels.is_empty() || !sheet.subsheets.is_empty();
    let format = if hierarchy { HIERARCHY_FORMAT } else { SHEET_FORMAT };
    write!(bytes, "{{\n  \"format\": \"{format}\",\n  \"id\": \"{}\",\n", sheet.sheet_id).map_err(|e| e.to_string())?;
    if hierarchy {
        bytes.extend_from_slice(b"  \"labels\": [\n");
        Ok(2)
    } else {
        bytes.extend_from_slice(b"  \"symbols\": [\n");
        Ok(4)
    }
}
