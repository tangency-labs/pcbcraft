//! Multi-unit regression cases use original native data, not another EDA implementation.
use super::*;
use crate::{
    authoring::{Action, Item},
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};

const SYMBOL: &str = include_str!("../../../examples/native-parts/symbols/dual-amplifier.json");

/// Keep commands on the same validated GUI/MCP path while supplying the current revision.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Install original native data and place the first drawing of one part.
fn session() -> Session {
    session_with_symbol(SYMBOL)
}

/// Exercise supported small physical packages without expanding footprint assignment scope.
fn session_with_symbol(symbol: &str) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session
        .execute(
            "library.load_source",
            &json!({"source":"units", "documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
                "symbols/dual.json":symbol,
            }}),
        )
        .unwrap();
    let definition = session.libraries.symbol("units", "dual-amplifier", 1).unwrap();
    let footprint = include_str!("../../../examples/native-parts/footprints/illustrative-six-pad.json").replace("parts-demo", "units");
    session
        .execute(
            "library.load_source",
            &json!({"source":"packages","documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
                "footprints/six-pad.json":footprint,
            }}),
        )
        .unwrap();
    let digest = library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":"units","definition":"dual-amplifier","definition_revision":1,
        "definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    session
}

/// Schemas cannot enforce terminal coverage, per-unit uniqueness or stable unit identities alone.
#[test]
fn codec_checks_units_and_logical_terminal_coverage() {
    let source: Value = serde_json::from_str(SYMBOL).unwrap();
    let definition = library::parse_symbol(SYMBOL).unwrap();
    assert_eq!(definition.pins().len(), 6);
    assert_eq!(definition.units.len(), 2);
    assert_eq!(definition.unit("b").unwrap().pins().len(), 4);
    let saved = serde_json::to_value(library::saved_copy(&definition)).unwrap();
    let mut with_library = saved.clone();
    with_library["library"] = json!("units");
    assert_eq!(library::read_saved_copy(with_library).unwrap().units, definition.units);
    for change in ["old_format", "duplicate_unit", "missing_terminal", "foreign_terminal", "duplicate_pin", "overlap", "empty_units"] {
        let mut value = source.clone();
        match change {
            "old_format" => value["format"] = json!(library::SYMBOL_FORMAT_V2),
            "duplicate_unit" => value["units"][1]["id"] = value["units"][0]["id"].clone(),
            "missing_terminal" => {
                value["units"][1]["pins"].as_array_mut().unwrap().retain(|p| p["terminal"] != "6");
            }
            "foreign_terminal" => value["units"][1]["pins"][0]["terminal"] = json!("99"),
            "duplicate_pin" => {
                let pin = value["units"][1]["pins"][0].clone();
                value["units"][1]["pins"].as_array_mut().unwrap().push(pin);
            }
            "overlap" => value["units"][1]["pins"][0]["at_nm"] = value["units"][1]["pins"][1]["at_nm"].clone(),
            _ => value["units"] = json!([]),
        }
        assert!(library::parse_symbol(&value.to_string()).is_err(), "accepted {change}");
    }
}

/// Adding a unit consumes one drawing identity, no new part/reference, even without the library.
#[test]
fn pinned_units_share_fields_and_round_trip_across_sheets() {
    let mut session = session();
    let initial = session.snapshot().unwrap();
    let part = initial.symbols()[0].component_id();
    session.execute("library.remove_source", &json!({"source":"units"})).unwrap();
    apply(&mut session, sheets::CREATE_SHEET, json!({"name":"Second"}));
    let target = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, PLACE_UNIT, json!({"sheet":target.id(),"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    let project = session.snapshot().unwrap();
    assert_eq!(project.components().count(), 1);
    assert_eq!(project.all_symbols().count(), 2);
    let second = project.for_sheet(target).unwrap();
    let unit_b = &second.symbols()[0];
    assert_eq!(unit_b.component_id(), part);
    assert_eq!(unit_b.display_reference("U1"), "U1B");
    assert!(unit_b.terminals().iter().any(|pin| pin.number() == "6"));
    assert!(!unit_b.terminals().iter().any(|pin| pin.number() == "1"));
    assert_eq!(second.terminal_position(unit_b.terminals()[0]), Some(Point { x_nm: 0, y_nm: -5_080_000 }));
    apply(&mut session, SET_SYMBOL_FIELDS, json!({"sheet":target.id(),"symbol":unit_b.id(),"reference":"U7","value":"Shared value"}));
    assert!(session.project().unwrap().components().all(|part| part.reference() == "U7" && part.value() == "Shared value"));
    let project = session.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), documents);
    assert_eq!(encode_documents(decode_packed(&encode_packed(project.clone()).unwrap()).unwrap().project).unwrap(), documents);
    let before = session.snapshot().unwrap();
    for unit in ["b", "missing"] {
        let revision = session.revision();
        assert!(
            session
                .apply(PLACE_UNIT, &json!({"sheet":target.id(),"component":part,"unit":unit,"x_nm":0,"y_nm":0,"expected_revision":revision}))
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    apply(&mut session, DELETE_SYMBOL, json!({"sheet":initial.sheet_target().id(),"symbol":initial.symbols()[0].id()}));
    assert_eq!(session.project().unwrap().components().count(), 1);
    assert!(session.project().unwrap().available_units(&part).iter().any(|unit| unit.unit_id() == "a"));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(session.project().unwrap().all_symbols().count(), 2);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(session.project().unwrap().all_symbols().count(), 1);
    apply(&mut session, sheets::DELETE_SHEET, json!({"sheet":target.id()}));
    assert_eq!(session.project().unwrap().components().count(), 0);
}

/// Repeated bare supply appearances remain unconnected, without creating false physical nets.
#[test]
fn bare_shared_terminal_appearances_remain_unconnected() {
    let mut session = session();
    let first = session.snapshot().unwrap();
    let part = first.symbols()[0].component_id();
    let a = first.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    apply(&mut session, sheets::CREATE_SHEET, json!({"name":"Second"}));
    let target = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, PLACE_UNIT, json!({"sheet":target.id(),"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    let project = session.snapshot().unwrap();
    let second = project.for_sheet(target).unwrap();
    let b = second.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    assert_ne!(a, b);
    assert!(project.electrical_connectivity().net_for(a).is_none());
    assert!(project.electrical_connectivity().unconnected.contains(&a));
    assert!(project.electrical_connectivity().unconnected.contains(&b));
    // Board intent collapses shared appearances back to a single logical terminal.
    let baseline = board::update::Baseline::proposed(&project, &Default::default(), board::update::capture_source(&project).unwrap()).unwrap();
    assert_eq!(baseline.components.len(), 1);
    assert_eq!(baseline.components.values().next().unwrap().terminals.len(), 6);
    assert!(baseline.nets.is_empty());
}

/// Editing another unit preserves every parked drawing and updates shared terminal metadata.
#[test]
fn authoring_preserves_all_units_and_stable_primitive_ids() {
    let definition = library::parse_symbol(SYMBOL).unwrap();
    let mut item = Item::from_symbol(&definition).unwrap();
    item.license = "MIT OR Apache-2.0".into();
    let mut editor = authoring::Session::new(item, false).unwrap();
    let a_objects = editor.snapshot().objects.clone();
    editor.execute(editor.revision(), Action::SelectUnit { unit: "b".into() }).unwrap();
    let mut pin = editor
        .snapshot()
        .objects
        .iter()
        .find(|(_, pin)| matches!(pin, authoring::Primitive::Pin {number,..} if number == "3"))
        .map(|(id, p)| (*id, p.clone()))
        .unwrap();
    if let authoring::Primitive::Pin { name, .. } = &mut pin.1 {
        *name = Some("Supply".into());
    }
    editor.execute(editor.revision(), Action::Replace { object: pin.0, primitive: pin.1 }).unwrap();
    let encoded = editor.snapshot().encode().unwrap();
    let reopened = Item::decode(&encoded).unwrap();
    assert_eq!(reopened.other_units["a"].keys().collect::<Vec<_>>(), a_objects.keys().collect::<Vec<_>>());
    let native = reopened.symbol().unwrap();
    assert_eq!(native.units.len(), 2);
    assert!(native.units.iter().all(|unit| unit.pins().iter().find(|pin| pin.number == "3").unwrap().name.as_deref() == Some("Supply")));
    editor.execute(editor.revision(), Action::Undo).unwrap();
    editor.execute(editor.revision(), Action::Undo).unwrap();
    assert_eq!(editor.snapshot().unit, "a");
}

/// Both drawings resolve to the same physical package and shared pad, with one board baseline item.
#[test]
fn package_assignment_and_board_intent_use_complete_part_terminals() {
    let mut symbol: Value = serde_json::from_str(SYMBOL).unwrap();
    symbol["terminals"].as_array_mut().unwrap().retain(|terminal| matches!(terminal["id"].as_str(), Some("1" | "2" | "3")));
    symbol["units"][0]["pins"].as_array_mut().unwrap().retain(|pin| matches!(pin["terminal"].as_str(), Some("1" | "3")));
    symbol["units"][1]["pins"].as_array_mut().unwrap().retain(|pin| matches!(pin["terminal"].as_str(), Some("5" | "3")));
    symbol["units"][1]["pins"][0]["terminal"] = json!("2");
    let mut session = session_with_symbol(&symbol.to_string());
    let first = session.snapshot().unwrap();
    let part = first.symbols()[0].component_id();
    let symbol_a = first.symbols()[0].id();
    apply(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":part,"library":"starter","definition":"sot23","definition_revision":4,
        "mapping":{"1":"1","2":"2","3":"3"}}),
    );
    apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part,"x_nm":0,"y_nm":0}));
    apply(&mut session, sheets::CREATE_SHEET, json!({"name":"Second"}));
    let target = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, PLACE_UNIT, json!({"sheet":target.id(),"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    let second = session.project().unwrap().for_sheet(target).unwrap();
    let symbol_b = second.symbols()[0].id();
    for (sheet, symbol, terminal, bend) in [(first.sheet_target(), symbol_a, "1", 5_080_000), (target, symbol_b, "2", -5_080_000)] {
        apply(
            &mut session,
            DRAW_WIRE,
            json!({"sheet":sheet.id(),"from":{"symbol":symbol,"terminal":terminal},"to":{"symbol":symbol,"terminal":"3"},
            "bends":[{"x_nm":bend,"y_nm":-5_080_000}],"auto_heal":false}),
        );
    }
    let project = session.snapshot().unwrap();
    let a = first.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    let b = second.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    assert_eq!(project.pad_for_terminal(a), project.pad_for_terminal(b));
    assert!(project.pad_for_terminal(a).is_some());
    apply(&mut session, board::update::INITIALIZE_BASELINE, json!({}));
    let project = session.snapshot().unwrap();
    let baseline = project.board.baseline.as_ref().unwrap();
    assert_eq!(baseline.components.len(), 1);
    assert_eq!(baseline.nets.len(), 1);
    assert_eq!(baseline.nets[0].terminals.len(), 3);
    assert_eq!(project.board.footprints().len(), 1);
    apply(&mut session, DELETE_SYMBOL, json!({"sheet":first.sheet_target().id(),"symbol":first.symbols()[0].id()}));
    assert_eq!(session.project().unwrap().board.footprints().len(), 1);
    assert_eq!(session.project().unwrap().components().count(), 1);
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// One hierarchy duplication must share its part remap across root and child unit drawings.
#[test]
fn clipboard_preserves_unit_groups_across_sheets() {
    let mut session = session();
    let first = session.snapshot().unwrap();
    let root = first.sheet_target();
    let part = first.symbols()[0].component_id();
    apply(&mut session, hierarchy::CREATE_CHILD, json!({"name":"Child","x_nm":30_480_000,"y_nm":0}));
    let project = session.snapshot().unwrap();
    let child = project.sheets().nth(1).unwrap().target;
    let block = format!("sheet-symbol-{}", project.subsheets[0].id);
    apply(&mut session, PLACE_UNIT, json!({"sheet":child.id(),"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    let project = session.snapshot().unwrap();
    let items = selection::Selection::one_symbol(first.symbols()[0].id());
    let fragment = clipboard::Fragment::capture_with_hierarchy(&project, &items, &std::collections::BTreeSet::from([block]), false).unwrap();
    let (copied, _) =
        fragment.placed(&project, Point { x_nm: 76_200_000, y_nm: 0 }, project.allocation_high_water() + 1, &Default::default()).unwrap();
    assert_eq!(copied.components().count(), 2);
    assert_eq!(copied.all_symbols().count(), 4);
    for part in copied.components() {
        assert_eq!(copied.all_symbols().filter(|symbol| symbol.component_id() == part.id()).count(), 2);
    }
    let docs = encode_documents(Arc::new(copied)).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    // Cutting one drawing preserves the part represented by the child's other drawing.
    apply(&mut session, clipboard::CUT, json!({"sheet":root.id(),"items":items}));
    assert_eq!(session.project().unwrap().components().count(), 1);
}

/// Authoring rejects ambiguous unit maps and keeps exported geometry independent of selection.
#[test]
fn authoring_unit_selection_and_rejected_payloads_preserve_data() {
    let definition = library::parse_symbol(SYMBOL).unwrap();
    let item = Item::from_symbol(&definition).unwrap();
    let mut session = authoring::Session::new(item, false).unwrap();
    let original = session.snapshot().native().unwrap();
    session.execute(session.revision(), Action::SelectUnit { unit: "b".into() }).unwrap();
    let mut switched = session.snapshot().native().unwrap();
    switched["revision"] = original["revision"].clone();
    assert_eq!(switched, original);
    let json = serde_json::to_value(session.snapshot().as_ref()).unwrap();
    let mut aliased = json.clone();
    let object = aliased["other_units"]["a"].as_object_mut().unwrap();
    let first = object.keys().next().unwrap().clone();
    let value = object.remove(&first).unwrap();
    object.insert(format!("0{first}"), value);
    assert!(Item::decode(aliased.to_string().as_bytes()).is_err());
    let mut colliding = json.clone();
    colliding["other_units"]["a"] = colliding["objects"].clone();
    assert!(Item::decode(colliding.to_string().as_bytes()).is_err());
    let repeated = format!("{{\"a\":{},\"a\":{}}}", json["other_units"]["a"], json["other_units"]["a"]);
    let mut encoded = json.clone();
    encoded["other_units"] = json!({});
    let encoded = encoded.to_string().replace("\"other_units\":{}", &format!("\"other_units\":{repeated}"));
    assert!(Item::decode(encoded.as_bytes()).is_err());
    let before = session.snapshot();
    assert!(session.execute(session.revision(), Action::AddUnit { unit: "a".into() }).is_err());
    assert_eq!(session.snapshot(), before);
}
