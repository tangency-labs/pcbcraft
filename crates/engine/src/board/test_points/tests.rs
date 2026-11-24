use super::*;
use crate::{
    ADD_VIA, SET_COPPER_LOCKED, SET_PLACEMENT_ORIGIN,
    board::track::tests::{routed_fixture, view},
    persistence::{self, Documents, DocumentsEncoder, decode_documents, encode_documents, test_board_path},
};

/// Build placements and copper through the production commands and return their actual IDs.
fn fixture() -> (Session, Value, Value) {
    let mut session = routed_fixture();
    session
        .apply(
            ADD_VIA,
            &json!({"at":{"x_nm":0,"y_nm":20_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,
            "expected_revision":session.revision()}),
        )
        .unwrap();
    let pad = json!({"kind":"pad","footprint":view(&session).footprints().next().unwrap().id,"pad":"1"});
    let via = json!({"kind":"via","via":view(&session).vias()[0].id});
    (session, pad, via)
}

fn params(session: &Session, target: &Value, purpose: &str, sides: Value) -> Value {
    json!({"board":session.project().unwrap().board_target().unwrap().board,"target":target,
        "expected_revision":session.revision(),"purpose":purpose,"sides":sides})
}

fn set(session: &mut Session, target: &Value, purpose: &str, sides: Value) -> Result<u64, CommandError> {
    session.apply(SET_TEST_POINT, &params(session, target, purpose, sides))
}

fn save(session: &Session) -> Documents {
    encode_documents(session.snapshot().unwrap()).unwrap()
}

/// Compare full native bytes, not just the format header. Two purposes can coexist on the same
/// object; clearing one does not clear the other. A new session reads actual encoded marks.
#[test]
fn pad_and_via_marks_survive_save_reopen_undo_and_redo() {
    let (mut session, pad, via) = fixture();
    let original = save(&session);
    let unmarked: Value = serde_json::from_slice(&original[&test_board_path(&original)]).unwrap();
    assert_eq!(unmarked["format"], "pcbcraft-board/13");
    assert!(unmarked.get("test_points").is_none());
    assert_eq!(encode_documents(decode_documents(&original).unwrap()).unwrap(), original);
    let revision = session.revision();
    set(&mut session, &pad, "assembly", Value::Null).unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(set(&mut session, &pad, "assembly", json!("top")).unwrap(), revision + 1);
    let pad_only = save(&session);
    assert_eq!(set(&mut session, &pad, "assembly", json!("top")).unwrap(), revision + 1);
    set(&mut session, &via, "fabrication", json!("bottom")).unwrap();
    let marked = save(&session);
    let path = test_board_path(&marked);
    let json: Value = serde_json::from_slice(&marked[&path]).unwrap();
    assert_eq!(json["format"], "pcbcraft-board/56");
    assert_eq!(
        json["test_points"],
        json!([
            {"target":pad,"flags":{"assembly":"top"}},
            {"target":via,"flags":{"fabrication":"bottom"}}
        ])
    );
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(save(&session), pad_only);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(save(&session), original);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(save(&session), pad_only);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(save(&session), marked);

    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&marked).unwrap());
    let project = reopened.inspect();
    assert_eq!(project["project"]["board"]["footprints"][0]["pins"][0]["test_point"], json!({"assembly":"top"}));
    assert_eq!(project["project"]["boards"][0]["vias"][0]["test_point"], json!({"fabrication":"bottom"}));
    assert_eq!(save(&reopened), marked);
    set(&mut reopened, &pad, "fabrication", json!("both")).unwrap();
    set(&mut reopened, &pad, "assembly", Value::Null).unwrap();
    assert_eq!(reopened.inspect()["project"]["board"]["footprints"][0]["pins"][0]["test_point"], json!({"fabrication":"both"}));
    set(&mut reopened, &pad, "fabrication", Value::Null).unwrap();
    set(&mut reopened, &via, "fabrication", Value::Null).unwrap();
    assert_eq!(save(&reopened), original);
}

/// Legacy marker centers are not physical lands. The shared resolver rejects them on edit and on load.
#[test]
fn legacy_placeholder_cannot_be_marked_or_loaded_as_a_test_point() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let symbol = session.project().unwrap().symbols[0].id();
    session
        .apply(
            crate::PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,
        "x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    let id = session.project().unwrap().board.footprints()[0].id;
    let pad = json!({"kind":"pad","footprint":format!("footprint-{id}"),"pad":"1"});
    let mut documents = save(&session);
    assert!(set(&mut session, &pad, "assembly", json!("top")).unwrap_err().to_string().contains("No such pad or via"));
    assert_eq!(save(&session), documents);
    let path = test_board_path(&documents);
    let mut board: Value = serde_json::from_slice(&documents[&path]).unwrap();
    board["format"] = json!("pcbcraft-board/56");
    board["test_points"] = json!([{"target":pad,"flags":{"assembly":"top"}}]);
    documents.insert(path, serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&documents).unwrap_err().contains("unresolved test-point target"));
}

/// A real land intentionally omitted from the electrical map is still addressable for marking.
#[test]
fn an_unconnected_pinned_land_can_be_marked_and_reopened() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let mut definition: Value = serde_json::from_str(include_str!("../../../../../examples/native-parts/footprints/resistor-0603.json")).unwrap();
    let mut extra = definition["pads"][0].clone();
    extra["number"] = json!("3");
    extra["center"] = json!({"x_nm":0,"y_nm":3_000_000});
    definition["pads"].as_array_mut().unwrap().push(extra);
    session
        .execute(
            "library.load_source",
            &json!({"source":"three-pad","documents":{
        "library.json":include_str!("../../../../../examples/native-parts/library.json"),
        "footprints/three.json":definition.to_string()}}),
        )
        .unwrap();
    let component = session.project().unwrap().components().next().unwrap().id();
    session
        .apply(
            crate::ASSIGN_FOOTPRINT,
            &json!({"component":component,"library":"parts-demo","definition":"resistor-0603",
        "definition_revision":1,"mapping":{"1":"1","2":"2"},"unconnected_pads":["3"],"expected_revision":session.revision()}),
        )
        .unwrap();
    session.apply(crate::PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let id = session.project().unwrap().board.footprints()[0].id;
    let pad = json!({"kind":"pad","footprint":format!("footprint-{id}"),"pad":"3"});
    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    let marked = save(&session);
    assert_eq!(encode_documents(decode_documents(&marked).unwrap()).unwrap(), marked);
}

/// Both Cut entry points refuse marked objects. Move preserves marks on the same identities.
#[test]
fn cut_refuses_marked_pad_or_via_but_move_preserves_marks() {
    use crate::board::{clipboard::CUT, movement::MOVE_ITEMS};
    let (mut session, pad, via) = fixture();
    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    set(&mut session, &via, "fabrication", json!("bottom")).unwrap();
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for items in [json!({"footprints":[pad["footprint"]]}), json!({"vias":[via["via"]]})] {
        let request = json!({"items":items,"expected_revision":revision});
        assert!(session.execute(CUT, &request).unwrap_err().to_string().contains("Use Move"));
        assert!(session.apply(CUT, &request).unwrap_err().to_string().contains("Use Move"));
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
    }
    session
        .apply(
            MOVE_ITEMS,
            &json!({"items":{"footprints":[pad["footprint"]],"vias":[via["via"]]},
        "dx_nm":500_000,"dy_nm":0,"expected_revision":revision}),
        )
        .unwrap();
    let saved = save(&session);
    let board: Value = serde_json::from_slice(&saved[&test_board_path(&saved)]).unwrap();
    assert_eq!(
        board["test_points"],
        json!([
            {"target":pad,"flags":{"assembly":"top"}},
            {"target":via,"flags":{"fabrication":"bottom"}}
        ])
    );
}

/// With a one-record budget each mark adds work to a distinct step, while a larger budget
/// emits exactly the same documents. Mark two pads on one placement to exercise the inner cursor.
#[test]
fn marks_consume_incremental_encoder_budget_and_keep_identical_bytes() {
    let (mut session, pad, via) = fixture();
    let encode = |session: &Session, budget| {
        let mut encoder = DocumentsEncoder::new(session.snapshot().unwrap());
        for steps in 1..=1024 {
            if let Some(documents) = encoder.step(budget).unwrap() {
                return (steps, documents);
            }
        }
        panic!("encoder did not finish within the fixture's step limit");
    };
    set(&mut session, &via, "assembly", json!("top")).unwrap();
    let (before, _) = encode(&session, 1);
    let mut second = pad.clone();
    second["pad"] = json!("2");
    for target in [&pad, &second] {
        set(&mut session, target, "assembly", json!("top")).unwrap();
    }
    let (marked_steps, documents) = encode(&session, 1);
    assert_eq!(marked_steps, before + 2);
    assert_eq!(encode(&session, 7).1, documents);
    assert_eq!(save(&session), documents);
    let board: Value = serde_json::from_slice(&documents[&test_board_path(&documents)]).unwrap();
    assert_eq!(board["test_points"].as_array().unwrap().len(), 3);
}

/// Every refusal leaves the exact same immutable project and history revision.
#[test]
fn validation_rejects_bad_addresses_sides_purposes_and_stale_revisions_atomically() {
    let (mut session, pad, _) = fixture();
    let before = session.snapshot().unwrap();
    let valid = params(&session, &pad, "assembly", json!("top"));
    for (field, invalid, message) in [
        ("board", json!(Uuid::from_u128(1)), "unknown board ID"),
        ("target", json!({"kind":"pad","footprint":pad["footprint"],"pad":"missing"}), "No such pad or via"),
        ("target", json!({"kind":"pad","footprint":"footprint-0","pad":"1"}), "must identify one placed pad or via"),
        ("target", json!({"kind":"via","via":Uuid::from_u128(1)}), "No such pad or via"),
        ("target", json!({"kind":"via","via":Uuid::nil()}), "must identify one placed pad or via"),
        ("target", json!({"kind":"pad","footprint":pad["footprint"],"pad":"1","via":Uuid::from_u128(1)}), "must identify one placed pad or via"),
        ("purpose", json!("any"), "must be assembly or fabrication"),
        ("sides", json!("left"), "must be top, bottom, both or null"),
    ] {
        let mut request = valid.clone();
        request[field] = invalid;
        let error = session.apply(SET_TEST_POINT, &request).unwrap_err();
        assert!(error.to_string().contains(message), "{request}: {error}");
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), valid["expected_revision"].as_u64().unwrap());
    }
    let mut stale = valid.clone();
    stale["expected_revision"] = json!(session.revision() - 1);
    assert!(matches!(session.apply(SET_TEST_POINT, &stale), Err(CommandError::StaleRevision { .. })));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    session.apply(SET_TEST_POINT, &valid).unwrap();
}

/// Format 56 carries older authoring, even when the via is locked, and refuses malformed,
/// duplicate, empty or mis-versioned marks instead of losing them during a later save.
#[test]
fn marked_format_composes_and_refuses_invalid_records() {
    let (mut session, pad, via) = fixture();
    let board = session.project().unwrap().board_target().unwrap().board;
    session.apply(SET_COPPER_LOCKED, &json!({"board":board,"vias":[via["via"]],"locked":true,"expected_revision":session.revision()})).unwrap();
    session
        .apply(
            SET_PLACEMENT_ORIGIN,
            &json!({"board":board,"origin":{"kind":"point","x_nm":123,"y_nm":456},
        "expected_revision":session.revision()}),
        )
        .unwrap();
    let original = save(&session);
    assert_eq!(serde_json::from_slice::<Value>(&original[&test_board_path(&original)]).unwrap()["format"], "pcbcraft-board/21");
    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    set(&mut session, &via, "fabrication", json!("bottom")).unwrap();
    assert!(view(&session).vias()[0].locked);
    let marked = save(&session);
    assert_eq!(encode_documents(decode_documents(&marked).unwrap()).unwrap(), marked);
    let path = test_board_path(&marked);
    let valid: Value = serde_json::from_slice(&marked[&path]).unwrap();
    assert_eq!(valid["placement_origin"]["anchor"]["x_nm"], 123);
    for (field, invalid, expected) in [
        ("format", json!("pcbcraft-board/24"), "require format 56"),
        ("test_points", json!([]), "nonempty array"),
        ("test_points", json!([valid["test_points"][0], valid["test_points"][0]]), "duplicate"),
        ("test_points", json!([{"target":pad,"flags":{}}]), "empty"),
        ("test_points", json!([{"target":pad,"flags":{"assembly":null}}]), "null"),
        ("test_points", json!([{"target":{"kind":"via","via":Uuid::from_u128(1)},"flags":{"assembly":"both"}}]), "unresolved"),
    ] {
        let mut board = valid.clone();
        board[field] = invalid;
        let mut candidate = marked.clone();
        candidate.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        let error = decode_documents(&candidate).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let mut board = valid;
    board.as_object_mut().unwrap().remove("test_points");
    let mut missing = marked;
    missing.insert(path, serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&missing).unwrap_err().contains("require format 56"));
    assert!(persistence::encode(persistence::diagnostic_snapshot(session.snapshot().unwrap())).unwrap_err().contains("test-point marks"));
    set(&mut session, &pad, "assembly", Value::Null).unwrap();
    set(&mut session, &via, "fabrication", Value::Null).unwrap();
    assert_eq!(save(&session), original);
}

/// Raw board JSON must reject positional JSON arrays and unknown fields on both `Record` and `Flags`.
#[test]
fn raw_text_test_points_reject_positional_arrays_and_unknown_fields() {
    let (mut session, pad, _) = fixture();
    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    let marked = save(&session);
    let path = test_board_path(&marked);
    let text = String::from_utf8(marked[&path].clone()).unwrap();
    let start = text.find("\"test_points\":").expect("canonical board.json must contain test_points");
    let end = start + text[start..].find("],").expect("canonical test_points array must terminate") + 1;
    let pad_json = serde_json::to_string(&pad).unwrap();
    for (replacement, expected) in [
        (format!(r#""test_points":[[{pad_json},{{"assembly":"top"}}]]"#), "expected a JSON object"),
        (format!(r#""test_points":[{{"target":{pad_json},"flags":["top",null]}}]"#), "expected a JSON object"),
        (format!(r#""test_points":[{{"target":{pad_json},"flags":{{"assembly":"top"}},"extra":1}}]"#), "unknown field `extra`"),
        (format!(r#""test_points":[{{"target":{pad_json},"flags":{{"assembly":"top","extra":1}}}}]"#), "unknown field `extra`"),
    ] {
        let mut raw = String::with_capacity(text.len() + replacement.len());
        raw.push_str(&text[..start]);
        raw.push_str(&replacement);
        raw.push_str(&text[end..]);
        let mut candidate = marked.clone();
        candidate.insert(path.clone(), raw.into_bytes());
        let error = decode_documents(&candidate).unwrap_err();
        assert!(error.contains(expected), "{replacement}: {error}");
    }
}

/// Format 56 carries test-point marks together with named layer groups, so a board with both
/// saves as 56 and reopens with both. Without marks the groups select 41, which refuses marks, and
/// the retired format 40 refuses them too. Removing the groups restores the ungrouped format 56 bytes.
#[test]
fn marks_and_named_layer_groups_save_as_56_and_reopen() {
    let (mut session, pad, via) = fixture();
    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    set(&mut session, &via, "fabrication", json!("bottom")).unwrap();
    let marked = save(&session);
    let path = test_board_path(&marked);
    let marked_board: Value = serde_json::from_slice(&marked[&path]).unwrap();
    assert_eq!(marked_board["format"], "pcbcraft-board/56");
    assert!(marked_board["setup"]["stackup"].get("layer_groups").is_none());
    let setup = session.project().unwrap().board_setup().unwrap();
    let copper: Vec<_> = setup.copper_stack().map(|facts| facts.layer).collect();
    let mut grouped_setup = json!(setup);
    grouped_setup["stackup"]["layer_groups"] = json!([{"id":crate::fresh_id().unwrap(),"name":"Probe sides","layers":copper}]);
    session.apply(crate::SET_BOARD_SETUP, &json!({"setup":grouped_setup,"expected_revision":session.revision()})).unwrap();
    let both = save(&session);
    let board: Value = serde_json::from_slice(&both[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/56");
    assert_eq!(board["test_points"], marked_board["test_points"]);
    assert_eq!(board["setup"]["stackup"]["layer_groups"].as_array().unwrap().len(), 1);
    let reopened = decode_documents(&both).unwrap();
    assert!(reopened.board.has_test_points());
    assert!(reopened.board_setup().unwrap().requires_group_format());
    assert_eq!(encode_documents(reopened).unwrap(), both);

    let mut candidate = both.clone();
    for (format, expected) in [("pcbcraft-board/41", "require format 56"), ("pcbcraft-board/40", "require format 56")] {
        let mut wrong = board.clone();
        wrong["format"] = json!(format);
        candidate.insert(path.clone(), serde_json::to_vec(&wrong).unwrap());
        let error = decode_documents(&candidate).unwrap_err();
        assert!(error.contains(expected), "{format}: {error}");
    }

    set(&mut session, &pad, "assembly", Value::Null).unwrap();
    set(&mut session, &via, "fabrication", Value::Null).unwrap();
    let grouped = save(&session);
    let board: Value = serde_json::from_slice(&grouped[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/41");
    assert!(board.get("test_points").is_none());
    assert_eq!(encode_documents(decode_documents(&grouped).unwrap()).unwrap(), grouped);

    set(&mut session, &pad, "assembly", json!("top")).unwrap();
    set(&mut session, &via, "fabrication", json!("bottom")).unwrap();
    assert_eq!(save(&session), both);
    let mut ungrouped_setup = json!(session.project().unwrap().board_setup().unwrap());
    ungrouped_setup["stackup"]["layer_groups"] = json!([]);
    session.apply(crate::SET_BOARD_SETUP, &json!({"setup":ungrouped_setup,"expected_revision":session.revision()})).unwrap();
    assert_eq!(save(&session), marked);
}
