//! Storage-only footprint lock coverage. Valid geometry is authored through production commands;
//! private snapshot edits supply metadata until a separate lock command/enforcement slice exists.
use super::*;
use crate::persistence::{decode_documents, decode_packed, encode_documents, encode_packed, test_board_path};
use crate::{Project, Session};
use std::sync::Arc;

/// Change only test-private metadata, never inventing a public mutation path or geometry.
fn with_lock(project: &Arc<Project>, locked: bool) -> Arc<Project> {
    let mut result = project.as_ref().clone();
    Arc::make_mut(&mut result.board).footprints[0].locked = locked;
    Arc::new(result)
}

/// Construct the original two-pad marker through the same commands as other legacy-board tests.
fn marker_fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let symbol = session.project().unwrap().symbols[0].id();
    session
        .apply(
            crate::PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"generic-through-hole-2",
        "pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    session
}

/// The flag survives folder/archive reopen for legacy, unconfigured physical and configured boards.
/// False is omitted, unrelated documents retain exact bytes, and unlocking restores the original bytes.
#[test]
fn footprint_lock_round_trips_without_changing_unlocked_encoding() {
    for session in [marker_fixture(), ratsnest::tests::fixture(), track::tests::routed_fixture()] {
        let original = session.snapshot().unwrap();
        let documents = encode_documents(original.clone()).unwrap();
        let packed = encode_packed(original.clone()).unwrap();
        let path = test_board_path(&documents);
        let old_board: Value = serde_json::from_slice(&documents[&path]).unwrap();
        assert!(old_board["footprints"].as_array().unwrap().iter().all(|f| f.get("locked").is_none()));
        assert!(!original.board.footprints[0].locked);
        let locked = with_lock(&original, true);
        let saved = encode_documents(locked.clone()).unwrap();
        let board: Value = serde_json::from_slice(&saved[&path]).unwrap();
        assert_eq!(board["format"], "pcbcraft-board/25");
        assert_eq!(board["footprints"][0]["locked"], true);
        assert_eq!(saved.len(), documents.len());
        for (name, bytes) in &documents {
            if name != &path {
                assert_eq!(&saved[name], bytes, "unrelated document {name}");
            }
        }
        let loaded = decode_documents(&saved).unwrap();
        assert_eq!(encode_documents(loaded.clone()).unwrap(), saved);
        assert_eq!(loaded.board.footprints[0].inspect(&loaded)["locked"], true);
        let view = loaded.board_view(loaded.board_target().unwrap()).unwrap();
        assert!(view.footprints().next().unwrap().locked);
        assert_eq!(loaded.board.footprints[0].id, original.board.footprints[0].id);
        let archive = encode_packed(locked).unwrap();
        let reopened = decode_packed(&archive).unwrap().project;
        assert!(reopened.board.footprints[0].locked);
        assert_eq!(encode_packed(reopened).unwrap(), archive);
        let unlocked = with_lock(&loaded, false);
        assert_eq!(encode_documents(unlocked.clone()).unwrap(), documents);
        assert_eq!(encode_packed(unlocked).unwrap(), packed);
        assert_eq!(encode_documents(original).unwrap(), documents, "original snapshot stays immutable");
    }
}

/// Active footprint locks cannot be smuggled under any older/reserved header; an empty extension
/// is refused too. These are downgrade attacks on the current reader, not an old-binary test.
#[test]
fn footprint_locks_require_their_own_nonempty_format() {
    let project = track::tests::routed_fixture().snapshot().unwrap();
    let original = encode_documents(project.clone()).unwrap();
    let path = test_board_path(&original);
    let saved = encode_documents(with_lock(&project, true)).unwrap();
    let text = String::from_utf8(saved[&path].clone()).unwrap();
    for version in 1..=24 {
        let mut bad = saved.clone();
        bad.insert(path.clone(), text.replace("pcbcraft-board/25", &format!("pcbcraft-board/{version}")).into_bytes());
        let error = decode_documents(&bad).unwrap_err();
        assert!(error.contains("footprint locks require board format 25"), "{version}: {error}");
    }
    let mut bad = original.clone();
    let mut board: Value = serde_json::from_slice(&bad[&path]).unwrap();
    board["format"] = json!("pcbcraft-board/25");
    bad.insert(path.clone(), serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&bad).unwrap_err().contains("requires an authored footprint lock"));
    let mut reserved = encode_documents(marker_fixture().snapshot().unwrap()).unwrap();
    let reserved_path = test_board_path(&reserved);
    let mut marker: Value = serde_json::from_slice(&reserved[&reserved_path]).unwrap();
    for version in [22, 23, 24] {
        marker["format"] = json!(format!("pcbcraft-board/{version}"));
        reserved.insert(reserved_path.clone(), serde_json::to_vec(&marker).unwrap());
        assert!(decode_documents(&reserved).unwrap_err().contains("unsupported board format"));
    }
    // Explicit false is readable under an existing header, but canonical saves omit it.
    board = serde_json::from_slice(&original[&path]).unwrap();
    board["footprints"][0]["locked"] = json!(false);
    bad.insert(path, serde_json::to_vec(&board).unwrap());
    assert_eq!(encode_documents(decode_documents(&bad).unwrap()).unwrap(), original);
}

/// Null, nonbooleans and duplicate keys are codec errors, never defaults or last-key-wins metadata.
#[test]
fn malformed_lock_flags_are_refused_before_project_construction() {
    let project = track::tests::routed_fixture().snapshot().unwrap();
    let saved = encode_documents(with_lock(&project, true)).unwrap();
    let path = test_board_path(&saved);
    let text = String::from_utf8(saved[&path].clone()).unwrap();
    assert!(text.contains("\"locked\":true"));
    for value in ["null", "1", "\"true\"", "[]", "{}"] {
        let mut bad = saved.clone();
        bad.insert(path.clone(), text.replacen("\"locked\":true", &format!("\"locked\":{value}"), 1).into_bytes());
        let error = decode_documents(&bad).unwrap_err();
        assert!(error.contains("bool"), "{value}: {error}");
    }
    let mut bad = saved.clone();
    bad.insert(path, text.replacen("\"locked\":true", "\"locked\":true,\"locked\":false", 1).into_bytes());
    assert!(decode_documents(&bad).unwrap_err().contains("duplicate field `locked`"));
}

/// Format 25 retains earlier optional origin, rules, zones, copper and copper locks with no data loss.
#[test]
fn footprint_lock_format_composes_with_preexisting_board_extensions() {
    let mut session = track::tests::routed_fixture();
    let layer = track::tests::copper_layers(&session)[0];
    track::tests::route(&mut session, layer, &[Point { x_nm: 0, y_nm: 30_000_000 }, Point { x_nm: 10_000_000, y_nm: 30_000_000 }]).unwrap();
    let board = session.project().unwrap().board_target().unwrap().board;
    for (command, mut params) in [
        (track::ADD_VIA, json!({"at":{"x_nm":0,"y_nm":40_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000})),
        (rules::SET_RULES, json!({"rules":rules::Values::default()})),
        (origin::SET_PLACEMENT_ORIGIN, json!({"origin":{"kind":"point","x_nm":2_000_000,"y_nm":3_000_000}})),
    ] {
        params["board"] = json!(board);
        params["expected_revision"] = json!(session.revision());
        session.apply(command, &params).unwrap();
    }
    let p = session.snapshot().unwrap();
    let anchor = p.inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
    session
        .apply(
            zones::ADD_ZONE,
            &json!({"layer":layer,"target":{"kind":"derived","anchor":anchor},
        "points":[{"x_nm":0,"y_nm":50_000_000},{"x_nm":10_000_000,"y_nm":50_000_000},{"x_nm":10_000_000,"y_nm":60_000_000}],
        "expected_revision":session.revision()}),
        )
        .unwrap();
    let p = session.snapshot().unwrap();
    session
        .apply(
            track::SET_COPPER_LOCKED,
            &json!({"board":board,"tracks":[p.board.tracks[0].id],"vias":[p.board.vias[0].id],
        "locked":true,"expected_revision":session.revision()}),
        )
        .unwrap();
    let original = session.snapshot().unwrap();
    let before = encode_documents(original.clone()).unwrap();
    let saved = encode_documents(with_lock(&original, true)).unwrap();
    let path = test_board_path(&saved);
    let mut expected: Value = serde_json::from_slice(&before[&path]).unwrap();
    expected["format"] = json!("pcbcraft-board/25");
    expected["footprints"][0]["locked"] = json!(true);
    assert_eq!(serde_json::from_slice::<Value>(&saved[&path]).unwrap(), expected);
    let loaded = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(loaded.clone()).unwrap(), saved);
    assert_eq!(encode_documents(with_lock(&loaded, false)).unwrap(), before);
}

/// Clipboard has no footprint-lock extension yet: exporting/importing cannot silently lose intent.
#[test]
fn clipboard_refuses_active_footprint_locks_without_changing_unlocked_fragments() {
    use clipboard::{BoardFragment, BoardSelection};
    let original = ratsnest::tests::fixture().snapshot().unwrap();
    let selection = BoardSelection { footprints: [format!("footprint-{}", original.board.footprints[0].id)].into(), ..Default::default() };
    let fragment = BoardFragment::capture(&original, &selection, false).unwrap();
    let text = fragment.to_text().unwrap();
    assert_eq!(BoardFragment::from_text(&text).unwrap().to_text().unwrap(), text);
    let locked = with_lock(&original, true);
    let error = BoardFragment::capture(&locked, &selection, false).unwrap_err();
    assert!(error.to_string().contains("footprint locks are not supported by the board clipboard format"));
    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    envelope["footprints"][0]["locked"] = json!(true);
    let error = BoardFragment::from_text(&serde_json::to_string(&envelope).unwrap()).unwrap_err();
    assert!(error.to_string().contains("footprint locks are not supported by the board clipboard format"));
    assert_eq!(BoardFragment::capture(&original, &selection, false).unwrap().to_text().unwrap(), text);
}

/// Diagnostic single-file paths explicitly refuse new lock metadata in either direction.
#[test]
fn diagnostic_codec_refuses_footprint_locks() {
    let original = marker_fixture().snapshot().unwrap();
    let error = crate::persistence::encode(with_lock(&original, true)).unwrap_err();
    assert!(error.contains("footprint locks") && error.contains("use split documents"));
    let mut document: Value = serde_json::from_slice(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap();
    // Malformed diagnostic input: the active lock must be rejected before placement ownership validation.
    let mut footprint = serde_json::to_value(&original.board.footprints[0]).unwrap();
    footprint["locked"] = json!(true);
    document["board"] = json!({"footprints":[footprint]});
    let error = crate::persistence::decode(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert!(error.contains("footprint locks require split board format 25"), "{error}");
}
