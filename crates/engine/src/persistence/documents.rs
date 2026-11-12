//! The Git working format: an entry point and independently editable documents.

use super::*;

/// The fixed, plain-text entry point inside a project folder.
pub const ENTRY_POINT: &str = "project.json";

/// Convert the previous entry-point name and board placement into the current folder layout.
/// Upgrade retired power markers too; unaffected board, sheet and library bytes survive.
/// Mixed layouts, standalone JSON and unsupported schemas are refused before adoption.
pub fn convert_legacy_documents(mut documents: Documents) -> Result<Documents, String> {
    if documents.contains_key(ENTRY_POINT) {
        return Err("Conversion requires the previous project.pc layout, without project.json.".to_owned());
    }
    let entry = documents.remove("project.pc").ok_or("project.pc: missing previous split entry point")?;
    documents.insert(ENTRY_POINT.to_owned(), entry);
    migrate_board_layout(super::migrate_power_markers(documents)?, ".")
}

/// Adopt board-folder and occurrence metadata, moving physical bytes without re-encoding them.
/// Validation covers both document sets; compatibility IDs depend on UUIDs, never storage paths.
pub fn migrate_board_layout(mut documents: Documents, root: &str) -> Result<Documents, String> {
    let original = decode_documents(&documents)?;
    let migrated = Arc::new(original.with_board_folders(root)?.with_sheet_occurrences()?);
    let mut canonical = encode_documents(migrated.clone())?;
    documents.insert(ENTRY_POINT.to_owned(), canonical.remove(ENTRY_POINT).ok_or("missing encoded entry point")?);
    if let (Some(before), Some(after)) = (original.board_document_path(), migrated.board_document_path())
        && before != after
    {
        let board = documents.remove(&before).ok_or("missing original board document")?;
        documents.insert(after, board);
    }
    if let Some(path) = migrated.instances_document_path() {
        documents.insert(path.clone(), canonical.remove(&path).ok_or("missing encoded instance document")?);
    }
    decode_documents(&documents)?;
    Ok(documents)
}
/// Each JSON document is bounded independently; large projects need not fit in one legacy file.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
/// Bound total decoded documents below the packed-file limit, leaving room for ZIP metadata.
pub const MAX_DOCUMENTS_BYTES: usize = 48 * 1024 * 1024;
/// Bound directory metadata as well as payload allocation.
pub const MAX_DOCUMENTS: usize = 4096;
const FOLDER_PROJECT_FORMAT: &str = "pcbcraft-project/14";
const OCCURRENCE_PROJECT_FORMAT: &str = "pcbcraft-project/16";
const INSTANCES_FORMAT: &str = "pcbcraft-instances/1";
const FIELDS_PROJECT_FORMAT: &str = "pcbcraft-project/11";
const OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT: &str = "pcbcraft-project/12";
const PART_PROJECT_FORMAT: &str = "pcbcraft-project/15";
const SCHEMATIC_PACKAGES_PROJECT_FORMAT: &str = "pcbcraft-project/13";
const TEXT_LAYOUT_FORMAT: &str = "pcbcraft-sheet/5";
const FIELDS_SHEET_FORMAT: &str = "pcbcraft-sheet/4";
const PROJECT_FORMAT: &str = "pcbcraft-project/6";
const WATERMARK_PROJECT_FORMAT: &str = "pcbcraft-project/7";
const OPTIONAL_BOARD_PROJECT_FORMAT: &str = "pcbcraft-project/9";
const PREVIOUS_PROJECT_FORMAT: &str = "pcbcraft-project/5";
const SHEET_FORMAT: &str = "pcbcraft-sheet/1";
const HIERARCHY_FORMAT: &str = "pcbcraft-sheet/2";
const NAMED_FORMAT: &str = "pcbcraft-sheet/3";
const BOARD_FORMAT: &str = "pcbcraft-board/1";
const CONFIGURED_BOARD_FORMAT: &str = "pcbcraft-board/2";
const MATERIAL_BOARD_FORMAT: &str = "pcbcraft-board/3";
const ROTATED_BOARD_FORMAT: &str = "pcbcraft-board/4";
const PHYSICAL_BOARD_FORMAT: &str = "pcbcraft-board/5";
const SILKSCREEN_BOARD_FORMAT: &str = "pcbcraft-board/6";
const ASSIGNED_BOARD_FORMAT: &str = "pcbcraft-board/7";
const OUTLINE_BOARD_FORMAT: &str = "pcbcraft-board/8";
const DRAWING_BOARD_FORMAT: &str = "pcbcraft-board/9";
const BASELINE_BOARD_FORMAT: &str = "pcbcraft-board/10";
const INTENT_BOARD_FORMAT: &str = "pcbcraft-board/11";
const TRACK_BOARD_FORMAT: &str = "pcbcraft-board/12";
const VIA_BOARD_FORMAT: &str = "pcbcraft-board/13";
const SIDE_BOARD_FORMAT: &str = "pcbcraft-board/14";
const REFERENCE_BOARD_FORMAT: &str = "pcbcraft-board/15";
const LOCKED_COPPER_BOARD_FORMAT: &str = "pcbcraft-board/18";
const MULTIPAD_BOARD_FORMAT: &str = "pcbcraft-board/17";
const STROKED_BOARD_FORMAT: &str = "pcbcraft-board/16";

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
    #[serde(default, deserialize_with = "present")]
    schematic_instances: Option<crate::instances::Instances>,
    #[serde(default, deserialize_with = "present")]
    footprint_choices: Option<Vec<crate::board::assignment::Assignment>>,
    #[serde(default, deserialize_with = "present")]
    boards: Option<Vec<BoardReference>>,
    #[serde(default, deserialize_with = "present")]
    layout_root: Option<String>,
    id: Uuid,
    name: String,
    #[serde(default, deserialize_with = "present")]
    numeric_id_high_water: Option<u64>,
    sheets: Vec<SheetReference>,
    components: Vec<ComponentRecord<'static>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstancesDocument {
    format: String,
    board: Uuid,
    circuit_uses: Vec<crate::instances::CircuitUse>,
    sheet_occurrences: Vec<crate::instances::SheetOccurrence>,
}

/// Storage-version-specific board registrations never become independent physical identities.
#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum BoardReference {
    Flat(crate::board::identity::Registration),
    Folder(FolderReference),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FolderReference {
    id: Uuid,
    name: String,
    folder: String,
}

impl ProjectDocument {
    /// Resolve only registered storage paths, refusing mixed vocabularies and unsafe roots.
    fn registry(&self) -> Result<(Option<Vec<crate::board::identity::Registration>>, Option<layout::Layout>), String> {
        let foldered = matches!(self.format.as_str(), FOLDER_PROJECT_FORMAT | OCCURRENCE_PROJECT_FORMAT)
            || (self.format == PART_PROJECT_FORMAT && self.layout_root.is_some());
        if foldered != self.layout_root.is_some() {
            return Err(format!("{ENTRY_POINT}: layout_root requires project format 14, 15 or 16"));
        }
        if foldered && self.boards.is_none() {
            return Err(format!("{ENTRY_POINT}: board-folder formats require a board registry"));
        }
        let mut folder = None;
        let records = self
            .boards
            .as_ref()
            .map(|records| {
                if records.len() > 1 {
                    return Err("this format supports zero or one physical board".to_owned());
                }
                records
                    .iter()
                    .map(|record| match record {
                        BoardReference::Flat(record) if !foldered => Ok(record.clone()),
                        BoardReference::Folder(record) if foldered && layout::portable_segment(&record.folder) => {
                            folder = Some(record.folder.clone());
                            Ok(crate::board::identity::Registration { id: record.id, name: record.name.clone(), file: "board.json".into() })
                        }
                        _ => Err(format!("{ENTRY_POINT}: incompatible board registration and project format")),
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?;
        let layout = self
            .layout_root
            .as_ref()
            .map(|root| {
                if !layout::valid_root(root) {
                    return Err(format!("{ENTRY_POINT}: invalid layout_root"));
                }
                Ok(layout::Layout { root: root.clone(), folder })
            })
            .transpose()?;
        Ok((records, layout))
    }
}

/// Resolve a stored project's sole physical path before bounded filesystem reads.
/// Full decoding still validates identities, schemas and exact document ownership.
pub fn project_board_path(entry: &[u8]) -> Result<Option<String>, String> {
    let project: ProjectDocument = serde_json::from_slice(entry).map_err(|error| format!("{ENTRY_POINT}: {error}"))?;
    let (records, layout) = project.registry()?;
    if let Some(layout) = layout {
        Ok(layout.folder.map(|folder| {
            if layout.root == "." { format!("boards/{folder}/board.json") } else { format!("{}/boards/{folder}/board.json", layout.root) }
        }))
    } else {
        Ok((!records.is_some_and(|records| records.is_empty())).then(|| "board.json".into()))
    }
}

/// Read an explicit board identity from the typed project registry without using a path as identity.
pub fn project_registered_board_id(entry: &[u8]) -> Result<Option<Uuid>, String> {
    let project: ProjectDocument = serde_json::from_slice(entry).map_err(|error| format!("{ENTRY_POINT}: {error}"))?;
    Ok(project.registry()?.0.and_then(|records| records.first().map(|record| record.id)))
}

/// Read the declared storage namespace without guessing ownership from directory contents.
pub fn project_layout_root(entry: &[u8]) -> Result<Option<String>, String> {
    let project: ProjectDocument = serde_json::from_slice(entry).map_err(|error| format!("{ENTRY_POINT}: {error}"))?;
    Ok(project.registry()?.1.map(|layout| layout.root))
}

/// Resolve only the registered current-format instance document; older layouts own no such file.
/// Schematic-only records are embedded in the entry point and have no separate managed path.
pub fn project_instances_path(entry: &[u8]) -> Result<Option<String>, String> {
    let project: ProjectDocument = serde_json::from_slice(entry).map_err(|error| format!("{ENTRY_POINT}: {error}"))?;
    if project.format != OCCURRENCE_PROJECT_FORMAT {
        return Ok(None);
    }
    Ok(project_board_path(entry)?.and_then(|path| path.strip_suffix("board.json").map(|prefix| format!("{prefix}instances.json"))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetDocument {
    format: String,
    id: Uuid,
    #[serde(default, deserialize_with = "present")]
    labels: Option<Vec<crate::hierarchy::Label>>,
    #[serde(default, deserialize_with = "present")]
    net_labels: Option<Vec<crate::net_labels::Label>>,
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
    baseline: Option<crate::board::update::Baseline>,
    #[serde(default, deserialize_with = "present")]
    setup: Option<Box<serde_json::value::RawValue>>,
    #[serde(default, deserialize_with = "present")]
    silkscreen: Option<crate::board::silkscreen::Settings>,
    #[serde(default, deserialize_with = "present")]
    assignments: Option<Vec<crate::board::assignment::Assignment>>,
    #[serde(default, deserialize_with = "present")]
    outline: Option<crate::board::outline::Outline>,
    #[serde(default, deserialize_with = "present")]
    graphics: Option<crate::board::drawing::Drawings>,
    #[serde(default, deserialize_with = "present")]
    tracks: Option<Vec<crate::board::track::Track>>,
    #[serde(default, deserialize_with = "present")]
    vias: Option<Vec<crate::board::track::Via>>,
    footprints: Vec<Footprint>,
}

/// Accept only paths owned by this format. Pinned item identities must match their files on decode.
pub fn is_document_path(path: &str) -> bool {
    if path == ENTRY_POINT || path == "board.json" || layout::is_board_path(path) || layout::is_instances_path(path) {
        return true;
    }
    let pieces: Vec<_> = path.split('/').collect();
    match pieces.as_slice() {
        ["sheets", name] => name.strip_suffix(".json").and_then(|name| Uuid::parse_str(name).ok()).is_some(),
        ["library", "symbols" | "footprints" | "parts" | "categories" | "manufacturer-parts", library, name] => {
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
    let mut project: ProjectDocument = parse(documents, ENTRY_POINT)?;
    let occurrences = project.format == OCCURRENCE_PROJECT_FORMAT;
    let part_format = matches!(project.format.as_str(), PART_PROJECT_FORMAT | OCCURRENCE_PROJECT_FORMAT);
    let foldered = project.layout_root.is_some();
    if !part_format
        && project
            .footprint_choices
            .as_ref()
            .is_some_and(|choices| choices.iter().any(|choice| crate::library::parts::mapping::extended(&choice.mapping, &choice.unconnected_pads)))
    {
        return Err("Extended package mappings require project format 15 or 16.".into());
    }
    let (mut registrations, storage_layout) = project.registry()?;
    let board_path = project_board_path(&documents[ENTRY_POINT])?;
    let registered = (part_format && project.boards.is_some())
        || matches!(
            project.format.as_str(),
            OPTIONAL_BOARD_PROJECT_FORMAT | OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT | SCHEMATIC_PACKAGES_PROJECT_FORMAT | FOLDER_PROJECT_FORMAT
        );
    let fields = part_format
        || matches!(
            project.format.as_str(),
            FIELDS_PROJECT_FORMAT | OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT | SCHEMATIC_PACKAGES_PROJECT_FORMAT | FOLDER_PROJECT_FORMAT
        );
    let schematic_packages = project.footprint_choices.is_some();
    if !matches!(
        project.format.as_str(),
        PROJECT_FORMAT
            | PREVIOUS_PROJECT_FORMAT
            | WATERMARK_PROJECT_FORMAT
            | OPTIONAL_BOARD_PROJECT_FORMAT
            | FIELDS_PROJECT_FORMAT
            | OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT
            | SCHEMATIC_PACKAGES_PROJECT_FORMAT
            | FOLDER_PROJECT_FORMAT
            | PART_PROJECT_FORMAT
            | OCCURRENCE_PROJECT_FORMAT
    ) || registered != project.boards.is_some()
    {
        return Err(format!("{ENTRY_POINT}: unsupported project format {}", project.format));
    }
    if !occurrences && project.schematic_instances.is_some() {
        return Err("schematic_instances requires project format 16".into());
    }
    if (!foldered && !part_format && (project.format == SCHEMATIC_PACKAGES_PROJECT_FORMAT) != schematic_packages)
        || (schematic_packages && !project.boards.as_ref().is_some_and(Vec::is_empty))
        || project.footprint_choices.as_ref().is_some_and(|choices| choices.is_empty() || choices.len() > MAX_SYMBOLS)
    {
        return Err("project footprint choices require format 13, 14, 15 or 16, no board and a nonempty bounded list".into());
    }
    if !part_format && project.components.iter().any(|component| component.part.is_some()) {
        return Err("Pinned part selections require project format 15 or 16.".into());
    }
    if !fields && project.components.iter().any(|component| component.metadata.is_some()) {
        return Err("component metadata requires project format 11, 12, 13, 14, 15 or 16".into());
    }
    if matches!(
        project.format.as_str(),
        WATERMARK_PROJECT_FORMAT
            | OPTIONAL_BOARD_PROJECT_FORMAT
            | FIELDS_PROJECT_FORMAT
            | OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT
            | SCHEMATIC_PACKAGES_PROJECT_FORMAT
            | FOLDER_PROJECT_FORMAT
            | PART_PROJECT_FORMAT
            | OCCURRENCE_PROJECT_FORMAT
    ) != project.numeric_id_high_water.is_some()
    {
        return Err(format!("{ENTRY_POINT}: numeric ID watermark requires project format 7, 9, 11, 12, 13, 14, 15 or 16"));
    }
    if project.sheets.is_empty()
        || project.sheets.len() > crate::sheets::MAX_SHEETS
        || (project.format == PREVIOUS_PROJECT_FORMAT && project.sheets.len() != 1)
    {
        return Err(format!("{ENTRY_POINT}: invalid sheet count for {}", project.format));
    }
    let mut sheet_ids = BTreeSet::from([project.id]);
    let mut sheet_names = BTreeSet::new();
    let mut used = BTreeSet::from([ENTRY_POINT.to_owned()]);
    let instance_records = if occurrences {
        match (project_instances_path(&documents[ENTRY_POINT])?, project.schematic_instances.clone()) {
            (Some(path), None) => {
                let instance: InstancesDocument = parse(documents, &path)?;
                let registered = registrations.as_ref().and_then(|records| records.first()).map(|record| record.id);
                if instance.format != INSTANCES_FORMAT || Some(instance.board) != registered {
                    return Err(format!("{path}: unsupported instances format or foreign board header"));
                }
                used.insert(path);
                Some(crate::instances::Instances { circuit_uses: instance.circuit_uses, sheet_occurrences: instance.sheet_occurrences })
            }
            (None, Some(records)) => Some(records),
            _ => return Err("Instance records require exactly one schematic or registered-board storage home.".into()),
        }
    } else {
        None
    };
    let mut layouts = Vec::new();
    let mut hierarchy_ids = BTreeSet::new();
    let mut anchors = BTreeSet::new();
    let mut hierarchy_sheets = Vec::new();
    let mut parsed_sheets = Vec::with_capacity(project.sheets.len());
    // Read all interfaces before resolving parent-side endpoint ownership.
    for reference in &project.sheets {
        let mut sheet: SheetDocument = parse(documents, &reference.file)?;
        match (sheet.format.as_str(), sheet.labels.is_some(), sheet.subsheets.is_some(), sheet.net_labels.is_some()) {
            (SHEET_FORMAT, false, false, false)
            | (HIERARCHY_FORMAT, true, true, false)
            | (NAMED_FORMAT, true, true, true)
            | (FIELDS_SHEET_FORMAT, true, true, true)
            | (TEXT_LAYOUT_FORMAT, true, true, true) => {}
            _ => return Err(format!("{}: incompatible sheet format and hierarchy fields", reference.file)),
        }
        if !matches!(sheet.format.as_str(), FIELDS_SHEET_FORMAT | TEXT_LAYOUT_FORMAT) && sheet.symbols.iter().any(|s| s.field_layout.is_some()) {
            return Err(format!("{}: field appearance requires sheet format 4", reference.file));
        }
        if sheet.format != TEXT_LAYOUT_FORMAT && sheet.net_labels.as_ref().is_some_and(|labels| labels.iter().any(|l| l.text.is_some())) {
            return Err(format!("{}: net-name appearance requires sheet format 5", reference.file));
        }
        let mut metadata = crate::sheets::SchematicSheet::empty(reference.id, reference.name.clone());
        let mut named = sheet.net_labels.take().unwrap_or_default();
        named.sort_by_key(|label| label.id);
        metadata.net_labels = Arc::new(named);
        for label in metadata.net_labels.iter() {
            validate_id(&mut hierarchy_ids, label.id)?;
            anchors.insert(WireEndpoint::NetLabel(label.id));
        }
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
        numeric_id_high_water: None,
        sheets: Arc::new(hierarchy_sheets.clone()),
        sheet_index: 0,
        components: Arc::default(),
        units: Arc::default(),
        schematic_packages: Arc::default(),
        board: Arc::default(),
        named_graph: Arc::default(),
        upgraded_from: None,
        layout: None,
        circuit_use: None,
    };
    metadata.validate_hierarchy()?;
    metadata.validate_net_labels()?;
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
        if !matches!(sheet.format.as_str(), SHEET_FORMAT | HIERARCHY_FORMAT | NAMED_FORMAT | FIELDS_SHEET_FORMAT | TEXT_LAYOUT_FORMAT)
            || sheet.id != reference.id
        {
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
                    Endpoint::NetLabel { net_label } => interface.contains(&WireEndpoint::NetLabel(*net_label)),
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
    // Older typed board addresses could equal a sheet UUID. Folder migration preserves that
    // exact compatibility identity and ordinary legacy name rather than inventing a new ID.
    if foldered
        && registrations.as_ref().is_some_and(|records| {
            records.len() == 1
                && records[0].id_collides(&metadata)
                && records[0].name == "Board"
                && metadata.board_target().is_some_and(|target| target.board == records[0].id)
        })
    {
        registrations = None;
    }
    let ownership = crate::board::identity::Ownership::decode(registrations.clone(), &metadata)?;
    let has_board = !matches!(ownership, crate::board::identity::Ownership::Explicit(None));
    let board: BoardDocument = if has_board {
        let path = board_path.as_deref().ok_or("missing registered board document path")?;
        used.insert(path.to_owned());
        parse(documents, path)?
    } else {
        BoardDocument {
            format: BOARD_FORMAT.into(),
            setup: None,
            silkscreen: None,
            assignments: None,
            outline: None,
            graphics: None,
            tracks: None,
            vias: None,
            baseline: None,
            footprints: vec![],
        }
    };

    // Format 12 carries tracks only, as first released; format 13 adds vias, with tracks optional;
    // format 14 adds bottom-side footprints, and format 15 adds reference placement and print overrides. Each may carry every
    // earlier field. Empty arrays are never written, so they are refused.
    let locked = board.format == LOCKED_COPPER_BOARD_FORMAT;
    let authored_locks = board.tracks.as_ref().is_some_and(|tracks| tracks.iter().any(|track| track.locked))
        || board.vias.as_ref().is_some_and(|vias| vias.iter().any(|via| via.locked));
    if locked != authored_locks {
        return Err("board.json: locked copper requires board format 18, which requires an authored lock.".into());
    }
    let multipad = board.format == MULTIPAD_BOARD_FORMAT;
    let extended = board.footprints.iter().any(|footprint| {
        footprint.mapping.as_ref().is_some_and(|mapping| crate::library::parts::mapping::extended(mapping, &footprint.unconnected_pads))
    }) || board.assignments.as_ref().is_some_and(|assignments| {
        assignments.iter().any(|assignment| crate::library::parts::mapping::extended(&assignment.mapping, &assignment.unconnected_pads))
    }) || board
        .baseline
        .as_ref()
        .is_some_and(|baseline| baseline.components.values().any(|component| component.package.as_ref().is_some_and(|package| package.extended())));
    if !locked && multipad != extended {
        return Err("board.json: multi-pad/unconnected-pad mappings require board format 17, which requires such a mapping.".into());
    }
    let stroked = board.format == STROKED_BOARD_FORMAT;
    let physical_graphics = board.graphics.as_ref().is_some_and(|data| data.items.iter().any(|item| item.stroke_nm.is_some()));
    if !locked && !multipad && stroked != physical_graphics {
        return Err("board.json: physical graphic strokes require format 16, which requires at least one".into());
    }
    let sided = board.format == SIDE_BOARD_FORMAT;
    let references = board.format == REFERENCE_BOARD_FORMAT;
    let tracked = matches!(
        board.format.as_str(),
        TRACK_BOARD_FORMAT
            | VIA_BOARD_FORMAT
            | SIDE_BOARD_FORMAT
            | REFERENCE_BOARD_FORMAT
            | STROKED_BOARD_FORMAT
            | MULTIPAD_BOARD_FORMAT
            | LOCKED_COPPER_BOARD_FORMAT
    );
    if board.tracks.as_ref().is_some_and(Vec::is_empty) || board.vias.as_ref().is_some_and(Vec::is_empty) {
        return Err("board.json: empty tracks or vias arrays are not allowed".into());
    }
    if (board.format == VIA_BOARD_FORMAT && board.vias.is_none())
        || (board.vias.is_some()
            && !matches!(
                board.format.as_str(),
                VIA_BOARD_FORMAT
                    | SIDE_BOARD_FORMAT
                    | REFERENCE_BOARD_FORMAT
                    | STROKED_BOARD_FORMAT
                    | MULTIPAD_BOARD_FORMAT
                    | LOCKED_COPPER_BOARD_FORMAT
            ))
    {
        return Err("board.json: vias require board format 13 or later, and format 13 requires at least one via".into());
    }
    if (board.format == TRACK_BOARD_FORMAT && board.tracks.is_none()) || (!tracked && board.tracks.is_some()) {
        return Err("board.json: tracks require board format 12 or later, and format 12 requires at least one track".into());
    }
    if !locked && !multipad && !references && !stroked && sided != board.footprints.iter().any(|footprint| footprint.bottom) {
        return Err("board.json: bottom-side footprints require board format 14 or later, and format 14 requires at least one".into());
    }
    if !locked
        && !multipad
        && !stroked
        && references != board.footprints.iter().any(|footprint| footprint.reference_offset.is_some() || footprint.reference_text.is_some())
    {
        return Err("board.json: footprint reference offsets and print overrides require board format 15, which requires at least one".into());
    }
    if !tracked && matches!(board.format.as_str(), BASELINE_BOARD_FORMAT | INTENT_BOARD_FORMAT) != board.baseline.is_some() {
        return Err("board.json: accepted baselines require board format 10, 11 or 12".into());
    }
    if let Some(baseline) = &board.baseline {
        baseline.validate()?;
        if baseline.components.values().any(|component| component.package.is_none()) {
            return Err("board.json: accepted baseline contains an unassigned package".into());
        }
        if baseline.project != project.id {
            return Err("board.json: accepted baseline has a foreign project owner".into());
        }
    }
    // Apply the same physical-membership rule as the encoder before constructing
    // Project. Excluded choices remain validated authored intent below, but do not
    // require the pending-physical-package format guard.
    let eligible: BTreeSet<_> = project
        .components
        .iter()
        .filter(|component| !component.metadata.as_ref().is_some_and(|metadata| metadata.attributes.exclude_from_board))
        .map(|component| component.id)
        .collect();
    let pending_intent = board.baseline.as_ref().is_some_and(|baseline| {
        board.assignments.iter().flatten().any(|assignment| eligible.contains(&assignment.component) && !baseline.validate_assignment(assignment))
    });
    if !tracked && (board.format == INTENT_BOARD_FORMAT) != pending_intent {
        return Err("board.json: pending desired packages require board format 11".into());
    }
    if board.format != ROTATED_BOARD_FORMAT
        && board.format != PHYSICAL_BOARD_FORMAT
        && board.format != SILKSCREEN_BOARD_FORMAT
        && board.format != ASSIGNED_BOARD_FORMAT
        && board.format != OUTLINE_BOARD_FORMAT
        && board.format != DRAWING_BOARD_FORMAT
        && board.format != BASELINE_BOARD_FORMAT
        && board.format != INTENT_BOARD_FORMAT
        && !tracked
        && board.footprints.iter().any(|f| f.rotation != 0)
    {
        return Err("board.json: footprint rotation requires board format 4".to_owned());
    }
    if board.format != PHYSICAL_BOARD_FORMAT
        && board.format != SILKSCREEN_BOARD_FORMAT
        && board.format != ASSIGNED_BOARD_FORMAT
        && board.format != OUTLINE_BOARD_FORMAT
        && board.format != DRAWING_BOARD_FORMAT
        && board.format != BASELINE_BOARD_FORMAT
        && board.format != INTENT_BOARD_FORMAT
        && !tracked
        && board.footprints.iter().any(|f| f.library.is_some())
    {
        return Err("board.json: pinned physical footprints require board format 5".into());
    }
    if !matches!(
        board.format.as_str(),
        ASSIGNED_BOARD_FORMAT
            | OUTLINE_BOARD_FORMAT
            | DRAWING_BOARD_FORMAT
            | BASELINE_BOARD_FORMAT
            | INTENT_BOARD_FORMAT
            | TRACK_BOARD_FORMAT
            | VIA_BOARD_FORMAT
            | SIDE_BOARD_FORMAT
            | REFERENCE_BOARD_FORMAT
            | STROKED_BOARD_FORMAT
            | MULTIPAD_BOARD_FORMAT
            | LOCKED_COPPER_BOARD_FORMAT
    ) && (board.format == SILKSCREEN_BOARD_FORMAT) != board.silkscreen.is_some()
    {
        return Err("board.json: explicit silkscreen settings require board format 6".into());
    }
    if (board.format == ASSIGNED_BOARD_FORMAT && board.assignments.is_none())
        || (!matches!(
            board.format.as_str(),
            ASSIGNED_BOARD_FORMAT
                | OUTLINE_BOARD_FORMAT
                | DRAWING_BOARD_FORMAT
                | BASELINE_BOARD_FORMAT
                | INTENT_BOARD_FORMAT
                | TRACK_BOARD_FORMAT
                | VIA_BOARD_FORMAT
                | SIDE_BOARD_FORMAT
                | REFERENCE_BOARD_FORMAT
                | STROKED_BOARD_FORMAT
                | MULTIPAD_BOARD_FORMAT
                | LOCKED_COPPER_BOARD_FORMAT
        ) && board.assignments.is_some())
        || board.assignments.as_ref().is_some_and(|a| a.is_empty() || a.len() > MAX_SYMBOLS)
    {
        return Err("board.json: nonempty assignments require board format 7".into());
    }
    if !tracked
        && !matches!(board.format.as_str(), BASELINE_BOARD_FORMAT | INTENT_BOARD_FORMAT)
        && (board.format == OUTLINE_BOARD_FORMAT) != board.outline.is_some()
    {
        return Err("board.json: an outline requires board format 8".into());
    }
    if !tracked
        && !matches!(board.format.as_str(), BASELINE_BOARD_FORMAT | INTENT_BOARD_FORMAT)
        && (board.format == DRAWING_BOARD_FORMAT) != board.graphics.is_some()
    {
        return Err("board.json: layer graphics require board format 9".into());
    }
    let setup = match (board.format.as_str(), board.setup) {
        (LOCKED_COPPER_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (LOCKED_COPPER_BOARD_FORMAT, None) => None,
        (STROKED_BOARD_FORMAT, None) => None,
        (MULTIPAD_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (MULTIPAD_BOARD_FORMAT, None) => None,
        (STROKED_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (REFERENCE_BOARD_FORMAT, None) => None,
        (REFERENCE_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (SIDE_BOARD_FORMAT, None) => None,
        (SIDE_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (VIA_BOARD_FORMAT, None) => None,
        (VIA_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (TRACK_BOARD_FORMAT, None) => None,
        (TRACK_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (INTENT_BOARD_FORMAT, None) => None,
        (INTENT_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (BASELINE_BOARD_FORMAT, None) => None,
        (BASELINE_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
        (DRAWING_BOARD_FORMAT, None) => None,
        (DRAWING_BOARD_FORMAT, Some(value)) => Some(decode_setup(&value, false)?),
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
    let mut symbol_copies = BTreeMap::new();
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
            symbol_copies.insert((definition.library.clone(), definition.id.clone(), definition.revision()), definition);
            definitions.push(value);
        }
    }
    let mut footprint_definitions = BTreeMap::new();
    for key in board
        .footprints
        .iter()
        .filter_map(Footprint::key)
        .chain(board.assignments.iter().flatten().map(crate::board::assignment::Assignment::key))
        .chain(project.footprint_choices.iter().flatten().map(crate::board::assignment::Assignment::key))
        .chain(board.baseline.iter().flat_map(crate::board::update::Baseline::package_keys))
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
    let mut reader = crate::part_instances::storage::Reader::default();
    let mut selections = BTreeMap::new();
    for component in &mut project.components {
        if let Some(record) = component.part.take() {
            let symbol = symbol_copies
                .get(&(component.library.to_string(), component.definition.to_string(), component.definition_revision))
                .cloned()
                .ok_or("Part has no pinned symbol.")?;
            selections.insert(component.id, reader.selection(record, symbol, documents, &mut used)?);
        }
    }
    if let Some(path) = documents.keys().find(|path| !used.contains(*path)) {
        return Err(format!("{path}: unexpected or unused project document"));
    }
    let previous_format = project.format == PREVIOUS_PROJECT_FORMAT;
    let numeric_id_high_water = project.numeric_id_high_water;
    let footprint_choices = project.footprint_choices;
    let document = Document {
        format: if registrations.is_some() { super::OPTIONAL_BOARD_FORMAT } else { super::FORMAT }.to_owned(),
        boards: registrations.clone(),
        id: project.id,
        name: project.name,
        numeric_id_high_water: if registrations.is_some() { numeric_id_high_water } else { None },
        symbol_definitions: definitions,
        components: project.components,
        sheet: combined,
        board: has_board.then_some(BoardRecords { baseline: board.baseline, footprints: board.footprints }),
    };
    let mut project = validate_with_hierarchy(document, None, footprint_definitions, &anchors, &hierarchy_ids)
        .map_err(|error| format!("{ENTRY_POINT}, sheets, board.json: {error}"))?;
    let project_data = Arc::make_mut(&mut project);
    if schematic_packages || project_data.board_target().is_none() {
        Arc::make_mut(&mut project_data.schematic_packages).definitions = std::mem::take(&mut Arc::make_mut(&mut project_data.board).definitions);
    }
    project_data.numeric_id_high_water = numeric_id_high_water;
    project_data.layout = storage_layout;
    let sheets = layouts
        .into_iter()
        .map(|(reference, symbols, junctions, wires)| {
            Ok(crate::sheets::SchematicSheet {
                sheet_id: reference.id,
                occurrence_id: None,
                sheet_name: if previous_format && reference.name == "Schematic" { "Main".into() } else { reference.name.clone() },
                symbols: Arc::new(project_data.symbols.iter().filter(|s| symbols.contains(&s.ordinal)).cloned().collect()),
                wires: Arc::new(project_data.wires.iter().filter(|w| wires.contains(&w.ordinal)).cloned().collect()),
                junctions: Arc::new(project_data.junctions.iter().filter(|(id, _)| junctions.contains(id)).map(|(&id, &at)| (id, at)).collect()),
                net_labels: hierarchy_sheets.iter().find(|s| s.sheet_id == reference.id).ok_or("missing named label sheet")?.net_labels.clone(),
                labels: hierarchy_sheets.iter().find(|s| s.sheet_id == reference.id).ok_or("missing hierarchy sheet")?.labels.clone(),
                subsheets: hierarchy_sheets.iter().find(|s| s.sheet_id == reference.id).ok_or("missing hierarchy sheet")?.subsheets.clone(),
                connectivity: Arc::default(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    project_data.sheets = Arc::new(sheets);
    for (id, selection) in selections {
        let component = Arc::make_mut(&mut project_data.components).get_mut(&id).ok_or("Missing part instance.")?;
        component.part = Some(selection);
    }
    if let Some(baseline) = &project_data.board.baseline {
        if !project_data.boards().any(|board| board.target.board == baseline.board) {
            return Err("board.json: accepted baseline has a foreign board owner".into());
        }
        baseline.validate_assets(project_data)?;
    }
    Arc::make_mut(&mut project_data.board).setup = setup;
    Arc::make_mut(&mut project_data.board).silkscreen = board.silkscreen;
    Arc::make_mut(&mut project_data.board).outline = board.outline;
    for assignment in board.assignments.into_iter().flatten() {
        if !assignment.valid(project_data) || project_data.board.assignments.contains_key(&assignment.component) {
            return Err("board.json: duplicate, unresolved or incompatible footprint assignment".into());
        }
        Arc::make_mut(&mut project_data.board).assignments.insert(assignment.component, assignment);
    }
    for assignment in footprint_choices.into_iter().flatten() {
        if !assignment.valid(project_data) || project_data.schematic_packages.assignments.contains_key(&assignment.component) {
            return Err("project.json: duplicate, unresolved or incompatible footprint choice".into());
        }
        Arc::make_mut(&mut project_data.schematic_packages).assignments.insert(assignment.component, assignment);
    }
    if let Some(mut graphics) = board.graphics {
        graphics.validate(project_data).map_err(|error| format!("board.json: {error}"))?;
        Arc::make_mut(&mut project_data.board).drawings = Some(graphics);
    }
    if tracked {
        let mut tracks = board.tracks.unwrap_or_default();
        let mut vias = board.vias.unwrap_or_default();
        crate::board::track::validate(project_data, &mut tracks, &mut vias).map_err(|error| format!("board.json: {error}"))?;
        let routed = Arc::make_mut(&mut project_data.board);
        routed.tracks = tracks;
        routed.vias = vias;
    }
    crate::part_instances::storage::validate_project(project_data)?;
    if let Some(records) = instance_records {
        project_data.install_sheet_instances(records)?;
    }
    validate_watermark(&project)?;
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
    instance_records: Option<crate::instances::Instances>,
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
            instance_records: None,
        }
    }

    /// Write at most `records` records or phase transitions. The board-header transition also
    /// writes its bounded setup (at most 64 material revisions, 128 layers, 127 strata and
    /// 256 nested prepreg plies) and an enabled baseline within its 16 MiB bound.
    /// Stop on completed documents.
    pub fn step(&mut self, records: usize) -> Result<Option<Documents>, String> {
        for _ in 0..records {
            match self.phase {
                0 => {
                    let p = &self.project;
                    crate::part_instances::storage::validate_project(p)?;
                    if p.has_sheet_occurrences() {
                        let records = p.sheet_instances();
                        Project::clone(p).install_sheet_instances(records.clone())?;
                        self.instance_records = Some(records);
                        if p.layout.is_none() {
                            return Err("Occurrence records require the current project folder layout.".into());
                        }
                    }
                    let format = if p.numeric_id_high_water.is_some() { WATERMARK_PROJECT_FORMAT } else { PROJECT_FORMAT };
                    let sheets = p
                        .sheets
                        .iter()
                        .map(|sheet| SheetReference {
                            id: sheet.sheet_id,
                            name: sheet.sheet_name.clone(),
                            file: format!("sheets/{}.json", sheet.sheet_id),
                        })
                        .collect::<Vec<_>>();
                    let registrations = p.board.ownership.records();
                    let has_metadata = p.components.values().any(|component| !component.metadata.is_empty());
                    let has_packages = !p.schematic_packages.assignments.is_empty();
                    if has_packages && (p.board_target().is_some() || p.numeric_id_high_water.is_none()) {
                        return Err("schematic package choices require no board and an allocation watermark".into());
                    }
                    let has_parts = p.components.values().any(|part| part.part.is_some());
                    let extended_choices = p
                        .schematic_packages
                        .assignments
                        .values()
                        .any(|choice| crate::library::parts::mapping::extended(&choice.mapping, &choice.unconnected_pads));
                    let format = if p.has_sheet_occurrences() {
                        OCCURRENCE_PROJECT_FORMAT
                    } else if has_parts || extended_choices {
                        PART_PROJECT_FORMAT
                    } else if p.layout.is_some() {
                        FOLDER_PROJECT_FORMAT
                    } else if has_packages {
                        SCHEMATIC_PACKAGES_PROJECT_FORMAT
                    } else {
                        match (registrations.is_some(), has_metadata) {
                            (true, true) => OPTIONAL_BOARD_FIELDS_PROJECT_FORMAT,
                            (true, false) => OPTIONAL_BOARD_PROJECT_FORMAT,
                            (false, true) => FIELDS_PROJECT_FORMAT,
                            (false, false) => format,
                        }
                    };
                    if has_metadata && p.numeric_id_high_water.is_none() {
                        return Err("component metadata requires an adopted numeric ID watermark".into());
                    }
                    write!(self.bytes, "{{\n  \"format\": \"{format}\",\n  \"id\": \"{}\",\n  \"name\": {},\n", p.id, json(&p.name)?)
                        .map_err(|e| e.to_string())?;
                    if let Some(high_water) = p.numeric_id_high_water {
                        writeln!(self.bytes, "  \"numeric_id_high_water\": {high_water},").map_err(|e| e.to_string())?;
                    }
                    if let Some(layout) = &p.layout {
                        let records = p
                            .boards()
                            .map(|board| {
                                Ok(FolderReference {
                                    id: board.target.board,
                                    name: board.name().into(),
                                    folder: layout.folder.clone().ok_or("missing board folder")?,
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        writeln!(self.bytes, "  \"layout_root\": {},", json(&layout.root)?).map_err(|e| e.to_string())?;
                        writeln!(self.bytes, "  \"boards\": {},", json(&records)?).map_err(|e| e.to_string())?;
                    } else if let Some(registrations) = registrations {
                        writeln!(self.bytes, "  \"boards\": {},", json(&registrations)?).map_err(|e| e.to_string())?;
                    }
                    write!(self.bytes, "  \"sheets\": {},\n  \"components\": [\n", json(&sheets)?).map_err(|e| e.to_string())?;
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
                                metadata: (!c.metadata.is_empty()).then(|| c.metadata.clone()),
                                part: c.part.as_ref().map(|part| part.record()),
                            },
                        )?;
                        if let Some(part) = &c.part {
                            crate::part_instances::storage::collect(part, &mut self.documents)?;
                        }
                        self.definitions
                            .insert((c.definition.library.clone(), c.definition.id.clone(), c.definition.revision()), c.definition.clone());
                        self.cursor = id;
                    } else {
                        if self.project.schematic_packages.assignments.is_empty() {
                            self.finish_project_document()?;
                        } else {
                            self.bytes.extend_from_slice(b"  ],\n  \"footprint_choices\": [\n");
                            self.cursor = 0;
                            self.first = true;
                            self.phase = 14;
                        }
                    }
                }
                14 => {
                    if let Some((&id, assignment)) =
                        self.project.schematic_packages.assignments.range((std::ops::Bound::Excluded(self.cursor), std::ops::Bound::Unbounded)).next()
                    {
                        record(&mut self.bytes, &mut self.first, assignment)?;
                        let key = assignment.key();
                        let definition =
                            self.project.schematic_packages.definitions.get(&key).ok_or("unresolved schematic footprint choice during save")?;
                        self.footprint_definitions.insert(key, Arc::clone(definition));
                        self.cursor = id;
                    } else {
                        self.finish_project_document()?;
                    }
                }
                15 => {
                    let records = self.instance_records.as_ref().ok_or("missing instance encoding state")?;
                    if let Some(occurrence) = records.sheet_occurrences.get(self.cursor as usize) {
                        record(&mut self.bytes, &mut self.first, occurrence)?;
                        self.cursor += 1;
                    } else {
                        let board_path = self.project.instances_document_path();
                        self.bytes.extend_from_slice(if board_path.is_some() { b"  ]\n}\n" } else { b"    ]\n  }\n}\n" });
                        if self.bytes.len() > MAX_DOCUMENT_BYTES {
                            return Err("encoded instance document exceeds its size limit".into());
                        }
                        self.documents.insert(board_path.unwrap_or_else(|| ENTRY_POINT.into()), std::mem::take(&mut self.bytes));
                        self.next();
                        self.phase = sheet_header(&mut self.bytes, &self.project.sheets[self.sheet_cursor])?;
                    }
                }
                12 => {
                    if let Some(label) = self.project.sheets[self.sheet_cursor].net_labels.get(self.cursor as usize) {
                        record(&mut self.bytes, &mut self.first, label)?;
                        self.cursor += 1;
                    } else {
                        self.bytes.extend_from_slice(b"  ],\n  \"labels\": [\n");
                        self.cursor = 0;
                        self.first = true;
                        self.phase = 2;
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
                                field_layout: (!s.field_layout.is_empty()).then(|| s.field_layout.clone()),
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
                        } else if self.project.board_target().is_some() {
                            self.next();
                            board_header(&mut self.bytes, &self.project)?;
                        } else {
                            self.phase = 9;
                            self.cursor = 0;
                            self.first = true;
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
                        self.finish_document(self.project.board_document_path().ok_or("missing board document path")?)?;
                        self.next();
                    }
                }
                9 => {
                    let next =
                        self.project.board.baseline.as_ref().and_then(|baseline| {
                            baseline.components.range((std::ops::Bound::Excluded(self.cursor), std::ops::Bound::Unbounded)).next()
                        });
                    if let Some((&id, component)) = next {
                        if let Some(package) = &component.package
                            && let Some(library) = &package.library
                        {
                            let key = (library.clone(), package.definition.clone(), package.revision);
                            let definition = self.project.board.definitions.get(&key).ok_or("unresolved accepted package during save")?;
                            self.footprint_definitions.insert(key, Arc::clone(definition));
                        }
                        self.cursor = id;
                    } else {
                        self.next();
                    }
                }
                10 => {
                    if let Some(((library, id, revision), definition)) = self.definitions.pop_first() {
                        let mut bytes = Vec::new();
                        saved_document(&mut bytes, &definition)?;
                        self.documents.insert(format!("library/symbols/{library}/{id}.r{revision}.json"), bytes);
                    } else {
                        self.next();
                    }
                }
                11 => {
                    if let Some(((library, id, revision), definition)) = self.footprint_definitions.pop_first() {
                        let mut bytes = serde_json::to_vec_pretty(definition.as_ref()).map_err(|e| e.to_string())?;
                        bytes.push(b'\n');
                        crate::part_instances::storage::insert(
                            &mut self.documents,
                            format!("library/footprints/{library}/{id}.r{revision}.json"),
                            bytes,
                        )?;
                    } else {
                        check_documents(&self.documents)?;
                        self.phase = 13;
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

    /// Finish components/package choices, writing adopted instances in their single storage home
    /// before starting sheets. Instance records consume the ordinary incremental record budget.
    fn finish_project_document(&mut self) -> Result<(), String> {
        if self.instance_records.is_some() {
            if self.project.board_target().is_some() {
                self.finish_document(ENTRY_POINT.to_owned())?;
                let board = self.project.board_target().ok_or("missing registered board")?.board;
                write!(self.bytes, "{{\n  \"format\": \"{INSTANCES_FORMAT}\",\n  \"board\": \"{board}\",\n").map_err(|error| error.to_string())?;
            } else {
                self.bytes.extend_from_slice(b"  ],\n  \"schematic_instances\": {\n");
            }
            let records = self.instance_records.as_ref().ok_or("missing instance encoding state")?;
            write!(self.bytes, "  \"circuit_uses\": {},\n  \"sheet_occurrences\": [\n", json(&records.circuit_uses)?)
                .map_err(|error| error.to_string())?;
            self.cursor = 0;
            self.first = true;
            self.phase = 15;
            return Ok(());
        }
        self.finish_document(ENTRY_POINT.to_owned())?;
        self.next();
        self.phase = sheet_header(&mut self.bytes, &self.project.sheets[self.sheet_cursor])?;
        Ok(())
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

/// Keep legacy bytes until newer authored data requires a newer format. Locked copper
/// uses format 18 only while at least one authored lock exists. Independent
/// assignments use version 7, earlier local rectangles version 8, layer graphics
/// version 9, explicitly enabled accepted baselines version 10, pending
/// desired package choices version 11, copper tracks version 12, vias version 13,
/// bottom-side footprints version 14, and reference placement/print overrides version 15. Versions 12 to 15 are supersets: each may carry any
/// earlier field, so routing or flipping never drops baseline or intent data. Version 12 stays
/// tracks-only and 13 has no bottom-side parts, so their released readers keep working.
fn board_header(bytes: &mut Vec<u8>, project: &Project) -> Result<(), String> {
    let board = &project.board;
    let setup = board.setup.as_ref();
    let silkscreen = board.silkscreen.as_ref();
    let outline = board.outline.as_ref();
    let assignments = !board.assignments.is_empty();
    let rotated = board.footprints().iter().any(|footprint| footprint.rotation != 0);
    let physical = !board.definitions.is_empty();
    let references = board.footprints().iter().any(|footprint| footprint.reference_offset.is_some() || footprint.reference_text.is_some());
    let extended = board.footprints().iter().any(|footprint| {
        footprint.mapping.as_ref().is_some_and(|mapping| crate::library::parts::mapping::extended(mapping, &footprint.unconnected_pads))
    }) || board
        .assignments
        .values()
        .any(|assignment| crate::library::parts::mapping::extended(&assignment.mapping, &assignment.unconnected_pads))
        || board.baseline.as_ref().is_some_and(|baseline| {
            baseline.components.values().any(|component| component.package.as_ref().is_some_and(|package| package.extended()))
        });

    if let Some(setup) = setup {
        let format = if board.tracks.iter().any(|track| track.locked) || board.vias.iter().any(|via| via.locked) {
            LOCKED_COPPER_BOARD_FORMAT
        } else if extended {
            MULTIPAD_BOARD_FORMAT
        } else if board.drawings.as_ref().is_some_and(|data| data.items.iter().any(|item| item.stroke_nm.is_some())) {
            STROKED_BOARD_FORMAT
        } else if references {
            REFERENCE_BOARD_FORMAT
        } else if board.footprints().iter().any(|footprint| footprint.bottom) {
            SIDE_BOARD_FORMAT
        } else if !board.vias.is_empty() {
            VIA_BOARD_FORMAT
        } else if !board.tracks.is_empty() {
            TRACK_BOARD_FORMAT
        } else if board.has_pending_package_intent(project) {
            INTENT_BOARD_FORMAT
        } else if board.baseline.is_some() {
            BASELINE_BOARD_FORMAT
        } else if board.drawings.is_some() {
            DRAWING_BOARD_FORMAT
        } else if outline.is_some() {
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
        let format = if board.tracks.iter().any(|track| track.locked) || board.vias.iter().any(|via| via.locked) {
            LOCKED_COPPER_BOARD_FORMAT
        } else if extended {
            MULTIPAD_BOARD_FORMAT
        } else if board.drawings.as_ref().is_some_and(|data| data.items.iter().any(|item| item.stroke_nm.is_some())) {
            STROKED_BOARD_FORMAT
        } else if references {
            REFERENCE_BOARD_FORMAT
        } else if board.footprints().iter().any(|footprint| footprint.bottom) {
            SIDE_BOARD_FORMAT
        } else if !board.vias.is_empty() {
            VIA_BOARD_FORMAT
        } else if !board.tracks.is_empty() {
            TRACK_BOARD_FORMAT
        } else if board.has_pending_package_intent(project) {
            INTENT_BOARD_FORMAT
        } else if board.baseline.is_some() {
            BASELINE_BOARD_FORMAT
        } else if board.drawings.is_some() {
            DRAWING_BOARD_FORMAT
        } else if outline.is_some() {
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
    if let Some(baseline) = &board.baseline {
        let encoded = baseline.encode()?;
        bytes.extend_from_slice(b"  \"baseline\": ");
        for (index, line) in encoded.split(|byte| *byte == b'\n').enumerate() {
            if index != 0 {
                bytes.extend_from_slice(b"\n  ");
            }
            bytes.extend_from_slice(line);
        }
        bytes.extend_from_slice(b",\n");
    }
    if let Some(settings) = silkscreen {
        writeln!(bytes, "  \"silkscreen\": {},", serde_json::to_string(settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    if let Some(outline) = outline {
        writeln!(bytes, "  \"outline\": {},", serde_json::to_string(outline).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    if let Some(graphics) = &board.drawings {
        bytes.extend_from_slice(b"  \"graphics\": {\n    \"layers\": [\n");
        let mut first = true;
        for layer in &graphics.layers {
            record(bytes, &mut first, layer)?;
        }
        bytes.extend_from_slice(b"    ],\n    \"items\": [\n");
        first = true;
        for item in &graphics.items {
            record(bytes, &mut first, item)?;
        }
        bytes.extend_from_slice(b"    ]\n  },\n");
    }
    if !board.tracks.is_empty() {
        bytes.extend_from_slice(b"  \"tracks\": [\n");
        let mut first = true;
        for track in &board.tracks {
            record(bytes, &mut first, track)?;
        }
        bytes.extend_from_slice(b"  ],\n");
    }
    if !board.vias.is_empty() {
        bytes.extend_from_slice(b"  \"vias\": [\n");
        let mut first = true;
        for via in &board.vias {
            record(bytes, &mut first, via)?;
        }
        bytes.extend_from_slice(b"  ],\n");
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
/// sheets retain version 1 and bypass hierarchy lists; named anchors add a bounded phase
/// before those lists in version 3. Explicit text appearance uses version 4 and retains all
/// three hierarchy/named arrays, including when empty.
fn sheet_header(bytes: &mut Vec<u8>, sheet: &crate::sheets::SchematicSheet) -> Result<u8, String> {
    let hierarchy = !sheet.labels.is_empty() || !sheet.subsheets.is_empty();
    let fields = sheet.symbols.iter().any(|s| !s.field_layout.is_empty());
    let text = sheet.net_labels.iter().any(|l| l.text.is_some());
    let format = if text {
        TEXT_LAYOUT_FORMAT
    } else if fields {
        FIELDS_SHEET_FORMAT
    } else if !sheet.net_labels.is_empty() {
        NAMED_FORMAT
    } else if hierarchy {
        HIERARCHY_FORMAT
    } else {
        SHEET_FORMAT
    };
    write!(bytes, "{{\n  \"format\": \"{format}\",\n  \"id\": \"{}\",\n", sheet.sheet_id).map_err(|e| e.to_string())?;
    if text || fields || !sheet.net_labels.is_empty() {
        bytes.extend_from_slice(b"  \"net_labels\": [\n");
        Ok(12)
    } else if hierarchy {
        bytes.extend_from_slice(b"  \"labels\": [\n");
        Ok(2)
    } else {
        bytes.extend_from_slice(b"  \"symbols\": [\n");
        Ok(4)
    }
}
