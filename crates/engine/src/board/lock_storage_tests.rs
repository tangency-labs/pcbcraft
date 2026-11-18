//! Storage-only footprint lock coverage. Valid geometry is authored through production commands;
//! private snapshot edits supply metadata until a separate lock command/enforcement slice exists.
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
/// with and without setup. Unlocked neighbors and unrelated documents retain their canonical bytes.
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
    ] {
        marker["format"] = json!(format!("pcbcraft-board/{version}"));
        incomplete.insert(marker_path.clone(), serde_json::to_vec(&marker).unwrap());
        let error = decode_documents(&incomplete).unwrap_err();
        assert!(error.contains(reason), "{version}: {error}");
    }
    // Reservations are not implemented formats, even when no lock or other extension is present.
    for version in 26..=33 {
        marker["format"] = json!(format!("pcbcraft-board/{version}"));
        incomplete.insert(marker_path.clone(), serde_json::to_vec(&marker).unwrap());
        let error = decode_documents(&incomplete).unwrap_err();
        assert!(error.contains("unsupported board format"), "{version}: {error}");
    }
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
/// the test fixture restores the original bytes. Invalid profile, report and via records still fail.
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
                        assert_invalid_board_record(&saved, &invalid, "a via needs a non-nil identity, a bounded position");
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

/// Clipboard refuses an otherwise valid multi-footprint fragment with one non-first active lock.
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
        assert_clipboard_refusal(&mut locked, command, &args);
    }
    assert_placement_locks(&original, &locked.snapshot().unwrap(), locked_id);

    add_redo_control(&mut source);
    let paste = json!({"board":board,"text":cut_text,"reuse_components":true,"x_nm":0,"y_nm":10_000_000,"expected_revision":source.revision()});
    let mut envelope: Value = serde_json::from_str(cut_text).unwrap();
    assert_eq!(envelope["footprints"][1]["id"], locked_id);
    envelope["footprints"][1]["locked"] = json!(true);
    let mut bad_paste = paste.clone();
    bad_paste["text"] = json!(serde_json::to_string(&envelope).unwrap());
    assert_clipboard_refusal(&mut source, PASTE, &bad_paste);
    source.execute(PASTE, &paste).unwrap();
    assert_eq!(source.project().unwrap().board.footprints.len(), original.board.footprints.len());
}

/// Create a real Redo entry without changing the active snapshot or its existing Undo entry.
fn add_redo_control(session: &mut Session) {
    let before = session.snapshot().unwrap();
    session.apply("project.rename", &json!({"name":"Clipboard redo control"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// A footprint-specific command error must preserve the snapshot and both populated history stacks.
fn assert_clipboard_refusal(session: &mut Session, command: &str, args: &Value) {
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = session.revision();
    let root = session.history_root_revision();
    let undo = session.undo.clone();
    let redo = session.redo.clone();
    assert!(undo.iter().any(Option::is_some) && redo.iter().any(Option::is_some), "{command} must exercise both history stacks");
    let error = session.execute(command, args).unwrap_err();
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
