//! Compatibility with released profile, stack, via-intent, material-report and edit-observer contracts.
use super::*;
use crate::edits::{CommittedEdit, EditObserver};
use std::sync::Mutex;

/// Author a pinned profile and its reviewed adoption before adding unsupported geometry.
fn adopt_profile(s: &mut Session) {
    let profile = Uuid::from_u128(0xa550);
    s.apply(
        crate::SET_ASSEMBLY_PROFILE,
        &json!({"profile":profile,"expected_revision":s.revision(),
        "content":{"revision":1,"name":"Arc fixture","provenance":{"kind":"authored"},"bottom_side":"viewed_from_top"}}),
    )
    .unwrap();
    let mut request = json!({"board":view(s).target.board,"profile":profile,"revision":1,"expected_revision":s.revision()});
    let preview = s.execute(crate::PREVIEW_ASSEMBLY_PROFILE, &request).unwrap();
    request["acceptance"] = preview["acceptance"].clone();
    s.apply(crate::ADOPT_ASSEMBLY_PROFILE, &request).unwrap();
}

/// Use the existing setup command to add extended stack metadata without replacing layer IDs.
fn extend_stack(s: &mut Session) {
    let mut setup = json!(s.project().unwrap().board_setup().unwrap());
    setup["stackup"]["strata"][0]["role"] = json!("signal");
    setup["stackup"]["strata"][0]["base_copper"] = json!({"weight_milli_oz":1000,"thickness_nm":35_000});
    setup["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    s.apply(crate::SET_BOARD_SETUP, &json!({"board":view(s).target.board,"setup":setup,"expected_revision":s.revision()})).unwrap();
}

/// An isolated explicit via has authored net intent but no physical pad connection.
fn assigned_via(s: &mut Session) {
    let registry = s.project().unwrap().inspect_net_registry().unwrap();
    s.apply(
        crate::ADD_VIA,
        &json!({"board":view(s).target.board,"at":{"x_nm":-30_000_000,"y_nm":-30_000_000},
        "target":{"kind":"derived","anchor":registry["nets"][0]["anchor"]},"expected_revision":s.revision()}),
    )
    .unwrap();
    assert!(view(s).vias()[0].net_assignment.is_some());
    assert_eq!(view(s).inspect_vias()[0]["connected_to_pad"], false);
    assert!(view(s).inspect_vias()[0]["net"].is_string());
}

/// Every nonempty combination of released profile, stack and via intent survives arcs and removal.
#[test]
fn arcs_preserve_released_22_23_24_records_and_exact_lower_bytes() {
    for (profile, stack, via, format) in [
        (true, false, false, "pcbcraft-board/22"),
        (false, true, false, "pcbcraft-board/23"),
        (false, false, true, "pcbcraft-board/24"),
        (true, true, false, "pcbcraft-board/23"),
        (true, false, true, "pcbcraft-board/24"),
        (false, true, true, "pcbcraft-board/24"),
        (true, true, true, "pcbcraft-board/24"),
    ] {
        let (mut s, arc) = fixture();
        if profile {
            adopt_profile(&mut s);
        }
        if stack {
            extend_stack(&mut s);
        }
        if via {
            assigned_via(&mut s);
        }
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let path = s.project().unwrap().board_document_path().unwrap();
        let previous: Value = serde_json::from_slice(&before[&path]).unwrap();
        assert_eq!(previous["format"], format);
        s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
        let saved = encode_documents(s.snapshot().unwrap()).unwrap();
        let mut current: Value = serde_json::from_slice(&saved[&path]).unwrap();
        assert_eq!(current["format"], "pcbcraft-board/26");
        current.as_object_mut().unwrap().remove("arcs");
        current["format"] = previous["format"].clone();
        assert_eq!(current, previous, "all non-arc board fields retained");
        for (name, bytes) in &before {
            if name != &path {
                assert_eq!(saved.get(name), Some(bytes), "{name}");
            }
        }
        assert_eq!(saved.len(), before.len());
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
        let packed = encode_packed(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
        s.apply(SET_ARCS, &params(&s, &[])).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
}

const REPORT_MATERIAL_ID: Uuid = Uuid::from_u128(0xa551);

/// Append a synthetic material through the setup command, including an omitted empty catalog.
fn add_material_reports(s: &mut Session) {
    let mut setup = json!(s.project().unwrap().board_setup().unwrap());
    let source = json!({"document":"Synthetic compatibility report"});
    setup.as_object_mut().unwrap().entry("materials").or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!({
        "id":REPORT_MATERIAL_ID,"revision":1,"name":"Arc report fixture","family":"Synthetic",
        "thermal_conductivity":[{"conductivity_mw_per_m_k":1,"basis":"typical","source":source}],
        "cte":[{"cte_ppb_per_k":1,"from_milli_c":0,"to_milli_c":1,"basis":"typical","source":source}],
        "dielectric_strength":[{"breakdown_v":1,"specimen_thickness_nm":1,"basis":"typical","source":source}],
        "cti":[{"cti_v":1,"basis":"typical","source":source}]
    }));
    s.apply(crate::SET_BOARD_SETUP, &json!({"board":view(s).target.board,"setup":setup,"expected_revision":s.revision()})).unwrap();
}

/// Report format 25 remains byte-exact after arcs, including every optional profile/stack/via combination.
#[test]
fn arcs_preserve_released_25_reports_and_exact_lower_bytes() {
    for (profile, stack, via) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, false),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let (mut s, arc) = fixture();
        if profile {
            adopt_profile(&mut s);
        }
        if stack {
            extend_stack(&mut s);
        }
        if via {
            assigned_via(&mut s);
        }
        add_material_reports(&mut s);
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let path = s.project().unwrap().board_document_path().unwrap();
        let previous: Value = serde_json::from_slice(&before[&path]).unwrap();
        assert_eq!(previous["format"], "pcbcraft-board/25");
        assert_eq!(encode_documents(decode_documents(&before).unwrap()).unwrap(), before);
        s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
        let saved = encode_documents(s.snapshot().unwrap()).unwrap();
        let mut current: Value = serde_json::from_slice(&saved[&path]).unwrap();
        assert_eq!(current["format"], "pcbcraft-board/26");
        current.as_object_mut().unwrap().remove("arcs");
        current["format"] = previous["format"].clone();
        assert_eq!(current, previous, "all non-arc board fields, including reports, retained");
        for (name, bytes) in &before {
            if name != &path {
                assert_eq!(saved.get(name), Some(bytes), "{name}");
            }
        }
        assert_eq!(saved.len(), before.len());
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
        let packed = encode_packed(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
        s.apply(SET_ARCS, &params(&s, &[])).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
}

/// Arc-free setup validates reports; arcs refuse setup edits while native decoding retains every validator and header obligation.
#[test]
fn arc_report_superset_retains_header_and_material_validation() {
    let (mut s, arc) = fixture();
    assigned_via(&mut s);
    let plain = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = s.project().unwrap().board_document_path().unwrap();
    let mut plain_board: Value = serde_json::from_slice(&plain[&path]).unwrap();
    assert_eq!(plain_board["format"], "pcbcraft-board/24");
    add_material_reports(&mut s);
    let reported = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut reported_board: Value = serde_json::from_slice(&reported[&path]).unwrap();
    assert_eq!(reported_board["format"], "pcbcraft-board/25");
    let reported_snapshot = s.snapshot().unwrap();
    let reported_state = s.inspect();
    let mut invalid_reports = Vec::new();
    for (field, member, value, reason) in [
        ("thermal_conductivity", "conductivity_mw_per_m_k", json!(0), "thermal conductivity must be"),
        ("cte", "to_milli_c", json!(0), "a thermal expansion temperature range"),
        ("dielectric_strength", "specimen_thickness_nm", json!(0), "specimen thickness must be"),
        ("cti", "cti_v", json!(0), "CTI must be"),
    ] {
        let mut candidate = reported_board.clone();
        let material =
            candidate["setup"]["materials"].as_array_mut().unwrap().iter_mut().find(|material| material["id"] == json!(REPORT_MATERIAL_ID)).unwrap();
        material[field][0][member] = value;
        let request = json!({"board":view(&s).target.board,"setup":candidate["setup"],"expected_revision":s.revision()});
        assert_invalid_params(s.apply(crate::SET_BOARD_SETUP, &request).unwrap_err(), reason);
        assert!(Arc::ptr_eq(&reported_snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.inspect(), reported_state);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), reported);
        invalid_reports.push((candidate, reason));
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), plain);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), reported);
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let snapshot = s.snapshot().unwrap();
    let state = s.inspect();
    let saved = encode_documents(snapshot.clone()).unwrap();
    let board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/26");
    let mut invalid = Vec::new();
    plain_board["format"] = json!("pcbcraft-board/25");
    invalid.push((plain_board, "format 25 requires at least one of them"));
    reported_board["format"] = json!("pcbcraft-board/24");
    invalid.push((reported_board, "reports require board format 25, 26, 34, 35 or 40"));
    let mut candidate = board.clone();
    candidate["format"] = json!("pcbcraft-board/25");
    invalid.push((candidate, "arcs require format 26"));
    let mut candidate = board.clone();
    candidate.as_object_mut().unwrap().remove("arcs");
    invalid.push((candidate, "format 26 requires arcs"));
    for (mut candidate, reason) in invalid_reports {
        candidate["format"] = json!("pcbcraft-board/26");
        candidate["arcs"] = board["arcs"].clone();
        let request = json!({"board":view(&s).target.board,"setup":candidate["setup"],"expected_revision":s.revision()});
        assert_unavailable(s.apply(crate::SET_BOARD_SETUP, &request).unwrap_err(), UNSUPPORTED);
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.inspect(), state);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
        invalid.push((candidate, reason));
    }
    for (candidate, reason) in invalid {
        let mut documents = saved.clone();
        documents.insert(path.clone(), serde_json::to_vec(&candidate).unwrap());
        let error = decode_documents(&documents).unwrap_err();
        assert!(error.contains(reason), "{reason}: {error}");
    }
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), state);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), reported);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Unsupported membership is unknown, not falsely disconnected or stripped of authored intent.
#[test]
fn arc_via_inspection_retains_authored_state_and_reports_unknown_geometry() {
    let (mut s, arc) = fixture();
    assigned_via(&mut s);
    let previous = view(&s).inspect_vias()[0].clone();
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let current = view(&s).inspect_vias()[0].clone();
    assert_eq!(current["net_assignment"], previous["net_assignment"]);
    assert_eq!(current["assignment_mode"], "explicit");
    assert_eq!(current["net_state"], "active");
    assert_eq!(current["net"], Value::Null);
    assert_eq!(current["connected_to_pad"], Value::Null);
    assert_eq!(current["geometry_error"], UNSUPPORTED);
    assert_eq!(s.inspect()["project"]["boards"][0]["vias"][0], current);
    s.apply(SET_ARCS, &params(&s, &[])).unwrap();
    assert_eq!(view(&s).inspect_vias()[0], previous);
}

/// Released route planning and via assignment refuse before publishing straight-only changes.
#[test]
fn released_physical_commands_refuse_arcs_at_current_revision() {
    let (mut s, arc) = fixture();
    assigned_via(&mut s);
    let via = view(&s).vias()[0].id;
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for (command, mut request) in [
        (crate::board::route::PLAN_ROUTE, json!({"layer":arc.layer,"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":10_000_000,"y_nm":0}})),
        (crate::SET_VIA_NET, json!({"vias":[via],"automatic":true})),
    ] {
        request["board"] = json!(view(&s).target.board);
        request["expected_revision"] = json!(revision);
        assert_unavailable(s.execute(command, &request).unwrap_err(), UNSUPPORTED);
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    }
}

/// Format 26 retains the released validation of optional profile, stack and via fields.
#[test]
fn arc_native_superset_still_validates_released_optional_fields() {
    let (mut s, arc) = fixture();
    adopt_profile(&mut s);
    extend_stack(&mut s);
    assigned_via(&mut s);
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = s.project().unwrap().board_document_path().unwrap();
    let board: Value = serde_json::from_slice(&saved[&path]).unwrap();
    let mut invalid = Vec::new();
    let mut candidate = board.clone();
    candidate["assembly_profiles"] = json!([]);
    invalid.push((candidate, "empty assembly_profiles"));
    let mut candidate = board.clone();
    candidate["assembly_profiles"][0]["revision"] = json!(2);
    invalid.push((candidate, "reference pinned content revisions"));
    let mut candidate = board.clone();
    candidate["setup"]["stackup"]["strata"][0]["role"] = json!("invalid-role");
    invalid.push((candidate, "invalid-role"));
    let mut candidate = board.clone();
    candidate["setup"]["stackup"]["finished_thickness_nm"] = json!(0);
    invalid.push((candidate, "finished board thickness must be a positive"));
    let mut candidate = board.clone();
    candidate["vias"][0]["net_assignment"] = Value::Null;
    invalid.push((candidate, "null"));
    let mut candidate = board;
    candidate["vias"][0]["net_assignment"]["net"] = json!(Uuid::nil());
    invalid.push((candidate, "non-nil identities"));
    for (candidate, reason) in invalid {
        let mut documents = saved.clone();
        documents.insert(path.clone(), serde_json::to_vec(&candidate).unwrap());
        let error = decode_documents(&documents).unwrap_err();
        assert!(error.contains(reason), "{reason}: {error}");
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    }
}

#[derive(Debug, Default)]
struct ArcLog(Mutex<Vec<CommittedEdit>>);
impl EditObserver for ArcLog {
    /// Root notifications are independent of command publications under test.
    fn opened(&self, _project: Arc<Project>) {}
    /// Retain the adopted project, exact command parameters and published revision.
    fn committed(&self, edit: CommittedEdit) {
        self.0.lock().unwrap().push(edit);
    }
}

/// Both dispatch paths notify once after adoption, and never for dry runs, no-ops or refusals.
#[test]
fn arc_apply_and_execute_notify_the_released_observer_exactly_once() {
    for execute in [false, true] {
        let (mut s, arc) = fixture();
        let log = Arc::new(ArcLog::default());
        s.observe_edits(Some(log.clone()));
        let request = params(&s, std::slice::from_ref(&arc));
        if execute {
            s.execute(SET_ARCS, &request).unwrap();
        } else {
            s.apply(SET_ARCS, &request).unwrap();
        }
        {
            let events = log.0.lock().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].command, SET_ARCS);
            assert_eq!(events[0].params, request);
            assert_eq!(events[0].revision, s.revision());
            assert!(Arc::ptr_eq(&events[0].project, &s.snapshot().unwrap()));
        }
        let unchanged = params(&s, std::slice::from_ref(&arc));
        s.apply(SET_ARCS, &unchanged).unwrap();
        s.execute(SET_ARCS, &unchanged).unwrap();
        let mut dry = params(&s, &[]);
        dry["dry_run"] = json!(true);
        s.apply(SET_ARCS, &dry).unwrap();
        s.execute(SET_ARCS, &dry).unwrap();
        let mut invalid = arc;
        invalid.width_nm = 0;
        let bad = params(&s, &[invalid]);
        assert_invalid_params(s.apply(SET_ARCS, &bad).unwrap_err(), "invalid circular arc geometry");
        assert_invalid_params(s.execute(SET_ARCS, &bad).unwrap_err(), "invalid circular arc geometry");
        assert_eq!(log.0.lock().unwrap().len(), 1);
        let remove = params(&s, &[]);
        if execute {
            s.execute(SET_ARCS, &remove).unwrap();
        } else {
            s.apply(SET_ARCS, &remove).unwrap();
        }
        let events = log.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].params, remove);
        assert!(events[1].project.boards().next().unwrap().arc_tracks().is_empty());
        assert_eq!(events[1].revision, s.revision());
    }
}
