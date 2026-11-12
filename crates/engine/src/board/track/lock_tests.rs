//! Fixed-copper regressions use original command-built boards and the shared mutation boundary.
use super::*;
use crate::persistence::{decode_documents, decode_packed, encode_documents, encode_packed};
use std::sync::Arc;

/// Build isolated copper far from footprint pads, with stable IDs obtained from the snapshot.
fn fixture() -> Session {
    let mut session = tests::routed_fixture();
    let layer = tests::copper_layers(&session)[0];
    tests::route(
        &mut session,
        layer,
        &[Point { x_nm: 0, y_nm: 30_000_000 }, Point { x_nm: 5_000_000, y_nm: 30_000_000 }, Point { x_nm: 10_000_000, y_nm: 30_000_000 }],
    )
    .unwrap();
    apply(&mut session, ADD_VIA, json!({"at":{"x_nm":0,"y_nm":40_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    session
}

/// Send the same explicitly board-addressed, revision-checked transaction as GUI and MCP callers.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["board"] = json!(session.project().unwrap().board_target().unwrap().board);
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// A refusal leaves all authored documents, the snapshot, revision and undo/redo authority intact.
fn refused(session: &mut Session, command: &str, mut params: Value) {
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let inspection = session.inspect();
    params["board"] = json!(before.board_target().unwrap().board);
    params["expected_revision"] = json!(revision);
    let error = session.apply(command, &params).unwrap_err();
    assert!(error.to_string().contains("locked"), "{command}: {error}");
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.inspect(), inspection);
}

/// One lock transaction preserves geometry/connectivity and supports no-op, history and strict targeting.
#[test]
fn lock_command_is_atomic_board_addressed_and_undoable() {
    let mut session = fixture();
    let original = session.snapshot().unwrap();
    let tracks: Vec<_> = original.board.tracks.iter().map(|track| track.id).collect();
    let via = original.board.vias[0].id;
    let copper = tests::view(&session).copper();
    let revision = session.revision();
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":true}));
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(tests::view(&session).copper(), copper);
    assert!(tests::view(&session).tracks().iter().all(|track| track.locked));
    let inspection = session.inspect();
    assert_eq!(inspection["project"]["boards"][0]["tracks"][0]["locked"], true);
    assert_eq!(inspection["project"]["boards"][0]["vias"][0]["locked"], true);
    let locked = session.snapshot().unwrap();
    let revision = session.revision();
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":true}));
    assert_eq!(revision, session.revision());
    for bad in [
        json!({"board":Uuid::from_u128(23),"tracks":tracks,"locked":false,"expected_revision":revision}),
        json!({"board":original.board_target().unwrap().board,"tracks":tracks,"locked":false,"expected_revision":revision-1}),
        json!({"tracks":tracks,"locked":false,"expected_revision":revision}),
        json!({"board":original.board_target().unwrap().board,"tracks":[tracks[0],tracks[0]],"locked":false,"expected_revision":revision}),
        json!({"board":original.board_target().unwrap().board,"tracks":[tracks[0],Uuid::from_u128(3)],"locked":false,"expected_revision":revision}),
        json!({"board":original.board_target().unwrap().board,"tracks":[],"locked":false,"expected_revision":revision}),
    ] {
        assert!(session.apply(SET_COPPER_LOCKED, &bad).is_err(), "{bad}");
        assert!(Arc::ptr_eq(&locked, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
    }
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&original, &session.snapshot().unwrap());
    session.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&locked, &session.snapshot().unwrap());
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":false}));
    assert_eq!(session.inspect()["project"]["boards"][0]["tracks"][0]["locked"], false);
    crate::assert_same_content(&original, &session.snapshot().unwrap());
}

/// Mixed selections never partially move, reshape, resize, delete or Cut protected copper.
#[test]
fn direct_and_mixed_edits_preserve_locked_copper() {
    let mut session = fixture();
    let tracks = tests::view(&session).tracks().to_vec();
    let via = tests::view(&session).vias()[0].clone();
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":[tracks[0].id],"vias":[via.id],"locked":true}));
    let ids: Vec<_> = tracks.iter().map(|track| track.id).collect();
    refused(&mut session, MOVE_TRACKS, json!({"tracks":ids,"vias":[via.id],"dx_nm":1_000_000,"dy_nm":0}));
    refused(
        &mut session,
        RESHAPE_TRACKS,
        json!({"tracks":tracks.iter().map(|track|json!({"track":track.id,"start":track.start,"end":{"x_nm":track.end.x_nm,"y_nm":track.end.y_nm+1_000_000}})).collect::<Vec<_>>()}),
    );
    refused(&mut session, SET_TRACK_WIDTH, json!({"tracks":ids,"width_nm":400_000}));
    refused(&mut session, SET_VIA_SIZE, json!({"vias":[via.id],"via_diameter_nm":800_000,"via_drill_nm":400_000}));
    refused(&mut session, DELETE_TRACKS, json!({"tracks":ids,"vias":[via.id]}));
    let selection = json!({"tracks":ids,"vias":[via.id]});
    refused(&mut session, super::super::clipboard::DELETE, json!({"items":selection}));
    refused(&mut session, super::super::clipboard::CUT, json!({"items":selection}));
    // A genuine no-op remains safe, while independent unlocked copper remains editable.
    let revision = session.revision();
    apply(&mut session, SET_TRACK_WIDTH, json!({"tracks":ids,"width_nm":tracks[0].width_nm}));
    assert_eq!(session.revision(), revision);
    apply(&mut session, MOVE_TRACKS, json!({"tracks":[tracks[1].id],"dx_nm":0,"dy_nm":1_000_000}));
    assert_eq!(tests::view(&session).tracks().iter().find(|t| t.id == tracks[0].id).unwrap().start, tracks[0].start);
}

/// Lock-bearing files use the new extension; unlocking restores the earlier no-op encoding.
#[test]
fn copper_locks_round_trip_and_old_format_headers_refuse_them() {
    let mut session = fixture();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let tracks: Vec<_> = tests::view(&session).tracks().iter().map(|track| track.id).collect();
    let via = tests::view(&session).vias()[0].id;
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":true}));
    let locked = session.snapshot().unwrap();
    let documents = encode_documents(locked.clone()).unwrap();
    let path = crate::persistence::test_board_path(&documents);
    let text = String::from_utf8(documents[&path].clone()).unwrap();
    assert!(text.contains("pcbcraft-board/18"));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let packed = encode_packed(locked).unwrap();
    let reopened = decode_packed(&packed).unwrap();
    assert_eq!(encode_packed(reopened.project).unwrap(), packed);
    for old in ["pcbcraft-board/13", "pcbcraft-board/17"] {
        let mut bad = documents.clone();
        bad.insert(path.clone(), text.replace("pcbcraft-board/18", old).into_bytes());
        assert!(decode_documents(&bad).is_err());
    }
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":false}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
    let mut bad = original;
    let text = String::from_utf8(bad[&path].clone()).unwrap().replace("pcbcraft-board/13", "pcbcraft-board/18");
    bad.insert(path, text.into_bytes());
    assert!(decode_documents(&bad).is_err(), "format 18 requires at least one authored lock");
}

/// Copied and duplicated copper gets fresh identities with the same lock intent; Cut is protected.
#[test]
fn clipboard_copies_preserve_locks_and_old_envelopes_refuse_them() {
    use super::super::clipboard::{BoardFragment, COPY, PASTE};
    let mut session = fixture();
    let tracks: Vec<_> = tests::view(&session).tracks().iter().map(|track| track.id).collect();
    let via = tests::view(&session).vias()[0].id;
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":tracks,"vias":[via],"locked":true}));
    let original = session.snapshot().unwrap();
    let text = session
        .execute(
            COPY,
            &json!({"board":original.board_target().unwrap().board,"items":{"tracks":tracks,"vias":[via]},"expected_revision":session.revision()}),
        )
        .unwrap()["clipboard"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["format"], "pcbcraft-clipboard/3");
    assert_eq!(BoardFragment::from_text(&text).unwrap().to_text().unwrap(), text);
    for old in ["pcbcraft-clipboard/1", "pcbcraft-clipboard/2"] {
        assert!(BoardFragment::from_text(&text.replace("pcbcraft-clipboard/3", old)).is_err());
    }
    for x in [20_000_000, 40_000_000] {
        apply(&mut session, PASTE, json!({"text":text,"x_nm":x,"y_nm":30_000_000}));
    }
    let copied = tests::view(&session);
    assert_eq!(copied.tracks().len(), tracks.len() * 3);
    assert_eq!(copied.vias().len(), 3);
    assert!(copied.tracks().iter().all(|track| track.locked));
    assert!(copied.vias().iter().all(|via| via.locked));
    for track in &original.board.tracks {
        assert_eq!(copied.tracks().iter().find(|copy| copy.id == track.id), Some(track));
    }
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    session.apply("edit.undo", &Value::Null).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&original, &session.snapshot().unwrap());
}

/// Footprint transforms and alignment cannot stretch protected copper through an indirect path.
#[test]
fn footprint_moves_rotations_and_alignment_cannot_carry_locked_tracks() {
    let mut session = tests::routed_fixture();
    let (footprint, position, pad) = {
        let footprint = tests::view(&session).footprints().next().unwrap();
        (footprint.id.clone(), footprint.position, footprint.pads[0].center)
    };
    let layer = tests::copper_layers(&session)[0];
    tests::route(&mut session, layer, &[pad, Point { x_nm: pad.x_nm, y_nm: pad.y_nm + 3_000_000 }]).unwrap();
    let track = tests::view(&session).tracks()[0].id;
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":[track],"locked":true}));
    let before = session.snapshot().unwrap();
    for params in [
        json!({"footprint":footprint,"x_nm":position.x_nm+1_000_000,"y_nm":position.y_nm,"rotation":0}),
        json!({"footprint":footprint,"x_nm":position.x_nm,"y_nm":position.y_nm,"rotation":90}),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(session.revision());
        assert!(session.apply(crate::SET_FOOTPRINT_TRANSFORM, &params).is_err());
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    let footprints: Vec<_> = tests::view(&session).footprints().map(|footprint| footprint.id).collect();
    let result = session.apply(crate::ALIGN_FOOTPRINTS, &json!({"footprints":footprints,"mode":"left","expected_revision":session.revision()}));
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    apply(&mut session, SET_COPPER_LOCKED, json!({"tracks":[track],"locked":false}));
    apply(
        &mut session,
        crate::SET_FOOTPRINT_TRANSFORM,
        json!({"footprint":footprint,"x_nm":position.x_nm+1_000_000,"y_nm":position.y_nm,"rotation":0}),
    );
    assert_ne!(tests::view(&session).tracks()[0].start, pad);
}

/// The lock extension retains format-17 mappings and a pending package choice alongside accepted history.
#[test]
fn locked_format_preserves_multipad_package_intent() {
    let mut session = fixture();
    let mut footprint: Value = serde_json::from_str(include_str!("../../../../../examples/native-parts/footprints/resistor-0603.json")).unwrap();
    for number in ["3", "4"] {
        let mut pad = footprint["pads"][0].clone();
        pad["number"] = json!(number);
        pad["center"] = json!({"x_nm":if number=="3"{-2_000_000}else{2_000_000},"y_nm":3_000_000});
        footprint["pads"].as_array_mut().unwrap().push(pad);
    }
    session
        .execute(
            "library.load_source",
            &json!({"source":"four-pad","documents":{
        "library.json":include_str!("../../../../../examples/native-parts/library.json"),"footprints/four.json":footprint.to_string()}}),
        )
        .unwrap();
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(
        &mut session,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":component,"library":"parts-demo","definition":"resistor-0603","definition_revision":1,
        "mapping":{"1":["1","3"],"2":"2"},"unconnected_pads":["4"]}),
    );
    let via = tests::view(&session).vias()[0].id;
    apply(&mut session, SET_COPPER_LOCKED, json!({"vias":[via],"locked":true}));
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = crate::persistence::test_board_path(&documents);
    assert!(String::from_utf8_lossy(&documents[&path]).contains("pcbcraft-board/18"));
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    assert!(
        reopened
            .footprint_assignments()
            .iter()
            .any(|a| a["component"] == component && a["unconnected_pads"] == json!(["4"]) && a["pending_update"] == true)
    );
}
