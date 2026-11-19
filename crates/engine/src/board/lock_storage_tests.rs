//! Footprint lock storage and edit refusals. Geometry is authored through production commands;
//! private snapshot edits supply lock metadata because there is no authoring command.
use super::*;
use crate::persistence::{
    decode_documents, decode_packed, diagnostic_snapshot, encode_documents, encode_packed, encode_packed_documents, test_board_path,
};
use crate::{Project, Session};
use std::sync::Arc;

/// Change only test-private metadata, never inventing a public mutation path or geometry.
fn with_placement_lock(project: &Arc<Project>, id: u64, locked: bool) -> Arc<Project> {
    let mut result = project.as_ref().clone();
    let footprint = Arc::make_mut(&mut result.board).footprints.iter_mut().find(|footprint| footprint.id == id).unwrap();
    footprint.locked = locked;
    Arc::new(result)
}

/// The composition fixtures exercise one placement and independently vary other authored records.
fn with_lock(project: &Arc<Project>, locked: bool) -> Arc<Project> {
    with_placement_lock(project, project.board.footprints[0].id, locked)
}

/// Check every original placement by ID in public session inspection and the typed view.
fn assert_placement_locks(original: &Project, reopened: &Arc<Project>, locked_id: u64) {
    let mut session = Session::default();
    session.open_snapshot(reopened.clone());
    let inspection = session.inspect();
    let inspected = inspection["project"]["board"]["footprints"].as_array().unwrap();
    assert_eq!(inspected.len(), original.board.footprints.len());
    let view = reopened.board_view(reopened.board_target().unwrap()).unwrap();
    assert_eq!(view.footprints().count(), original.board.footprints.len());
    for footprint in &original.board.footprints {
        let id = format!("footprint-{}", footprint.id);
        let saved = inspected.iter().find(|entry| entry["id"].as_str() == Some(id.as_str())).unwrap();
        let expected = footprint.id == locked_id;
        assert_eq!(saved["locked"], expected, "{id}");
        assert_eq!(view.footprint(&id).unwrap().locked, expected, "{id}");
    }
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

/// Split documents and packed bytes retain a legacy marker lock and a non-first physical lock,
/// with and without setup. Compare the board as JSON and the unrelated documents as bytes.
#[test]
fn footprint_lock_round_trips_without_changing_unlocked_encoding() {
    for (session, lock_index) in [(marker_fixture(), 0), (ratsnest::tests::fixture(), 1), (track::tests::routed_fixture(), 1)] {
        let original = session.snapshot().unwrap();
        let documents = encode_documents(original.clone()).unwrap();
        let packed = encode_packed(original.clone()).unwrap();
        let path = test_board_path(&documents);
        let locked_id = original.board.footprints[lock_index].id;
        assert!(original.board.footprints.iter().all(|footprint| !footprint.locked));
        let mut expected: Value = serde_json::from_slice(&documents[&path]).unwrap();
        let footprints = expected["footprints"].as_array_mut().unwrap();
        assert!(footprints.iter().all(|footprint| footprint.get("locked").is_none()));
        assert_eq!(footprints[0]["id"] == locked_id, lock_index == 0);
        footprints.iter_mut().find(|footprint| footprint["id"] == locked_id).unwrap()["locked"] = json!(true);
        expected["format"] = json!("pcbcraft-board/34");
        let locked = with_placement_lock(&original, locked_id, true);
        let saved = encode_documents(locked.clone()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&saved[&path]).unwrap(), expected);
        assert_eq!(saved.len(), documents.len());
        for (name, bytes) in &documents {
            if name != &path {
                assert_eq!(&saved[name], bytes, "unrelated document {name}");
            }
        }
        let loaded = decode_documents(&saved).unwrap();
        assert_eq!(encode_documents(loaded.clone()).unwrap(), saved);
        assert_placement_locks(&original, &loaded, locked_id);
        let archive = encode_packed(locked).unwrap();
        let reopened = decode_packed(&archive).unwrap().project;
        assert_placement_locks(&original, &reopened, locked_id);
        assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
        assert_eq!(encode_packed(reopened).unwrap(), archive);
        let unlocked = with_placement_lock(&loaded, locked_id, false);
        assert_eq!(encode_documents(unlocked.clone()).unwrap(), documents);
        assert_eq!(encode_packed(unlocked).unwrap(), packed);
        assert_eq!(encode_documents(original).unwrap(), documents, "original snapshot stays immutable");
    }
}

/// A non-first lock requires format 34 even without setup; omitted and false-only flags
/// cannot satisfy that envelope. This exercises the current reader, not an older executable.
#[test]
fn footprint_locks_require_their_own_nonempty_format() {
    for session in [ratsnest::tests::fixture(), track::tests::routed_fixture()] {
        let project = session.snapshot().unwrap();
        let original = encode_documents(project.clone()).unwrap();
        let path = test_board_path(&original);
        let locked_id = project.board.footprints[1].id;
        let mut saved = encode_documents(with_placement_lock(&project, locked_id, true)).unwrap();
        let mut board: Value = serde_json::from_slice(&saved[&path]).unwrap();
        assert_eq!(board["format"], "pcbcraft-board/34");
        assert!(board["footprints"][0].get("locked").is_none());
        assert_eq!(board["footprints"][1]["id"], locked_id);
        assert_eq!(board["footprints"][1]["locked"], true);
        for version in 1..=25 {
            board["format"] = json!(format!("pcbcraft-board/{version}"));
            saved.insert(path.clone(), serde_json::to_vec(&board).unwrap());
            let error = decode_documents(&saved).unwrap_err();
            assert!(error.contains("footprint locks require board format 34"), "{version}: {error}");
        }
        let mut board: Value = serde_json::from_slice(&original[&path]).unwrap();
        let original_format = board["format"].clone();
        board["format"] = json!("pcbcraft-board/34");
        saved.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&saved).unwrap_err().contains("requires an authored footprint lock"));
        assert_eq!(board["footprints"][1]["id"], locked_id);
        board["footprints"][1]["locked"] = json!(false);
        saved.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&saved).unwrap_err().contains("requires an authored footprint lock"));
        // Explicit false is readable under the original header, but canonical saves omit it.
        board["format"] = original_format;
        saved.insert(path, serde_json::to_vec(&board).unwrap());
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), original);
    }
    let mut incomplete = encode_documents(marker_fixture().snapshot().unwrap()).unwrap();
    let marker_path = test_board_path(&incomplete);
    let mut marker: Value = serde_json::from_slice(&incomplete[&marker_path]).unwrap();
    assert_eq!(marker["format"], "pcbcraft-board/1");
    let mut noncanonical = incomplete.clone();
    marker["footprints"][0]["locked"] = json!(false);
    noncanonical.insert(marker_path.clone(), serde_json::to_vec(&marker).unwrap());
    assert_eq!(encode_documents(decode_documents(&noncanonical).unwrap()).unwrap(), incomplete);
    marker["footprints"][0].as_object_mut().unwrap().remove("locked");
    for (version, reason) in [
        (22, "Format 22 requires at least one profile"),
        (23, "unsupported board format or incompatible setup"),
        (24, "format 24 requires an assignment"),
        (25, "unsupported board format or incompatible setup"),
        (26, "format 26 requires arcs"),
    ] {
        marker["format"] = json!(format!("pcbcraft-board/{version}"));
        incomplete.insert(marker_path.clone(), serde_json::to_vec(&marker).unwrap());
        let error = decode_documents(&incomplete).unwrap_err();
        assert!(error.contains(reason), "{version}: {error}");
    }
    // Reservations are not implemented formats, even when no lock or other extension is present.
    for version in 27..=33 {
        marker["format"] = json!(format!("pcbcraft-board/{version}"));
        incomplete.insert(marker_path.clone(), serde_json::to_vec(&marker).unwrap());
        let error = decode_documents(&incomplete).unwrap_err();
        assert!(error.contains("unsupported board format"), "{version}: {error}");
    }
}

/// Author one exact arc through the command and return the full record expected on disk.
fn add_arc(session: &mut Session) -> Value {
    let target = session.project().unwrap().board_target().unwrap().board;
    let arc = json!({
        "id": uuid::Uuid::from_u128(0xa11c), "layer": track::tests::copper_layers(session)[0],
        "width_nm": 250_000, "center": {"x_nm":0,"y_nm":0},
        "start": {"x_nm":5_000_000,"y_nm":0}, "end": {"x_nm":0,"y_nm":5_000_000},
        "direction": "clockwise"
    });
    session.apply(arc::SET_ARCS, &json!({"board": target, "expected_revision": session.revision(), "arcs": [arc.clone()]})).unwrap();
    arc
}

/// Round-trip each header and return to the exact lower-format bytes when either feature is removed.
#[test]
fn arc_and_footprint_lock_formats_keep_their_own_bytes() {
    let mut session = track::tests::routed_fixture();
    let neither = session.snapshot().unwrap();
    let lock_only = with_lock(&neither, true);
    let target = neither.board_target().unwrap().board;
    let arc = add_arc(&mut session);
    let arc_only = session.snapshot().unwrap();
    let both = with_lock(&arc_only, true);
    let neither_docs = encode_documents(neither.clone()).unwrap();
    let arc_docs = encode_documents(arc_only.clone()).unwrap();
    let lock_docs = encode_documents(lock_only.clone()).unwrap();
    let both_docs = encode_documents(both.clone()).unwrap();
    let path = test_board_path(&neither_docs);
    let baseline: Value = serde_json::from_slice(&neither_docs[&path]).unwrap();
    assert!(!["pcbcraft-board/26", "pcbcraft-board/34", "pcbcraft-board/35"].contains(&baseline["format"].as_str().unwrap()));
    assert!(baseline.get("arcs").is_none());
    assert!(baseline["footprints"][0].get("locked").is_none());
    for (snapshot, docs) in [(neither, &neither_docs), (arc_only, &arc_docs), (lock_only, &lock_docs), (both, &both_docs)] {
        assert_eq!(encode_documents(decode_documents(docs).unwrap()).unwrap(), *docs);
        let packed = encode_packed(snapshot).unwrap();
        let reopened = decode_packed(&packed).unwrap().project;
        assert_eq!(encode_documents(reopened.clone()).unwrap(), *docs);
        assert_eq!(encode_packed(reopened).unwrap(), packed);
    }
    for (docs, header, has_arc, has_lock) in
        [(&arc_docs, "pcbcraft-board/26", true, false), (&lock_docs, "pcbcraft-board/34", false, true), (&both_docs, "pcbcraft-board/35", true, true)]
    {
        let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
        assert_eq!(board["format"], header);
        assert_eq!(board.get("arcs"), has_arc.then(|| json!([arc.clone()])).as_ref());
        assert_eq!(board["footprints"][0].get("locked"), has_lock.then_some(Value::Bool(true)).as_ref());
    }
    let reopened = decode_documents(&both_docs).unwrap();
    assert_eq!(encode_documents(with_lock(&reopened, false)).unwrap(), arc_docs);
    session.open_snapshot(reopened);
    session.apply(arc::SET_ARCS, &json!({"board": target, "expected_revision": session.revision(), "arcs": []})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), lock_docs);
}

/// Both features are required in format 35 and cannot be relabeled as either single-feature format.
#[test]
fn combined_format_requires_both_features() {
    let mut session = track::tests::routed_fixture();
    add_arc(&mut session);
    let saved = encode_documents(with_lock(&session.snapshot().unwrap(), true)).unwrap();
    let path = test_board_path(&saved);
    let board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/35");
    for (header, reason) in [("pcbcraft-board/26", "footprint locks require"), ("pcbcraft-board/34", "arcs require")] {
        let mut mislabeled = board.clone();
        mislabeled["format"] = json!(header);
        assert_invalid_board_record(&saved, &mislabeled, reason);
    }
    let mut missing_arcs = board.clone();
    missing_arcs.as_object_mut().unwrap().remove("arcs");
    assert_invalid_board_record(&saved, &missing_arcs, "format 35");
    let mut missing_lock = board;
    missing_lock["footprints"][0].as_object_mut().unwrap().remove("locked");
    assert_invalid_board_record(&saved, &missing_lock, "requires an authored footprint lock");
}

/// Reject the next unassigned header without triggering either feature gate.
#[test]
fn board_format_36_is_unsupported() {
    let saved = encode_documents(marker_fixture().snapshot().unwrap()).unwrap();
    let path = test_board_path(&saved);
    let mut board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/1");
    assert!(board["footprints"][0].get("locked").is_none());
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    board["format"] = json!("pcbcraft-board/36");
    assert_invalid_board_record(&saved, &board, "unsupported board format or incompatible setup: pcbcraft-board/36");
}

/// Board formats 23 and 25 need their own content even when setup is valid. A report does
/// not satisfy the lock gate, and format 34 still validates earlier extension records.
#[test]
fn configured_format_headers_require_their_own_content() {
    let mut session = track::tests::routed_fixture();
    let configured = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = test_board_path(&configured);
    assert_eq!(encode_documents(decode_documents(&configured).unwrap()).unwrap(), configured);
    let mut lower: Value = serde_json::from_slice(&configured[&path]).unwrap();
    assert!(lower["setup"].is_object());
    let mut trial = configured.clone();
    for (version, reason) in [(23, "format 23 requires at least one of them"), (25, "format 25 requires at least one of them")] {
        lower["format"] = json!(format!("pcbcraft-board/{version}"));
        trial.insert(path.clone(), serde_json::to_vec(&lower).unwrap());
        let error = decode_documents(&trial).unwrap_err();
        assert!(error.contains(reason), "format {version}: {error}");
    }

    add_material_report(&mut session);
    let project = session.snapshot().unwrap();
    let locked_id = project.board.footprints[1].id;
    let reported = encode_documents(project.clone()).unwrap();
    let mut report: Value = serde_json::from_slice(&reported[&path]).unwrap();
    assert_eq!(report["format"], "pcbcraft-board/25");
    assert_eq!(encode_documents(decode_documents(&reported).unwrap()).unwrap(), reported);
    assert_eq!(report["footprints"][1]["id"], locked_id);
    report["footprints"][1]["locked"] = json!(true);
    trial = reported;
    trial.insert(path.clone(), serde_json::to_vec(&report).unwrap());
    let error = decode_documents(&trial).unwrap_err();
    assert!(error.contains("footprint locks require board format 34"), "{error}");

    report["format"] = json!("pcbcraft-board/34");
    let canonical = encode_documents(with_placement_lock(&project, locked_id, true)).unwrap();
    report["footprints"][0]["locked"] = json!(false);
    trial.insert(path.clone(), serde_json::to_vec(&report).unwrap());
    assert_eq!(encode_documents(decode_documents(&trial).unwrap()).unwrap(), canonical);
    report["assembly_profiles"] = json!([]);
    trial.insert(path.clone(), serde_json::to_vec(&report).unwrap());
    let error = decode_documents(&trial).unwrap_err();
    assert!(error.contains("an empty assembly_profiles array is not allowed"), "{error}");
    report.as_object_mut().unwrap().remove("assembly_profiles");
    report["footprints"][1]["locked"] = json!(false);
    trial.insert(path, serde_json::to_vec(&report).unwrap());
    let error = decode_documents(&trial).unwrap_err();
    assert!(error.contains("requires an authored footprint lock"), "{error}");
}

/// Null, nonbooleans and duplicates fail split decoding; a valid archive manifest does not mask a null flag.
#[test]
fn malformed_footprint_lock_flags_fail_document_decode() {
    let project = track::tests::routed_fixture().snapshot().unwrap();
    let locked_id = project.board.footprints[1].id;
    let saved = encode_documents(with_placement_lock(&project, locked_id, true)).unwrap();
    let path = test_board_path(&saved);
    let text = String::from_utf8(saved[&path].clone()).unwrap();
    assert_eq!(text.matches("\"locked\":true").count(), 1);
    for value in ["null", "1", "\"true\"", "[]", "{}"] {
        let mut bad = saved.clone();
        bad.insert(path.clone(), text.replacen("\"locked\":true", &format!("\"locked\":{value}"), 1).into_bytes());
        let error = decode_documents(&bad).unwrap_err();
        assert!(error.contains("bool"), "{value}: {error}");
    }
    let mut bad = saved.clone();
    bad.insert(path.clone(), text.replacen("\"locked\":true", "\"locked\":true,\"locked\":false", 1).into_bytes());
    assert!(decode_documents(&bad).unwrap_err().contains("duplicate field `locked`"));

    let archive = encode_packed_documents(saved.clone()).unwrap();
    assert_eq!(decode_packed(&archive).unwrap().documents, saved);
    bad.insert(path.clone(), text.replacen("\"locked\":true", "\"locked\":null", 1).into_bytes());
    let archive = encode_packed_documents(bad).unwrap();
    let error = decode_packed(&archive).err().expect("the packed board's null lock must be refused");
    assert!(error.contains(path.as_str()) && error.contains("invalid type: null") && error.contains("bool"), "{error}");
}

/// Format 34 requires a placement list and validates the shape and identity of its records.
#[test]
fn locked_board_requires_valid_placement_records() {
    let project = ratsnest::tests::fixture().snapshot().unwrap();
    let documents = encode_documents(with_placement_lock(&project, project.board.footprints[1].id, true)).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let path = test_board_path(&documents);
    let good: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(good["format"], "pcbcraft-board/34");
    assert!(good.get("setup").is_none() && good.get("tracks").is_none() && good.get("vias").is_none());

    let mut invalid = good.clone();
    invalid.as_object_mut().unwrap().remove("footprints");
    assert_invalid_board_record(&documents, &invalid, "missing field `footprints`");
    invalid["footprints"] = json!([]);
    assert_invalid_board_record(&documents, &invalid, "requires an authored footprint lock");
    invalid = good.clone();
    let repeated = project.board.footprints[0].id;
    invalid["footprints"][1]["id"] = json!(repeated);
    assert_invalid_board_record(&documents, &invalid, &format!("invalid or duplicate object ID {repeated}"));
    invalid = good;
    invalid["footprints"][1]["unexpected_lock_field"] = json!(true);
    assert_invalid_board_record(&documents, &invalid, "unknown field `unexpected_lock_field`");
}

/// Format 34 retains earlier physical records, but still validates origin, zones, rules and copper setup.
#[test]
fn footprint_lock_format_composes_with_preexisting_board_extensions() {
    for reports in [false, true] {
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
        if reports {
            add_material_report(&mut session);
        }
        let original = session.snapshot().unwrap();
        let before = encode_documents(original.clone()).unwrap();
        let saved = encode_documents(with_lock(&original, true)).unwrap();
        let path = test_board_path(&saved);
        let mut expected: Value = serde_json::from_slice(&before[&path]).unwrap();
        expected["format"] = json!("pcbcraft-board/34");
        expected["footprints"][0]["locked"] = json!(true);
        assert_eq!(serde_json::from_slice::<Value>(&saved[&path]).unwrap(), expected);
        let loaded = decode_documents(&saved).unwrap();
        assert_eq!(encode_documents(loaded.clone()).unwrap(), saved);
        assert_eq!(encode_documents(with_lock(&loaded, false)).unwrap(), before);
        let packed = encode_packed(loaded).unwrap();
        assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
        if !reports {
            let mut invalid = expected.clone();
            invalid["placement_origin"]["id"] = json!(uuid::Uuid::nil());
            assert_invalid_board_record(&saved, &invalid, "Unsupported placement origin format or invalid origin identity");
            invalid = expected.clone();
            invalid["zones"] = json!([]);
            assert_invalid_board_record(&saved, &invalid, "and a nonempty array; format 20 requires zones");
            invalid["zones"] = Value::Null;
            assert_invalid_board_record(&saved, &invalid, "invalid type: null, expected a sequence");
            invalid = expected.clone();
            invalid["tracks"][0]["layer"] = json!(uuid::Uuid::from_u128(9042));
            assert_invalid_board_record(&saved, &invalid, "tracks must stay on a copper layer of the board setup");
            invalid = expected.clone();
            invalid.as_object_mut().unwrap().remove("setup");
            assert_invalid_board_record(&saved, &invalid, "tracks must stay on a copper layer of the board setup");
            invalid = expected.clone();
            invalid["rules"]["values"]["clearance_nm"] = json!(-1);
            assert_invalid_board_record(&saved, &invalid, "Copper clearance must be from 0 to");
        }
    }
}

/// Authored silkscreen settings remain optional under format 34, but must be valid when present.
#[test]
fn locked_board_retains_valid_authored_silkscreen() {
    let mut session = marker_fixture();
    let style = json!({"text_height_nm":1_200_000,"stroke_width_nm":180_000,"pad_clearance_nm":400_000});
    session.apply(crate::SET_SILKSCREEN, &json!({"settings":style,"expected_revision":session.revision()})).unwrap();
    let original = session.snapshot().unwrap();
    let saved = encode_documents(with_lock(&original, true)).unwrap();
    let path = test_board_path(&saved);
    let mut board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/34");
    assert_eq!(board["silkscreen"], style);
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    board["silkscreen"]["text_height_nm"] = json!(0);
    assert_invalid_board_record(&saved, &board, "silkscreen requires positive text height up to 10 mm");
    board["silkscreen"] = Value::Null;
    assert_invalid_board_record(&saved, &board, &format!("{path}: invalid type: null"));
}

/// An otherwise valid locked via-only board still needs at least two copper layers.
#[test]
fn locked_via_only_board_requires_copper_setup() {
    let mut session = track::tests::routed_fixture();
    let target = session.project().unwrap().board_target().unwrap();
    session
        .apply(
            track::ADD_VIA,
            &json!({"board":target.board,"at":{"x_nm":0,"y_nm":40_000_000},"via_diameter_nm":600_000,
        "via_drill_nm":300_000,"expected_revision":session.revision()}),
        )
        .unwrap();
    let saved = encode_documents(with_lock(&session.snapshot().unwrap(), true)).unwrap();
    let path = test_board_path(&saved);
    let mut board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/34");
    assert!(board.get("tracks").is_none());
    assert_eq!(board["vias"].as_array().unwrap().len(), 1);
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    assert!(board.as_object_mut().unwrap().remove("setup").is_some());
    assert_invalid_board_record(&saved, &board, "vias need at least two copper layers in the board setup");
}

/// Replace just the board document and check the validator's own diagnostic.
fn assert_invalid_board_record(good: &crate::persistence::Documents, board: &Value, reason: &str) {
    let mut trial = good.clone();
    trial.insert(test_board_path(good), serde_json::to_vec(board).unwrap());
    let error = decode_documents(&trial).unwrap_err();
    assert!(error.contains(reason), "expected {reason}: {error}");
}

/// Add an unused synthetic report through the production setup command, without changing
/// geometry, pins or stack fields. This report does not assert a material design limit.
fn add_material_report(session: &mut Session) {
    let mut setup = serde_json::to_value(session.project().unwrap().board_setup().unwrap()).unwrap();
    let material = json!({"id":uuid::Uuid::from_u128(9041),"revision":1,"name":"Lock storage report","family":"Synthetic",
        "thermal_conductivity":[{"conductivity_mw_per_m_k":1000,"axis":"z","basis":"typical",
            "source":{"document":"Synthetic lock-storage regression","locator":"fixture"}}]});
    if setup.get("materials").is_none() {
        setup["materials"] = json!([]);
    }
    setup["materials"].as_array_mut().unwrap().push(material);
    session.apply(crate::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":session.revision()})).unwrap();
}

/// Every combination of published report, profile, stack and via-intent fields remains optional
/// in format 34. Report-only stays 25 and active locks use 34; clearing the stored flag in
/// the test fixture restores the original bytes. The reader still rejects a bad profile pin,
/// a report missing its source and a via assignment containing a nil net address.
#[test]
fn footprint_lock_retains_published_profile_stack_and_via_extensions() {
    use assembly_profile::{ADOPT_ASSEMBLY_PROFILE, PREVIEW_ASSEMBLY_PROFILE, SET_ASSEMBLY_PROFILE};
    for reports in [false, true] {
        for profiles in [false, true] {
            for stack in [false, true] {
                for via_intent in [false, true] {
                    let mut session = track::tests::routed_fixture();
                    let board = session.project().unwrap().board_target().unwrap().board;
                    if profiles {
                        session
                            .apply(
                                SET_ASSEMBLY_PROFILE,
                                &json!({"content":{"name":"Lock compatibility","revision":1,"provenance":{"kind":"authored"}},
                                "expected_revision":session.revision()}),
                            )
                            .unwrap();
                        let profile = session.project().unwrap().assembly_profiles().heads[0].profile;
                        let mut adoption = json!({"board":board,"profile":profile,"revision":1,"expected_revision":session.revision()});
                        adoption["acceptance"] = session.execute(PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap()["acceptance"].clone();
                        session.apply(ADOPT_ASSEMBLY_PROFILE, &adoption).unwrap();
                    }
                    if stack {
                        let mut setup = serde_json::to_value(session.project().unwrap().board_setup().unwrap()).unwrap();
                        setup["stackup"]["finished_thickness_nm"] = json!(1_600_000);
                        session.apply(crate::SET_BOARD_SETUP, &json!({"board":board,"setup":setup,"expected_revision":session.revision()})).unwrap();
                    }
                    if via_intent {
                        let anchor = session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
                        session
                            .apply(
                                track::ADD_VIA,
                                &json!({"board":board,"at":{"x_nm":-30_000_000,"y_nm":-30_000_000},
                                "target":{"kind":"derived","anchor":anchor},"expected_revision":session.revision()}),
                            )
                            .unwrap();
                    }
                    if reports {
                        add_material_report(&mut session);
                    }
                    let original = session.snapshot().unwrap();
                    let before = encode_documents(original.clone()).unwrap();
                    assert_eq!(encode_documents(decode_documents(&before).unwrap()).unwrap(), before, "unlocked no-op");
                    let original_archive = encode_packed(original.clone()).unwrap();
                    let saved = encode_documents(with_lock(&original, true)).unwrap();
                    let path = test_board_path(&saved);
                    let mut expected: Value = serde_json::from_slice(&before[&path]).unwrap();
                    if reports || via_intent || stack || profiles {
                        assert_eq!(
                            expected["format"],
                            format!(
                                "pcbcraft-board/{}",
                                if reports {
                                    25
                                } else if via_intent {
                                    24
                                } else if stack {
                                    23
                                } else {
                                    22
                                }
                            )
                        );
                    }
                    let mut explicit_false = before.clone();
                    let mut false_board = expected.clone();
                    false_board["footprints"][0]["locked"] = json!(false);
                    explicit_false.insert(path.clone(), serde_json::to_vec(&false_board).unwrap());
                    assert_eq!(encode_documents(decode_documents(&explicit_false).unwrap()).unwrap(), before, "explicit false canonicalizes");
                    expected["format"] = json!("pcbcraft-board/34");
                    expected["footprints"][0]["locked"] = json!(true);
                    assert_eq!(serde_json::from_slice::<Value>(&saved[&path]).unwrap(), expected);
                    for (name, bytes) in &before {
                        if name != &path {
                            assert_eq!(&saved[name], bytes, "unrelated document {name}");
                        }
                    }
                    let loaded = decode_documents(&saved).unwrap();
                    assert_eq!(encode_documents(loaded.clone()).unwrap(), saved);
                    if stack && !reports && !profiles && !via_intent {
                        let mut invalid = expected.clone();
                        assert_eq!(invalid["setup"]["stackup"]["finished_thickness_nm"], 1_600_000);
                        invalid["setup"]["stackup"]["finished_thickness_nm"] = json!(0);
                        assert_invalid_board_record(&saved, &invalid, "finished board thickness must be a positive whole number of nanometers");
                    }
                    if profiles && !reports && !stack && !via_intent {
                        let mut invalid = expected.clone();
                        invalid["assembly_profiles"][0]["revision"] = json!(2);
                        assert_invalid_board_record(&saved, &invalid, "reference pinned content revisions");
                    }
                    if reports && via_intent && !profiles && !stack {
                        let mut invalid = expected.clone();
                        let report = invalid["setup"]["materials"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|material| material["thermal_conductivity"].is_array())
                            .unwrap();
                        assert!(report["thermal_conductivity"][0].as_object_mut().unwrap().remove("source").is_some());
                        assert_invalid_board_record(&saved, &invalid, "missing field `source`");
                        let mut invalid = expected;
                        assert!(invalid["vias"][0]["net_assignment"]["net"].is_string());
                        invalid["vias"][0]["net_assignment"]["net"] = json!(uuid::Uuid::nil());
                        assert_invalid_board_record(&saved, &invalid, &format!("{path}: Via net addresses require non-nil identities."));
                    }
                    let packed = encode_packed(loaded.clone()).unwrap();
                    assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
                    let unlocked = with_lock(&loaded, false);
                    assert_eq!(encode_documents(unlocked.clone()).unwrap(), before);
                    assert_eq!(encode_packed(unlocked).unwrap(), original_archive);
                }
            }
        }
    }
}

/// Copy can capture an unlocked neighbor, but a selection containing a locked placement is refused.
/// An omitted or explicit false flag does not change the canonical unlocked encoding.
#[test]
fn clipboard_refuses_active_footprint_locks_without_changing_unlocked_fragments() {
    use clipboard::{BoardFragment, BoardSelection};
    let original = ratsnest::tests::fixture().snapshot().unwrap();
    let locked_id = original.board.footprints[1].id;
    let selection = BoardSelection {
        footprints: original.board.footprints.iter().map(|footprint| format!("footprint-{}", footprint.id)).collect(),
        ..Default::default()
    };
    let fragment = BoardFragment::capture(&original, &selection, false).unwrap();
    let text = fragment.to_text().unwrap();
    assert_eq!(BoardFragment::from_text(&text).unwrap().to_text().unwrap(), text);
    let locked = with_placement_lock(&original, locked_id, true);
    let unlocked_only = BoardSelection { footprints: [format!("footprint-{}", original.board.footprints[0].id)].into(), ..Default::default() };
    let copied = BoardFragment::capture(&locked, &unlocked_only, false).unwrap();
    assert_eq!(copied.counts(), (1, 0, 0, 0));
    for cut in [false, true] {
        BoardFragment::capture(&original, &selection, cut).unwrap();
        let error = BoardFragment::capture(&locked, &selection, cut).unwrap_err();
        assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error}");
        assert!(error.to_string().contains("footprint locks are not supported by the board clipboard format"));
    }
    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    assert!(envelope["footprints"][0].get("locked").is_none());
    assert_eq!(envelope["footprints"][1]["id"], locked_id);
    envelope["footprints"][1]["locked"] = json!(false);
    assert_eq!(BoardFragment::from_text(&serde_json::to_string(&envelope).unwrap()).unwrap().to_text().unwrap(), text);
    envelope["footprints"][1]["locked"] = json!(true);
    let error = BoardFragment::from_text(&serde_json::to_string(&envelope).unwrap()).unwrap_err();
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error}");
    assert!(error.to_string().contains("footprint locks are not supported by the board clipboard format"));
    for malformed in [json!("true"), Value::Null] {
        envelope["footprints"][1]["locked"] = malformed;
        let error = BoardFragment::from_text(&serde_json::to_string(&envelope).unwrap()).unwrap_err();
        assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error}");
        assert!(error.to_string().contains("Clipboard does not contain a supported PCBcraft board fragment"));
    }
}

/// Compare rejected commands against unlocked controls through the public command path.
/// Errors do not return cut text, consume a revision or change physical objects or history.
#[test]
fn clipboard_commands_refuse_footprint_locks_without_a_state_change() {
    use clipboard::{BoardSelection, COPY, CUT, PASTE};
    let original = track::tests::routed_fixture().snapshot().unwrap();
    let board = original.board_target().unwrap().board;
    let locked_id = original.board.footprints[1].id;
    let items = BoardSelection {
        footprints: original.board.footprints.iter().map(|footprint| format!("footprint-{}", footprint.id)).collect(),
        ..Default::default()
    };
    let mut source = Session::default();
    source.open_snapshot(original.clone());
    let revision = source.revision();
    let copy_args = json!({"board":board,"items":items,"expected_revision":revision});
    let copied = source.execute(COPY, &copy_args).unwrap();
    assert!(copied["clipboard"].is_string());
    assert_eq!(source.revision(), revision);
    let cut = source.execute(CUT, &copy_args).unwrap();
    assert!(source.project().unwrap().board.footprints.is_empty());
    let cut_text = cut["clipboard"].as_str().unwrap();

    let mut locked = Session::default();
    locked.open_snapshot(with_placement_lock(&original, locked_id, true));
    locked.apply("project.rename", &json!({"name":"Locked clipboard control"})).unwrap();
    add_redo_control(&mut locked);
    let args = json!({"board":board,"items":items,"expected_revision":locked.revision()});
    for command in [COPY, CUT] {
        assert_clipboard_refusal(&mut locked, command, &args, false);
    }
    assert_clipboard_refusal(&mut locked, CUT, &args, true);
    assert_placement_locks(&original, &locked.snapshot().unwrap(), locked_id);

    let mut direct_cut = Session::default();
    direct_cut.open_snapshot(original.clone());
    direct_cut.apply("project.rename", &json!({"name":"Unlocked clipboard control"})).unwrap();
    add_redo_control(&mut direct_cut);
    let revision = direct_cut.revision();
    direct_cut.apply(CUT, &json!({"board":board,"items":items,"expected_revision":revision})).unwrap();
    assert_eq!(direct_cut.revision(), revision + 1);
    assert!(direct_cut.project().unwrap().board.footprints.is_empty());

    add_redo_control(&mut source);
    let paste = json!({"board":board,"text":cut_text,"reuse_components":true,"x_nm":0,"y_nm":10_000_000,"expected_revision":source.revision()});
    let mut envelope: Value = serde_json::from_str(cut_text).unwrap();
    assert_eq!(envelope["footprints"][1]["id"], locked_id);
    envelope["footprints"][1]["locked"] = json!(true);
    let mut bad_paste = paste.clone();
    bad_paste["text"] = json!(serde_json::to_string(&envelope).unwrap());
    assert_clipboard_refusal(&mut source, PASTE, &bad_paste, false);
    source.execute(PASTE, &paste).unwrap();
    assert_eq!(source.project().unwrap().board.footprints.len(), original.board.footprints.len());
}

/// Direct layout edits refuse all selected locks, even a stationary alignment anchor. The same
/// requests succeed on the unlocked fixture, and refusal never replaces a snapshot or its history.
#[test]
fn locked_placements_refuse_direct_edits_atomically() {
    use crate::{ALIGN_FOOTPRINTS, DELETE_FOOTPRINTS, PLACE_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM};
    use clipboard::{BoardSelection, DELETE};
    use movement::{MOVE_ITEMS, MovePlan};

    let mut fixture = Session::default();
    fixture.apply("project.new", &json!({"create_board":true})).unwrap();
    for (x_nm, y_nm) in [(0, 0), (10_000_000, 8_000_000), (30_000_000, 16_000_000)] {
        fixture.apply(PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":0,"expected_revision":fixture.revision()})).unwrap();
        let symbol = fixture.project().unwrap().symbols().last().unwrap().id();
        fixture
            .apply(
                PLACE_FOOTPRINT,
                &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":x_nm,"y_nm":y_nm,
                    "expected_revision":fixture.revision()}),
            )
            .unwrap();
    }
    let original = fixture.snapshot().unwrap();
    let target = original.board_target().unwrap();
    let board = target.board;
    let view = original.board_view(target).unwrap();
    let ids: Vec<_> = view.footprint_ids().collect();
    let locked_ref = view.footprint(&ids[0]).unwrap().reference;
    let other_ref = view.footprint(&ids[2]).unwrap().reference;
    let items = BoardSelection { footprints: ids.iter().cloned().collect(), ..Default::default() };
    let commands = [
        (MOVE_ITEMS, json!({"board":board,"items":items,"dx_nm":1_000_000,"dy_nm":0})),
        (SET_FOOTPRINT_TRANSFORM, json!({"board":board,"footprint":ids[0],"x_nm":1_000_000,"y_nm":0,"rotation":0})),
        (SET_FOOTPRINT_TRANSFORM, json!({"board":board,"footprint":ids[0],"x_nm":0,"y_nm":0,"rotation":90})),
        (SET_FOOTPRINT_TRANSFORM, json!({"board":board,"footprint":ids[0],"x_nm":0,"y_nm":0,"rotation":0,"side":"back"})),
        (ALIGN_FOOTPRINTS, json!({"board":board,"footprints":ids,"mode":"left"})),
        (ALIGN_FOOTPRINTS, json!({"board":board,"footprints":ids,"mode":"distribute_x"})),
        (DELETE_FOOTPRINTS, json!({"board":board,"footprints":ids})),
        (DELETE, json!({"board":board,"items":items})),
    ];
    let marked = with_placement_lock(&original, original.board.footprints[0].id, true);
    let mut locked = Session::default();
    locked.open_snapshot(decode_documents(&encode_documents(marked.clone()).unwrap()).unwrap());
    locked.apply("project.rename", &json!({"name":"Locked layout fixture"})).unwrap();
    add_redo_control(&mut locked);
    let before = locked.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = locked.revision();
    let root = locked.history_root_revision();
    let undo = locked.undo.len();
    let redo = locked.redo.len();
    assert!(undo > 0 && redo > 0);
    let expect_name = format!("{locked_ref} ({})", ids[0]);
    let preview = MovePlan::new(before.clone(), target, items).unwrap_err();
    assert!(preview.to_string().contains(&expect_name), "{preview}");
    for (command, mut params) in commands {
        params["expected_revision"] = json!(revision);
        let error = locked.execute(command, &params).unwrap_err();
        assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{command}: {error}");
        assert!(error.to_string().contains(&expect_name), "{command}: {error}");
        assert!(Arc::ptr_eq(&before, &locked.snapshot().unwrap()), "{command}");
        assert_eq!(encode_documents(locked.snapshot().unwrap()).unwrap(), bytes, "{command}");
        assert_eq!(
            (locked.revision(), locked.history_root_revision(), locked.undo.len(), locked.redo.len()),
            (revision, root, undo, redo),
            "{command}"
        );

        let mut control = Session::default();
        control.open_snapshot(original.clone());
        let base = control.revision();
        params["expected_revision"] = json!(base);
        control.execute(command, &params).unwrap();
        assert_eq!(control.revision(), base + 1, "{command}");
        control.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&original, &control.snapshot().unwrap()), "{command}");
    }
    locked
        .execute(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":board,"footprint":ids[0],"x_nm":0,"y_nm":0,"rotation":0,
            "side":"front","expected_revision":revision}),
        )
        .unwrap();
    assert_eq!(locked.revision(), revision);

    let missing = "footprint-999999";
    for (command, mut params) in [
        (ALIGN_FOOTPRINTS, json!({"board":board,"footprints":[ids[0],missing],"mode":"left"})),
        (DELETE_FOOTPRINTS, json!({"board":board,"footprints":[ids[0],missing]})),
        (DELETE, json!({"board":board,"items":{"footprints":[ids[0],missing]}})),
    ] {
        params["expected_revision"] = json!(revision);
        assert_eq!(locked.execute(command, &params).unwrap_err().code(), crate::error_codes::ErrorCode::NotFound, "{command}");
    }
    let missing_move = json!({"board":board,"items":{"footprints":[ids[0],missing]},"dx_nm":1_000_000,"dy_nm":0,"expected_revision":revision});
    assert_eq!(
        locked.execute(MOVE_ITEMS, &missing_move).unwrap_err(),
        crate::CommandError::InvalidParams("Select existing objects on this board to move.".into())
    );
    assert!(Arc::ptr_eq(&before, &locked.snapshot().unwrap()));
    assert_eq!(locked.revision(), revision);
    locked.apply("edit.redo", &Value::Null).unwrap();
    locked.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &locked.snapshot().unwrap()));

    let both = with_placement_lock(&marked, original.board.footprints[2].id, true);
    locked.open_snapshot(both);
    let error = locked
        .execute(ALIGN_FOOTPRINTS, &json!({"board":board,"footprints":ids,"mode":"left","expected_revision":locked.revision()}))
        .unwrap_err()
        .to_string();
    assert!(error.contains(&expect_name) && error.contains(&format!("{other_ref} ({})", ids[2])), "{error}");
    let revision = locked.revision();
    locked
        .execute(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":board,"footprint":ids[1],"x_nm":10_000_000,"y_nm":9_000_000,
            "rotation":0,"expected_revision":revision}),
        )
        .unwrap();
    assert_eq!(locked.revision(), revision + 1);
}

/// Count selected lookup and geometry work through live commands and real preview capture.
#[test]
fn sparse_placement_edits_bound_lookups_and_bounds_on_large_boards() {
    use crate::{ALIGN_FOOTPRINTS, PLACE_FOOTPRINT, PLACE_RESISTOR};
    use clipboard::BoardSelection;
    use movement::MovePlan;

    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    for i in 0..512 {
        let (x_nm, y_nm) = match i {
            0 => (0, 0),
            255 => (10_000_000, 0),
            511 => (20_000_000, 0),
            _ => (i % 32 * 5_000_000, (i / 32 + 2) * 5_000_000),
        };
        session.apply(PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":y_nm,"expected_revision":session.revision()})).unwrap();
        let symbol = session.project().unwrap().symbols().last().unwrap().id();
        session
            .apply(
                PLACE_FOOTPRINT,
                &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":x_nm,"y_nm":y_nm,
                    "expected_revision":session.revision()}),
            )
            .unwrap();
    }
    let original = session.snapshot().unwrap();
    assert_eq!(original.board.footprints.len(), 512);
    let target = original.board_target().unwrap();
    let ids = [0, 255, 511].map(|i| format!("footprint-{}", original.board.footprints[i].id));
    let selection = BoardSelection { footprints: ids.iter().cloned().collect(), ..Default::default() };
    let measure = |run: &mut dyn FnMut()| {
        PLACEMENT_LOOKUP_PROBES.set(0);
        PLACEMENT_GEOMETRY_VISITS.set(0);
        run();
        (PLACEMENT_LOOKUP_PROBES.get(), PLACEMENT_GEOMETRY_VISITS.get())
    };
    // No movement is needed; the live commands still resolve the selected bounds.
    // Test the unchanged snapshot to exclude publication and copper rebuilds from this bound.
    for snapshot in [original.clone(), with_placement_lock(&original, original.board.footprints[511].id, true)] {
        let locked = snapshot.board.footprints[511].locked;
        session.open_snapshot(snapshot.clone());
        for mode in ["top", "distribute_x"] {
            let params = json!({"board":target.board,"footprints":ids,"mode":mode,"expected_revision":session.revision()});
            let (probes, resolved) = measure(&mut || {
                let result = session.execute(ALIGN_FOOTPRINTS, &params);
                if locked {
                    let error = result.unwrap_err();
                    assert!(error.to_string().contains(&ids[2]), "{mode}: {error}");
                } else {
                    result.unwrap();
                }
            });
            assert!((3..=72).contains(&probes), "{mode}: {probes} ID probes");
            assert_eq!(resolved, if locked { 0 } else { 3 }, "{mode}: visit only selected bounds");
            assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()), "{mode}");
        }
        let (probes, resolved) = measure(&mut || {
            let result = MovePlan::new(snapshot.clone(), target, selection.clone());
            if locked {
                let error = result.unwrap_err();
                assert!(error.to_string().contains(&ids[2]), "{error}");
            } else {
                result.unwrap();
            }
        });
        assert!((3..=36).contains(&probes), "MovePlan: {probes} ID probes");
        assert_eq!(resolved, 0, "no track ends to capture on the unrouted fixture");
    }
}

/// Create a real Redo entry without changing the active snapshot or its existing Undo entry.
fn add_redo_control(session: &mut Session) {
    let before = session.snapshot().unwrap();
    session.apply("project.rename", &json!({"name":"Redo control"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Check preservation of the snapshot and both populated history stacks on refusal, including
/// direct Apply Cut, which skips Execute's clipboard preflight.
fn assert_clipboard_refusal(session: &mut Session, command: &str, args: &Value, apply_directly: bool) {
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = session.revision();
    let root = session.history_root_revision();
    let undo = session.undo.clone();
    let redo = session.redo.clone();
    assert!(undo.iter().any(Option::is_some) && redo.iter().any(Option::is_some), "{command} must exercise both history stacks");
    let error = if apply_directly { session.apply(command, args).unwrap_err() } else { session.execute(command, args).unwrap_err() };
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{command}: {error}");
    assert!(error.to_string().contains("footprint locks are not supported by the board clipboard format"), "{command}: {error}");
    let after = session.snapshot().unwrap();
    assert!(Arc::ptr_eq(&before, &after), "{command}");
    assert_eq!(encode_documents(after).unwrap(), bytes, "{command}");
    assert_eq!(session.revision(), revision, "{command}");
    assert_eq!(session.history_root_revision(), root, "{command}");
    for (previous, current) in [(&undo, &session.undo), (&redo, &session.redo)] {
        assert_eq!(previous.len(), current.len(), "{command}");
        for (before, after) in previous.iter().zip(current) {
            match (before, after) {
                (Some(before), Some(after)) => assert!(Arc::ptr_eq(before, after), "{command}"),
                (None, None) => (),
                _ => panic!("{command} changed a history entry"),
            }
        }
    }
}

/// A historical marker is encodable and readable until only its footprint lock flag changes.
#[test]
fn diagnostic_codec_refuses_footprint_locks() {
    use crate::persistence::{decode, encode};
    let original = diagnostic_snapshot(marker_fixture().snapshot().unwrap());
    let id = original.board.footprints[0].id;
    let bytes = encode(original.clone()).unwrap();
    let reopened = decode(&bytes).unwrap();
    assert_eq!(reopened.board.footprints, original.board.footprints);
    let error = encode(with_placement_lock(&original, id, true)).unwrap_err();
    assert!(error.contains("footprint locks") && error.contains("use split documents"), "{error}");
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["board"]["footprints"][0]["id"], id);
    document["board"]["footprints"][0]["locked"] = json!(true);
    let error = decode(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert!(error.contains("footprint locks require split board format 34"), "{error}");
    document["board"]["footprints"][0]["locked"] = Value::Null;
    let error = decode(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert!(error.contains("invalid project JSON") && error.contains("bool"), "{error}");
}
