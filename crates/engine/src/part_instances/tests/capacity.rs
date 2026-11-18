//! A full 32 x 32 ball grid array at the footprint, symbol and terminal-map limits, built as an
//! original native library and placed through the shared library, schematic and board commands.
use super::*;
use crate::{
    Project,
    board::definition::{MAX_ARTWORK, MAX_PADS},
    board::update::INITIALIZE_BASELINE,
    library::MAX_TERMINALS,
};

/// Ball row names, the usual letters for grid arrays with I, O, Q, S, X and Z left out; the last
/// row exists only to name pads past the limit.
const ROWS: [&str; 33] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "J", "K", "L", "M", "N", "P", "R", "T", "U", "V", "W", "Y", "AA", "AB", "AC", "AD", "AE", "AF", "AG",
    "AH", "AJ", "AK", "AL", "AM", "AN",
];
/// Pins drawn in each symbol unit; a unit has only 441 grid spots within the pin reach.
const PINS_PER_UNIT: usize = 128;

/// Row-letter, column-number ball names in row order, 32 balls to a row.
fn balls(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("{}{}", ROWS[index / 32], index % 32 + 1)).collect()
}

/// Round 0.5 mm lands at a 1 mm pitch, centered on the origin, with `artwork` fabrication strokes.
fn footprint(pads: usize, artwork: usize) -> Json {
    let pads: Vec<_> = balls(pads)
        .into_iter()
        .enumerate()
        .map(|(index, number)| {
            let (row, column) = ((index / 32) as i64, (index % 32) as i64);
            json!({"number":number,"center":{"x_nm":(2 * column - 31) * 500_000,"y_nm":(2 * row - 31) * 500_000},
                "width_nm":500_000,"height_nm":500_000,"rotation":0,"shape":"circle","copper":"component_side"})
        })
        .collect();
    let artwork: Vec<_> = (0..artwork as i64)
        .map(|index| {
            let x = index * 100_000 - 12_800_000;
            json!({"purpose":"fabrication","graphic":{"geometry":{"type":"line","from":{"x_nm":x,"y_nm":-17_000_000},"to":{"x_nm":x,"y_nm":-16_600_000}},
                "stroke_nm":50_000,"fill":"none"}})
        })
        .collect();
    let mut footprint = json!({"format":"pcbcraft-footprint/1","library":"grid","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32",
        "license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"PCBcraft capacity fixture","locator":"Synthetic 1 mm pitch grid"},
        "pads":pads,"graphics":[{"purpose":"courtyard","center":{"x_nm":0,"y_nm":0},"width_nm":34_000_000,"height_nm":34_000_000,"rotation":0}]});
    if !artwork.is_empty() {
        footprint["artwork"] = json!(artwork);
    }
    footprint
}

/// One terminal per ball, drawn on as many units as needed with `PINS_PER_UNIT` pins each.
fn symbol(terminals: usize) -> Json {
    let names = balls(terminals);
    let units: Vec<_> = names
        .chunks(PINS_PER_UNIT)
        .enumerate()
        .map(|(unit, chunk)| {
            let pins: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(index, name)| json!({"terminal":name,"at_nm":[(index as i64 % 16 - 8) * 1_270_000,(index as i64 / 16 - 4) * 1_270_000]}))
                .collect();
            json!({"id":format!("u{}", unit + 1),"drawing":[{"type":"rect","min_nm":[-11_430_000,-6_350_000],"max_nm":[10_160_000,5_080_000]}],"pins":pins})
        })
        .collect();
    let terminals: Vec<_> = names.iter().map(|name| json!({"id":name,"electrical_type":"passive"})).collect();
    json!({"format":"pcbcraft-symbol/4","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32","reference_prefix":"U",
        "default_value":"BGA","terminals":terminals,"units":units})
}

/// A part whose only package maps each of `terminals` terminals to the ball of the same name.
fn part(terminals: usize) -> Json {
    let mapping: serde_json::Map<_, _> = balls(terminals).into_iter().map(|name| (name.clone(), json!([name]))).collect();
    json!({"format":"pcbcraft-part/1","library":"grid","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32",
        "license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"PCBcraft capacity fixture","locator":"Synthetic 1 mm pitch grid"},
        "symbol":{"library":"grid","id":"bga-1024","revision":1},
        "packages":{"bga":{"name":"BGA","footprint":{"library":"grid","id":"bga-1024","revision":1},"mapping":mapping}}})
}

/// Load the three records as one native source through the shared command and return its problems.
fn load(session: &mut Session, symbol: Json, footprint: Json, part: Json) -> Json {
    let documents: sources::Documents = [
        ("library.json", r#"{"format":"pcbcraft-library/1","id":"grid","name":"Grid arrays","license":"MIT OR Apache-2.0"}"#.to_owned()),
        ("symbols/bga-1024.json", serde_json::to_string_pretty(&symbol).unwrap()),
        ("footprints/bga-1024.json", serde_json::to_string_pretty(&footprint).unwrap()),
        ("parts/bga-1024.json", part.to_string()),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text))
    .collect();
    let loaded = session.execute(sources::LOAD, &json!({"source":"grid","documents":documents})).unwrap();
    loaded.as_array().unwrap().iter().find(|source| source["source"] == "grid").unwrap()["problems"].clone()
}

/// A 1,024-ball footprint with 256 artwork objects and a 1,024-terminal symbol place, map every
/// pin, save and reopen byte-identical; one more pad, terminal, map entry or artwork object is
/// refused with an error naming the limit.
#[test]
fn ball_grid_array_at_the_pad_and_terminal_limits_places_saves_and_reopens() {
    assert_eq!((MAX_PADS, MAX_TERMINALS, MAX_ARTWORK), (1024, 1024, 256));
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    assert_eq!(load(&mut session, symbol(MAX_TERMINALS), footprint(MAX_PADS, MAX_ARTWORK), part(MAX_TERMINALS)), json!([]));
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1})).unwrap();
    apply(
        &mut session,
        PLACE_PART,
        json!({"library":"grid","definition":"bga-1024","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0}),
    );
    let component = session.project().unwrap().symbols()[0].component_id();
    for unit in 2..=MAX_TERMINALS / PINS_PER_UNIT {
        let x = unit as i64 * 30_480_000;
        apply(&mut session, crate::PLACE_UNIT, json!({"component":component,"unit":format!("u{unit}"),"x_nm":x,"y_nm":0}));
    }
    let id = session.project().unwrap().components().next().unwrap().id();
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":id,"x_nm":0,"y_nm":0}));
    apply(&mut session, INITIALIZE_BASELINE, json!({}));

    let project = session.snapshot().unwrap();
    assert_eq!(project.symbols().len(), MAX_TERMINALS / PINS_PER_UNIT);
    assert_eq!(project.symbols().iter().map(|symbol| symbol.pins().len()).sum::<usize>(), MAX_TERMINALS);
    let definition = project.footprint_catalog().into_iter().find(|d| d.library == "grid" && d.id == "bga-1024").unwrap();
    assert_eq!((definition.pads.len(), definition.artwork.len()), (MAX_PADS, MAX_ARTWORK));
    let mapping = project.board.footprints()[0].mapping.clone().unwrap();
    assert_eq!(mapping.len(), MAX_TERMINALS);
    assert!(mapping.iter().all(|(terminal, pads)| *pads == terminal.as_str()));
    assert_eq!(project.default_footprint_mapping(&id, &definition).unwrap(), mapping);
    let baseline = project.board.baseline.as_ref().unwrap();
    assert_eq!(baseline.components.values().map(|component| component.terminals.len()).sum::<usize>(), MAX_TERMINALS);

    let documents = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    assert_eq!(reopened.board.footprints()[0].mapping.as_ref(), Some(&mapping));
    assert_eq!(reopened.symbols().iter().map(|symbol| symbol.pins().len()).sum::<usize>(), MAX_TERMINALS);
    let archive = encode_packed(project).unwrap();
    let packed_reopened = decode_packed(&archive).unwrap().project;
    assert_eq!(encode_documents(packed_reopened.clone()).unwrap(), documents);
    assert_eq!(encode_packed(packed_reopened).unwrap(), archive);
    session.execute(sources::REMOVE, &json!({"source":"grid"})).unwrap();
    session.open_snapshot(reopened);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents);

    let refused = |symbol: Json, footprint: Json, part: Json| {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        load(&mut session, symbol, footprint, part).to_string()
    };
    let problems = refused(symbol(MAX_TERMINALS), footprint(MAX_PADS + 1, 0), part(MAX_TERMINALS));
    assert!(problems.contains("a footprint has at most 1024 pads, not 1025"), "{problems}");
    let problems = refused(symbol(MAX_TERMINALS), footprint(MAX_PADS, MAX_ARTWORK + 1), part(MAX_TERMINALS));
    assert!(problems.contains("a footprint has at most 256 artwork objects, not 257"), "{problems}");
    let problems = refused(symbol(MAX_TERMINALS + 1), footprint(MAX_PADS, 0), part(MAX_TERMINALS));
    assert!(problems.contains("a symbol has 1 to 1024 terminals, not 1025"), "{problems}");
    let problems = refused(symbol(MAX_TERMINALS), footprint(MAX_PADS, 0), part(MAX_TERMINALS + 1));
    assert!(problems.contains("holds at most 1024 identities"), "{problems}");
}

/// A legal native 1,024-pad terminal works both at default placement and when selected later.
/// The same pinned choice supplies the physical footprint and survives an offline round trip.
#[test]
fn native_wide_terminal_can_be_selected_placed_and_reopened() {
    let mut wide = part(1);
    let pads = balls(MAX_PADS);
    wide["packages"]["bga"]["mapping"] = json!({"A1": pads});
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    assert_eq!(load(&mut session, symbol(1), footprint(MAX_PADS, 0), wide), json!([]));
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1})).unwrap();
    apply(
        &mut session,
        PLACE_PART,
        json!({"library":"grid","definition":"bga-1024","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0}),
    );
    let component = session.project().unwrap().components().next().unwrap().id();
    let id = session.project().unwrap().symbols()[0].component;
    assert_eq!(session.project().unwrap().board.assignments[&id].mapping["A1"].pads(), pads);
    apply(&mut session, SET_PART, json!({"component":component,"package":null}));
    assert!(!session.project().unwrap().board.assignments.contains_key(&id));
    apply(&mut session, SET_PART, json!({"component":component,"package":"bga"}));
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":0,"y_nm":0}));
    apply(&mut session, INITIALIZE_BASELINE, json!({}));
    let snapshot = session.snapshot().unwrap();
    let board = &snapshot.board;
    assert_eq!(board.footprints()[0].mapping.as_ref().unwrap()["A1"].pads(), pads);
    let accepted = &board.baseline.as_ref().unwrap().components[&id];
    assert_eq!(accepted.package.as_ref().unwrap().mapping["A1"].pads(), pads);
    let documents = encode_documents(snapshot.clone()).unwrap();
    let archive = encode_packed(snapshot).unwrap();
    session.execute(sources::REMOVE, &json!({"source":"grid"})).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    session.open_snapshot(reopened);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents);
    assert_eq!(encode_packed(decode_packed(&archive).unwrap().project).unwrap(), archive);
}

/// Compact external input can fit the source bound even though its canonical pinned output does
/// not. Project reopen accepts the latter; the external source entry point still refuses it.
#[test]
fn compact_multi_package_source_reopens_from_a_larger_pinned_document() {
    let mut native = part(MAX_TERMINALS);
    let package = native["packages"]["bga"].clone();
    for variant in 1..48 {
        native["packages"][format!("bga-{variant}")] = package.clone();
    }
    assert!(native.to_string().len() < sources::MAX_FILE_BYTES);
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    assert_eq!(load(&mut session, symbol(MAX_TERMINALS), footprint(MAX_PADS, 0), native), json!([]));
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1})).unwrap();
    apply(
        &mut session,
        PLACE_PART,
        json!({"library":"grid","definition":"bga-1024","definition_revision":1,"definition_digest":resolved["digest"],
            "package":"bga","x_nm":0,"y_nm":0}),
    );
    let snapshot = session.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    let pinned = std::str::from_utf8(&documents["library/parts/grid/bga-1024.r1.json"]).unwrap();
    assert!(pinned.len() > sources::MAX_FILE_BYTES);
    assert_eq!(crate::library::parts::PartDefinition::read(pinned).unwrap_err(), "Native library record exceeds 1 MiB.");
    let external = session.execute(sources::LOAD, &json!({"source":"oversize","documents":{"parts/bga-1024.json":pinned}}));
    assert!(
        matches!(external, Err(crate::CommandError::InvalidParams(message)) if message == "`documents` must contain bounded native path-to-text entries")
    );
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let archive = encode_packed(snapshot).unwrap();
    assert_eq!(encode_packed(decode_packed(&archive).unwrap().project).unwrap(), archive);
}

/// Seed only the aggregate-size test, sharing the pinned symbol and footprint across
/// components. The final edits use live commands rather than filling history during setup.
fn nearly_full_choice_document(board: bool) -> (Session, String, u64) {
    use crate::persistence::{ENTRY_POINT, MAX_DOCUMENT_BYTES};
    let long = |id: &Json| format!("{:0>8}", id.as_str().unwrap());
    let mut sym = symbol(MAX_TERMINALS);
    for terminal in sym["terminals"].as_array_mut().unwrap() {
        terminal["id"] = json!(long(&terminal["id"]));
    }
    for unit in sym["units"].as_array_mut().unwrap() {
        for pin in unit["pins"].as_array_mut().unwrap() {
            pin["terminal"] = json!(long(&pin["terminal"]));
        }
    }
    let mut physical = footprint(MAX_PADS, 0);
    for pad in physical["pads"].as_array_mut().unwrap() {
        pad["number"] = json!(long(&pad["number"]));
    }
    let mut native = part(MAX_TERMINALS);
    native["packages"]["bga"]["mapping"] = Json::Object(
        balls(MAX_PADS)
            .into_iter()
            .map(|id| {
                let id = format!("{id:0>8}");
                (id.clone(), json!([id]))
            })
            .collect(),
    );
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    assert_eq!(load(&mut session, sym, physical, native), json!([]));
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1})).unwrap();
    apply(
        &mut session,
        PLACE_PART,
        json!({"library":"grid","definition":"bga-1024","definition_revision":1,
        "definition_digest":resolved["digest"],"x_nm":0,"y_nm":0}),
    );
    let mut project = Project::clone(session.project().unwrap());
    let mut component = project.components.values().next().unwrap().clone();
    let placement = project.symbols()[0].clone();
    let mut choice = project.package_assignments()[&component.ordinal].clone();
    component.part = None;
    // Component and placement IDs both have constant widths throughout this fixture. The two
    // documents have the same record framing after the first assignment; measure that delta.
    let first = 1000;
    let count = 800;
    let parts = Arc::make_mut(&mut project.components);
    parts.clear();
    let mut units = Vec::new();
    for id in first..first + count {
        parts.insert(id, crate::PartInstance { ordinal: id, reference_number: id as usize, reference: format!("U{id}"), ..component.clone() });
        units.push(crate::SymbolPlacement { ordinal: id + count, component: id, ..placement.clone() });
    }
    Arc::make_mut(&mut project.sheets)[0].symbols = Arc::new(units);
    project.numeric_id_high_water = Some(project.highest_id());
    let path = project.board_document_path().unwrap_or_else(|| ENTRY_POINT.into());
    let insert = |project: &mut Project, choice: &crate::board::assignment::Assignment, id| {
        let choices =
            if board { &mut Arc::make_mut(&mut project.board).assignments } else { &mut Arc::make_mut(&mut project.schematic_packages).assignments };
        choices.insert(id, crate::board::assignment::Assignment { component: id, ..choice.clone() });
    };
    if board {
        Arc::make_mut(&mut project.board).assignments.clear();
    } else {
        Arc::make_mut(&mut project.schematic_packages).assignments.clear();
    }
    choice.component = first;
    insert(&mut project, &choice, first);
    let one = encode_documents(Arc::new(project.clone())).unwrap()[&path].len();
    insert(&mut project, &choice, first + 1);
    let two = encode_documents(Arc::new(project.clone())).unwrap()[&path].len();
    let stride = two - one;
    let n = 2 + (MAX_DOCUMENT_BYTES - two) / stride;
    assert!(n + 2 < count as usize);
    for id in first + 2..first + n as u64 - 1 {
        insert(&mut project, &choice, id);
    }
    session.open_snapshot(Arc::new(project));
    (session, path, first + n as u64 - 1)
}

/// The first refused edit must leave the published Arc, revision, undo/redo, and saved bytes intact.
fn refuse_choice_overflow(session: &mut Session, command: &str, mut params: Json, reason: &str) {
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let history = (session.undo.len(), session.redo.len());
    let revision = session.revision();
    params["expected_revision"] = json!(revision);
    let error = session.apply(command, &params).unwrap_err();
    assert!(matches!(&error, crate::CommandError::InvalidParams(msg) if msg.contains(reason)), "{error:?}");
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// Without a baseline, the last fitting assignment saves, and the next assignment or unrouted
/// placement is refused; schematic-only intent likewise cannot overflow the project document.
#[test]
fn large_maps_preflight_unrouted_board_and_schematic_choice_documents() {
    use crate::board::assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT};
    use crate::persistence::MAX_DOCUMENT_BYTES;
    for board in [true, false] {
        let (mut session, path, id) = nearly_full_choice_document(board);
        let project = session.project().unwrap();
        assert!(project.board.baseline.is_none() && project.board.tracks.is_empty() && project.board.vias.is_empty());
        let before = session.snapshot().unwrap();
        let mapping = &before.package_assignments()[&1000].mapping;
        let mut params = json!({"component":format!("component-{id}"),"library":"grid","definition":"bga-1024",
            "definition_revision":1,"mapping":mapping});
        apply(&mut session, ASSIGN_FOOTPRINT, params.clone());
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        assert!(saved[&path].len() <= MAX_DOCUMENT_BYTES && saved[&path].len() > MAX_DOCUMENT_BYTES - 24_000);
        drop(saved);
        params["component"] = json!(format!("component-{}", id + 1));
        let reason = if board { "board document exceeds the 16 MiB limit" } else { "encoded document exceeds its size limit" };
        refuse_choice_overflow(&mut session, ASSIGN_FOOTPRINT, params, reason);
        if board {
            refuse_choice_overflow(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":"component-1000","x_nm":0,"y_nm":0}), reason);
            session.open_snapshot(before);
            apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":"component-1000","x_nm":0,"y_nm":0}));
            assert!(encode_documents(session.snapshot().unwrap()).is_ok());
            refuse_choice_overflow(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":"component-1001","x_nm":40_000_000,"y_nm":0}), reason);
        }
    }
}

/// The published schemas and command parameters state the limits the engine enforces.
#[test]
fn published_schemas_state_the_pad_and_terminal_limits() {
    let read = |text: &str| serde_json::from_str::<Json>(text).unwrap();
    let footprints = read(include_str!("../../../../../docs/schemas/footprint-v1.schema.json"));
    assert_eq!(footprints["properties"]["pads"]["maxItems"], MAX_PADS);
    assert_eq!(footprints["properties"]["artwork"]["maxItems"], MAX_ARTWORK);
    for symbols in [
        include_str!("../../../../../docs/schemas/symbol-v1.schema.json"),
        include_str!("../../../../../docs/schemas/symbol-v2.schema.json"),
        include_str!("../../../../../docs/schemas/symbol-v3.schema.json"),
    ] {
        assert_eq!(read(symbols)["properties"]["terminals"]["maxItems"], 441);
    }
    let multi_unit = read(include_str!("../../../../../docs/schemas/symbol-v4.schema.json"));
    assert_eq!(multi_unit["properties"]["terminals"]["maxItems"], MAX_TERMINALS);
    let common = read(include_str!("../../../../../docs/schemas/library-common-v1.schema.json"));
    assert_eq!(common["$defs"]["unit"]["properties"]["pins"]["maxItems"], 441);
    let parts = read(include_str!("../../../../../docs/schemas/part-v2.schema.json"));
    for package in [&common["$defs"]["package"]["properties"], &parts["$defs"]["package"]["properties"]] {
        assert_eq!(package["mapping"]["maxProperties"], MAX_TERMINALS);
        assert_eq!(package["mapping"]["additionalProperties"]["maxItems"], MAX_PADS);
        assert_eq!(package["unconnected_pads"]["maxItems"], MAX_PADS);
    }
    let placement = read(include_str!("../../../../../docs/schemas/footprint-placement-v17.schema.json"));
    let older = read(include_str!("../../../../../docs/schemas/footprint-placement-v15.schema.json"));
    let command = crate::ParamKind::FootprintMapping.schema();
    for mapping in [&placement["properties"]["mapping"], &older["properties"]["mapping"], &command] {
        assert_eq!(mapping["maxProperties"], MAX_TERMINALS);
    }
    assert_eq!(placement["properties"]["mapping"]["additionalProperties"]["oneOf"][1]["maxItems"], MAX_PADS);
    assert_eq!(command["additionalProperties"]["oneOf"][1]["maxItems"], crate::board::MAX_COMMAND_PADS_PER_TERMINAL);
    assert_eq!(placement["properties"]["unconnected_pads"]["maxItems"], MAX_PADS);
    assert_eq!(crate::ParamKind::UnconnectedPads.schema()["maxItems"], MAX_PADS);
}

/// Assign/place retain their published 128-pad command arrays while native packages hold more.
/// Excess arrays name that bound before resolution and leave project bytes and revision unchanged.
#[test]
fn command_pad_arrays_keep_the_released_per_terminal_limit() {
    assert_eq!(crate::board::MAX_COMMAND_PADS_PER_TERMINAL, 128);
    let mapping = json!({"A1":balls(128)});
    assert_eq!(crate::ParamKind::FootprintMapping.validate(&mapping), Ok(()));
    assert_eq!(crate::board::parse_mapping(&mapping).unwrap()["A1"].pads().len(), 128);
    let mut native = part(1);
    native["packages"]["bga"]["mapping"] = mapping.clone();
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    assert_eq!(load(&mut session, symbol(1), footprint(128, 0), native), json!([]));
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1})).unwrap();
    apply(
        &mut session,
        PLACE_PART,
        json!({"library":"grid","definition":"bga-1024","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0}),
    );
    let component = session.project().unwrap().components().next().unwrap().id();
    let mut assignment = json!({"component":component,"library":"grid","definition":"bga-1024","definition_revision":1,
        "mapping":mapping,"expected_revision":session.revision()});
    session.apply(crate::board::assignment::ASSIGN_FOOTPRINT, &assignment).unwrap();
    let snapshot = session.snapshot().unwrap();
    let before = encode_documents(snapshot.clone()).unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let excessive = json!({"A1":balls(129)});
    assert_eq!(crate::ParamKind::FootprintMapping.validate(&excessive), Err("must map each terminal to at most 128 pads"));
    assert!(crate::board::parse_mapping(&excessive).is_none());
    assignment["mapping"] = excessive.clone();
    assignment["expected_revision"] = json!(revision);
    let placement = json!({"symbol":session.project().unwrap().symbols()[0].id(),
        "definition":crate::board::definition::BASIC_COMPONENT_IDS[0],"mapping":excessive,"x_nm":0,"y_nm":0,"expected_revision":revision});
    for (command, params) in [(crate::board::assignment::ASSIGN_FOOTPRINT, assignment), (crate::PLACE_FOOTPRINT, placement)] {
        let error = session.apply(command, &params).unwrap_err();
        assert!(
            matches!(&error, crate::CommandError::InvalidParams(message) if message == "`mapping` must map each terminal to at most 128 pads"),
            "{error:?}"
        );
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), history);
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Each raised limit accepts its last good value and refuses the next one: command mappings and
/// unconnected pads, one terminal's pad list, and no-connect intent on one part.
#[test]
fn raised_limits_accept_the_last_good_value_and_refuse_the_next() {
    let entries = |count: usize| balls(count).into_iter().map(|ball| (ball.clone(), json!(ball))).collect::<serde_json::Map<_, _>>();
    let mapping = crate::ParamKind::FootprintMapping;
    assert!(mapping.validate(&Json::Object(entries(MAX_TERMINALS))).is_ok());
    assert_eq!(
        mapping.validate(&Json::Object(entries(MAX_TERMINALS + 1))),
        Err("must map terminal IDs to pad IDs of 1 to 8 letters, digits, +, - or _")
    );
    let pads = crate::ParamKind::UnconnectedPads;
    assert!(pads.validate(&json!(balls(MAX_PADS))).is_ok());
    assert_eq!(pads.validate(&json!(balls(MAX_PADS + 1))), Err("must be distinct bounded pad identities"));
    assert!(crate::library::parts::PadIds::Many(balls(MAX_PADS)).valid());
    assert!(!crate::library::parts::PadIds::Many(balls(MAX_PADS + 1)).valid());

    let intents = |count: usize| {
        balls(count)
            .into_iter()
            .map(|terminal| crate::no_connect::Intent { terminal, electrical_type: crate::schematic::ElectricalType::Passive })
            .collect::<Vec<_>>()
    };
    assert_eq!(crate::no_connect::read_intents(intents(MAX_TERMINALS)).unwrap().len(), MAX_TERMINALS);
    let error = crate::no_connect::read_intents(intents(MAX_TERMINALS + 1)).unwrap_err();
    assert!(error.contains("at most 1024 terminals"), "{error}");
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.apply(crate::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let mut project = (*session.snapshot().unwrap()).clone();
    let part = Arc::make_mut(&mut project.components).values_mut().next().unwrap();
    part.no_connects = balls(MAX_TERMINALS).into_iter().map(|token| (token, crate::schematic::ElectricalType::Passive)).collect();
    assert!(project.validate_no_connects().is_ok());
    let part = Arc::make_mut(&mut project.components).values_mut().next().unwrap();
    part.no_connects.insert(balls(MAX_TERMINALS + 1).pop().unwrap(), crate::schematic::ElectricalType::Passive);
    // The message is shared by three causes. The project total stays within its own limit and the
    // added terminal is a valid pin number, so the 1,025th intent on one part is the only cause.
    assert!(project.no_connect_count() <= crate::no_connect::MAX_MARKERS);
    assert_eq!(project.validate_no_connects(), Err("No-connect intent exceeds its terminal or project limits.".to_string()));
}
