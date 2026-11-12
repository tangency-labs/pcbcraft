use super::*;
use crate::{
    board::track::tests::routed_fixture,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};

/// Exercise the same explicit board target and revision that GUI/MCP submit.
fn set(session: &mut Session, values: Value) -> Result<u64, CommandError> {
    let board = session.project().unwrap().board_target().unwrap().board;
    session.apply(SET_RULES, &json!({"board":board,"expected_revision":session.revision(),"rules":values}))
}

/// Shared rule changes preserve geometry, record identity and atomic history.
#[test]
fn board_rules_are_addressed_atomic_and_undoable() {
    let mut session = routed_fixture();
    let original = session.snapshot().unwrap();
    let values = Values { clearance_nm: 400_000, preferred_track_width_nm: 400_000, ..Values::default() };
    set(&mut session, json!(values)).unwrap();
    let saved = session.snapshot().unwrap();
    assert_eq!(original.board.tracks, saved.board.tracks);
    let id = saved.board.rules.as_ref().unwrap().id;
    let rev = session.revision();
    set(&mut session, json!(values)).unwrap();
    assert_eq!(session.revision(), rev);
    for bad in [
        json!({"preferred_track_width_nm":1}),
        json!(Values { min_via_drill_nm: 900_000, ..values }),
        json!(Values { min_track_width_nm: 500_000, ..values }),
    ] {
        assert!(set(&mut session, bad).is_err());
        assert!(Arc::ptr_eq(&saved, &session.snapshot().unwrap()));
    }
    let mut params = json!({"rules":values,"expected_revision":rev});
    assert!(session.apply(SET_RULES, &params).is_err());
    params["board"] = json!(Uuid::nil());
    assert!(session.apply(SET_RULES, &params).is_err());
    params["board"] = json!(saved.board_target().unwrap().board);
    params["expected_revision"] = json!(rev - 1);
    assert!(session.apply(SET_RULES, &params).is_err());
    set(&mut session, json!(Values { clearance_nm: 500_000, ..values })).unwrap();
    assert_eq!(session.project().unwrap().board.rules.as_ref().unwrap().id, id);
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&saved, &session.snapshot().unwrap());
    set(&mut session, Value::Null).unwrap();
    crate::assert_same_content(&original, &session.snapshot().unwrap());
}

/// New formats are selected only for authored rules, including existing copper locks.
#[test]
fn rules_format_round_trips_and_reset_restores_previous_bytes() {
    let mut session = routed_fixture();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    set(&mut session, json!(Values::default())).unwrap();
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = crate::persistence::test_board_path(&documents);
    let text = String::from_utf8(documents[&path].clone()).unwrap();
    assert!(text.contains("pcbcraft-board/19"));
    assert_eq!(documents, encode_documents(decode_documents(&documents).unwrap()).unwrap());
    let packed = encode_packed(session.snapshot().unwrap()).unwrap();
    assert_eq!(packed, encode_packed(decode_packed(&packed).unwrap().project).unwrap());
    for old in ["pcbcraft-board/13", "pcbcraft-board/18"] {
        let mut bad = documents.clone();
        bad.insert(path.clone(), text.replace("pcbcraft-board/19", old).into_bytes());
        assert!(decode_documents(&bad).is_err());
    }
    set(&mut session, Value::Null).unwrap();
    assert_eq!(original, encode_documents(session.snapshot().unwrap()).unwrap());
}

/// Malformed saved intent is refused before adoption rather than repaired by defaults.
#[test]
fn malformed_rules_never_adopt_and_old_boards_keep_their_encoding() {
    let mut session = routed_fixture();
    set(&mut session, json!(Values::default())).unwrap();
    let docs = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = crate::persistence::test_board_path(&docs);
    let value: Value = serde_json::from_slice(&docs[&path]).unwrap();
    for rules in [
        Value::Null,
        json!({}),
        json!({"format":"pcbcraft-rules/2","id":Uuid::from_u128(7),"values":Values::default()}),
        json!({"format":"pcbcraft-rules/1","id":Uuid::nil(),"values":Values::default()}),
        json!({"format":"pcbcraft-rules/1","id":Uuid::from_u128(7),"values":Values{preferred_via_drill_nm:800_000,..Values::default()}}),
    ] {
        let mut bad = docs.clone();
        let mut changed = value.clone();
        changed["rules"] = rules;
        bad.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove("rules");
    let mut bad = docs.clone();
    bad.insert(path, serde_json::to_vec(&missing).unwrap());
    assert!(decode_documents(&bad).is_err());
    for values in [
        Values { max_via_diameter_nm: 500_000, min_via_diameter_nm: 700_000, ..Default::default() },
        Values { max_via_drill_nm: 40_000, ..Default::default() },
        Values { min_annular_ring_nm: i64::MAX, ..Default::default() },
        Values { preferred_track_width_nm: i64::MIN, ..Default::default() },
    ] {
        assert!(set(&mut session, json!(values)).is_err());
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
    }
}

/// Requests may tighten saved bounds, never weaken them; geometry queries share provenance.
#[test]
fn drc_and_queries_resolve_saved_limits_and_invalidate_authority() {
    let mut session = routed_fixture();
    set(&mut session, json!(Values::default())).unwrap();
    let board = session.project().unwrap().board_target().unwrap();
    let report = session.check_design(&json!({"board":board.board})).unwrap();
    assert_eq!(report["rules"]["clearance_nm"], 200_000);
    assert_eq!(report["sources"]["clearance_nm"]["kind"], "board");
    let query = session.clearance_query_with_rules(board, &json!({})).unwrap();
    assert_eq!(query.authority().rules().values.clearance_nm, 200_000);
    assert!(session.clearance_query(board, 0).is_err());
    assert!(session.check_design(&json!({"board":board.board,"clearance_nm":0})).is_err());
    let tightened = session.check_design(&json!({"board":board.board,"clearance_nm":500_000})).unwrap();
    assert_eq!(tightened["sources"]["clearance_nm"]["kind"], "request");
    set(&mut session, json!(Values { clearance_nm: 300_000, ..Values::default() })).unwrap();
    assert!(!query.authority().is_current(&session));
    set(&mut session, Value::Null).unwrap();
    assert!(session.check_design(&json!({"board":board.board,"clearance_nm":0})).is_ok());
}

/// GUI and MCP can omit upcoming dimensions; explicit copper remains authored and DRC checks
/// it against the saved limits rather than silently resizing it when the limits change.
#[test]
fn routing_preferences_and_via_diagnostics_share_the_saved_values() {
    use crate::board::{
        drc::CHECK_DESIGN,
        track::{ADD_TRACKS, ADD_VIA, SET_COPPER_LOCKED},
    };
    let mut session = routed_fixture();
    let board = session.project().unwrap().board_target().unwrap();
    let layers = crate::board::track::tests::copper_layers(&session);
    let values =
        Values { preferred_track_width_nm: 400_000, preferred_via_diameter_nm: 900_000, preferred_via_drill_nm: 400_000, ..Values::default() };
    set(&mut session, json!(values)).unwrap();
    let execute = |session: &mut Session, id, mut params: Value| {
        params["board"] = json!(board.board);
        params["expected_revision"] = json!(session.revision());
        session.apply(id, &params).unwrap();
    };
    execute(
        &mut session,
        ADD_TRACKS,
        json!({"layer":layers[0],"points":[{"x_nm":0,"y_nm":30_000_000},{"x_nm":5_000_000,"y_nm":30_000_000},{"x_nm":10_000_000,"y_nm":30_000_000}],"layer_changes":[{"at":1,"layer":layers[1]}]}),
    );
    execute(&mut session, ADD_VIA, json!({"at":{"x_nm":0,"y_nm":40_000_000}}));
    let project = session.project().unwrap();
    assert!(project.board.tracks.iter().all(|t| t.width_nm == 400_000));
    assert!(project.board.vias.iter().all(|v| v.diameter_nm == 900_000 && v.drill_nm == 400_000));
    let tracks: Vec<_> = project.board.tracks.iter().map(|t| t.id).collect();
    execute(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"locked":true}));
    // Tightening intent is permitted even when existing locked copper violates it.
    set(
        &mut session,
        json!(Values {
            min_track_width_nm: 500_000,
            preferred_track_width_nm: 500_000,
            min_via_diameter_nm: 1_000_000,
            preferred_via_diameter_nm: 1_000_000,
            min_via_drill_nm: 500_000,
            preferred_via_drill_nm: 500_000,
            ..values
        }),
    )
    .unwrap();
    let report = session.execute(CHECK_DESIGN, &json!({"board":board.board})).unwrap();
    assert_eq!(report["design_check"]["counts"]["track_width"], 2);
    assert_eq!(report["design_check"]["counts"]["via_diameter"], 2);
    assert_eq!(report["design_check"]["counts"]["via_drill"], 2);
    let docs = encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert!(reopened.board.tracks.iter().all(|t| t.locked));
    assert_eq!(docs, encode_documents(reopened).unwrap());
}

/// Board-only intent never promotes baseline nets or rewrites the independent logical registry.
/// A combined project/17 + board/19 save retains both authorities through history and reopen.
#[test]
fn rule_changes_preserve_net_registry_identity_and_instance_bytes() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    for x_nm in [0, 25_400_000] {
        session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let symbols: Vec<_> = session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    session
        .apply(
            crate::CONNECT_PINS,
            &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":session.revision()}),
        )
        .unwrap();
    let components: Vec<_> = session.project().unwrap().components().map(|component| component.id()).collect();
    let footprint = crate::board::definition::starter("resistor-0402").unwrap();
    for component in components {
        session.apply(crate::ASSIGN_FOOTPRINT,&json!({"component":component,"library":footprint.library,"definition":footprint.id,"definition_revision":footprint.revision,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()})).unwrap();
    }
    session.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    assert!(session.project().unwrap().net_registry.records.is_empty());
    set(&mut session, json!(Values::default())).unwrap();
    assert!(session.project().unwrap().net_registry.records.is_empty());
    let symbol = session.project().unwrap().symbols()[0].id();
    session.apply(crate::nets::PROMOTE, &json!({"anchor":{"symbol":symbol,"terminal":"1"},"expected_revision":session.revision()})).unwrap();
    let registry = session.project().unwrap().inspect_net_registry().unwrap();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    set(&mut session, json!(Values { clearance_nm: 700_000, ..Values::default() })).unwrap();
    assert_eq!(session.project().unwrap().inspect_net_registry().unwrap(), registry);
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let changed: Vec<_> = after.iter().filter(|(path, bytes)| before.get(*path) != Some(*bytes)).map(|(path, _)| path.as_str()).collect();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].ends_with("board.json"));
    let reopened = decode_documents(&after).unwrap();
    assert_eq!(reopened.inspect_net_registry().unwrap(), registry);
    assert_eq!(encode_documents(reopened).unwrap(), after);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
}
