use super::*;
use crate::{Session, persistence};
use serde_json::json;

#[test]
fn new_schematic_has_no_board_or_physical_file() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let project = session.project().unwrap();
    assert!(project.board_target().is_none());
    assert_eq!(project.boards().count(), 0);
    assert_eq!(session.inspect()["project"]["boards"], json!([]));
    assert!(session.available(CREATE_BOARD));
    assert!(session.available(crate::ASSIGN_FOOTPRINT));
    let before = session.snapshot().unwrap();
    assert!(
        session
            .apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()}))
            .unwrap_err()
            .to_string()
            .contains("no physical board")
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let documents = persistence::encode_documents(before.clone()).unwrap();
    assert!(!documents.contains_key("board.json"));
    assert_eq!(serde_json::from_slice::<Value>(&documents["project.pc"]).unwrap()["boards"], json!([]));
    let reopened = persistence::decode_documents(&documents).unwrap();
    assert!(reopened.board_target().is_none());
    assert_eq!(documents, persistence::encode_documents(reopened).unwrap());
    let single = persistence::encode(before.clone()).unwrap();
    assert!(persistence::decode(&single).unwrap().board_target().is_none());
    let packed = persistence::encode_packed(before).unwrap();
    assert!(persistence::decode_packed(&packed).unwrap().project.board_target().is_none());
}

#[test]
fn explicit_creation_preserves_schematic_and_is_undoable() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    session.apply(CREATE_BOARD, &json!({"name":"Controller", "expected_revision":revision})).unwrap();
    let target = session.project().unwrap().board_target().unwrap();
    assert_eq!(target.project, before.id());
    assert_eq!(session.project().unwrap().boards().next().unwrap().name(), "Controller");
    assert_eq!(session.project().unwrap().inspect_schematic(), before.inspect_schematic());
    let created = session.snapshot().unwrap();
    assert!(session.apply(CREATE_BOARD, &json!({})).is_err());
    assert!(Arc::ptr_eq(&created, &session.snapshot().unwrap()));
    assert!(session.project().unwrap().resolve_board(Some(&Uuid::nil().to_string())).is_err());
    session.apply("edit.undo", &json!({})).unwrap();
    assert!(session.project().unwrap().board_target().is_none());
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(session.project().unwrap().board_target(), Some(target));
    let documents = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = persistence::decode_documents(&documents).unwrap();
    assert_eq!(reopened.board_target(), Some(target));
    assert_eq!(documents, persistence::encode_documents(reopened).unwrap());
}

#[test]
fn new_pcb_is_one_atomic_project_creation() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    assert_eq!(session.project().unwrap().boards().count(), 1);
    session.apply("edit.undo", &json!({})).unwrap();
    assert!(session.project().is_none());
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(session.project().unwrap().boards().count(), 1);
}

#[test]
fn invalid_registration_never_adopts_foreign_or_hidden_physical_data() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let original = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    for mutation in 0..8 {
        let mut documents = original.clone();
        let mut entry: Value = serde_json::from_slice(&documents["project.pc"]).unwrap();
        match mutation {
            0 => entry["boards"] = json!([]), // a board.json cannot hide in a boardless project
            1 => entry["boards"] = Value::Null,
            2 => entry["boards"][0]["id"] = entry["id"].clone(),
            3 => entry["boards"][0]["file"] = json!("../foreign.json"),
            4 => entry["boards"] = json!([entry["boards"][0].clone(), entry["boards"][0].clone()]),
            5 => entry["boards"][0]["name"] = json!(" "),
            6 => entry["boards"][0]["name"] = json!(" Board "),
            _ => entry["boards"][0]["name"] = json!("\nBoard\n"),
        }
        documents.insert("project.pc".into(), serde_json::to_vec(&entry).unwrap());
        assert!(persistence::decode_documents(&documents).is_err(), "mutation {mutation}");
    }
    let mut missing = original;
    missing.remove("board.json");
    assert!(persistence::decode_documents(&missing).is_err());
}

#[test]
fn stale_board_creation_preserves_the_schematic_only_snapshot() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let stale = session.revision();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let before = session.snapshot().unwrap();
    assert!(session.apply(CREATE_BOARD, &json!({"expected_revision":stale})).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

#[test]
fn first_legacy_physical_edit_persists_the_same_board_identity() {
    let legacy = persistence::decode(include_bytes!("../../../../app/tests/fixtures/two-resistors.pc")).unwrap();
    let target = legacy.board_target().unwrap();
    let before = persistence::encode_documents(legacy.clone()).unwrap();
    let mut session = Session::default();
    session.open_snapshot(legacy);
    assert_eq!(persistence::encode_documents(session.snapshot().unwrap()).unwrap(), before);
    session
        .apply(
            crate::PLACE_FOOTPRINT,
            &json!({
                "symbol":"symbol-2", "definition":"generic-through-hole-2", "pin_1_pad":1,
                "x_nm":0,"y_nm":0,"expected_revision":session.revision()
            }),
        )
        .unwrap();
    assert_eq!(session.project().unwrap().board_target(), Some(target));
    let after = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&after["project.pc"]).unwrap();
    assert_eq!(entry["boards"][0]["id"], json!(target.board));
    assert_eq!(entry["boards"][0]["name"], "Board");
    assert_eq!(persistence::decode_documents(&after).unwrap().board_target(), Some(target));
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(session.project().unwrap().board_target(), Some(target));
    assert!(matches!(session.project().unwrap().board.ownership, Ownership::Legacy));
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(persistence::encode_documents(session.snapshot().unwrap()).unwrap(), after);
}

#[test]
fn legacy_typed_uuid_collision_keeps_a_readable_compatibility_encoding() {
    let mut legacy = persistence::decode(include_bytes!("../../../../app/tests/fixtures/two-resistors.pc")).unwrap();
    let target = legacy.board_target().unwrap();
    // Exercise the legacy UUID boundary in memory, then use the production encoder/decoder.
    // The old schema permits this cross-kind collision, unlike the explicit registry.
    Arc::make_mut(&mut Arc::make_mut(&mut legacy).sheets)[0].sheet_id = target.board;
    let loaded = persistence::decode(&persistence::encode(legacy).unwrap()).unwrap();
    let mut session = Session::default();
    session.open_snapshot(loaded);
    for symbol in ["symbol-2", "symbol-3"] {
        session
            .apply(
                crate::PLACE_FOOTPRINT,
                &json!({
                    "symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,
                    "x_nm":0,"y_nm":0,"expected_revision":session.revision()
                }),
            )
            .unwrap();
        assert!(matches!(session.project().unwrap().board.ownership, Ownership::Legacy));
        let encoded = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
        let reopened = persistence::decode_documents(&encoded).unwrap();
        assert_eq!(reopened.board_target(), Some(target));
        assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), encoded);
        session.open_snapshot(reopened);
    }
    assert_eq!(session.project().unwrap().board.footprints().len(), 2);
}

/// Part instance metadata and explicit zero/one ownership use a combined vocabulary;
/// no reader may reinterpret its fields under either separate predecessor schema.
#[test]
fn metadata_and_optional_board_ownership_round_trip_with_strict_combined_schema() {
    use crate::schematic::fields::Metadata;
    for create_board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":create_board})).unwrap();
        session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        let before = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
        let target = session.project().unwrap().board_target();
        let mut metadata = Metadata::default();
        metadata.attributes.do_not_populate = true;
        session
            .apply(
                crate::SET_SYMBOL_FIELDS,
                &json!({"symbol":"symbol-2","reference":"R1","value":"10k","metadata":metadata,"expected_revision":session.revision()}),
            )
            .unwrap();
        let documents = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
        let header: Value = serde_json::from_slice(&documents["project.pc"]).unwrap();
        assert_eq!(header["format"], "pcbcraft-project/12");
        assert_eq!(header["boards"].as_array().unwrap().len(), usize::from(create_board));
        assert_eq!(documents.contains_key("board.json"), create_board);
        let loaded = persistence::decode_documents(&documents).unwrap();
        assert_eq!(loaded.board_target(), target);
        assert_eq!(persistence::encode_documents(loaded.clone()).unwrap(), documents);
        let packed = persistence::encode_packed(loaded).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_packed(&packed).unwrap().project).unwrap(), documents);
        assert!(persistence::encode(session.snapshot().unwrap()).is_err());
        for mutation in 0..4 {
            let mut malformed = documents.clone();
            let mut entry = header.clone();
            match mutation {
                0 => entry["format"] = json!("pcbcraft-project/9"),
                1 => entry["format"] = json!("pcbcraft-project/11"),
                2 => {
                    entry.as_object_mut().unwrap().remove("boards");
                }
                _ => {
                    entry.as_object_mut().unwrap().remove("numeric_id_high_water");
                }
            }
            malformed.insert("project.pc".into(), serde_json::to_vec(&entry).unwrap());
            assert!(persistence::decode_documents(&malformed).is_err(), "mutation {mutation}");
        }
        session.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(session.snapshot().unwrap()).unwrap(), before);
        session.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(session.snapshot().unwrap()).unwrap(), documents);
    }
}

/// Legacy metadata keeps its implicit board and format-11 bytes until an actual
/// physical edit registers the same board identity under the combined schema.
#[test]
fn legacy_metadata_remains_version_11_until_the_first_physical_edit() {
    let legacy = persistence::decode(include_bytes!("../../../../app/tests/fixtures/two-resistors.pc")).unwrap();
    let mut session = Session::default();
    session.open_snapshot(legacy);
    let target = session.project().unwrap().board_target().unwrap();
    let mut metadata = crate::schematic::fields::Metadata::default();
    metadata.attributes.do_not_populate = true;
    session
        .apply(
            crate::SET_SYMBOL_FIELDS,
            &json!({"symbol":"symbol-2","reference":"R1","value":"1k","metadata":metadata,"expected_revision":session.revision()}),
        )
        .unwrap();
    let before = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&before["project.pc"]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/11");
    assert!(entry.get("boards").is_none());
    let reopened = persistence::decode_documents(&before).unwrap();
    assert_eq!(reopened.board_target(), Some(target));
    assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), before);
    session.open_snapshot(reopened);
    session.apply(crate::PLACE_FOOTPRINT,&json!({"board":target.board,"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let after = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let registered: Value = serde_json::from_slice(&after["project.pc"]).unwrap();
    assert_eq!(registered["format"], "pcbcraft-project/12");
    assert_eq!(registered["boards"][0]["id"], json!(target.board));
    assert_eq!(registered["components"], entry["components"]);
    let reopened = persistence::decode_documents(&after).unwrap();
    assert_eq!(reopened.board_target(), Some(target));
    assert_eq!(persistence::encode_documents(reopened).unwrap(), after);
    session.apply("edit.undo", &json!({})).unwrap();
    let undone = persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let undone_entry: Value = serde_json::from_slice(&undone["project.pc"]).unwrap();
    assert_eq!(undone_entry["format"], "pcbcraft-project/11");
    assert_eq!(undone_entry["components"], entry["components"]);
    assert_eq!(session.project().unwrap().board_target(), Some(target));
}
