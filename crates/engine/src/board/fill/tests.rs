use super::*;
use crate::{
    Session,
    board::{
        track::tests::{copper_layers, routed_fixture, view},
        zones::{ADD_ZONE, Options},
    },
};
use serde_json::json;

#[test]
fn headless_refill_and_status_are_revision_checked_derived_results() {
    let mut s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let revision = s.revision();
    let native = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let pending = s.execute(super::INSPECT_FILL, &json!({"board":target.board})).unwrap();
    assert_eq!(pending["zone_fill"]["result"]["state"], "pending");
    let ready = s.execute(super::REFILL_ZONES, &json!({"board":target.board})).unwrap();
    assert_eq!(ready["zone_fill"]["result"]["state"], "ready");
    assert_eq!(ready["revision"], revision);
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), native);
}

#[test]
fn browser_worker_uses_the_native_source_codec_and_rejects_foreign_results() {
    let s = fixture(Connection::Solid, 0);
    let mut job = s.prepare_zone_fill(view(&s).target, false).unwrap();
    let request = job.worker_request().unwrap();
    let mut completed = Vec::new();
    let response = super::worker::execute_reporting(&request, &mut |count| completed.push(count));
    assert_eq!(completed, vec![1]);
    let filled = job.accept_worker_response(&response).unwrap().unwrap();
    assert!(view(&s).ratsnest_with_copper(&filled.copper).unwrap().lines.len() < 3);
    let mut wrong: serde_json::Value = serde_json::from_str(&response).unwrap();
    wrong["source"] = json!("another immutable source");
    let mut job = s.prepare_zone_fill(view(&s).target, true).unwrap();
    let _ = job.worker_request().unwrap();
    assert!(job.accept_worker_response(&wrong.to_string()).is_err());
    assert!(view(&s).filled_zones().is_none());
}

/// Native progress advances during the evaluation. Canceling from a progress callback
/// leaves the runtime slot empty and keeps authored bytes untouched.
#[test]
fn fill_job_reports_completed_zones_before_publishing() {
    let mut s = fixture(Connection::Solid, 0);
    let (target, points) = {
        let view = view(&s);
        let geometry = BoardGeometry::new(view);
        let pad = obstacles(view, &geometry).unwrap().into_iter().find(|o| o.pad && o.net == Some(view.zones()[0].net)).unwrap();
        (view.net_class_target_for_copper(&pad.object.name).unwrap().unwrap(), view.zones()[0].points.clone())
    };
    let layer = copper_layers(&s)[1];
    s.apply(ADD_ZONE, &json!({"target":target,"layer":layer,"points":points,"expected_revision":s.revision()})).unwrap();
    let board = view(&s).target;
    let mut job = s.prepare_zone_fill(board, false).unwrap();
    assert_eq!((job.progress().completed, job.progress().total), (0, 2));
    let cancel = job.cancellation();
    let mut progress = Vec::new();
    assert!(job.step_reporting(1, &mut |p| {
        progress.push((p.completed, p.total));
        cancel.store(true, std::sync::atomic::Ordering::Release);
    }));
    assert_eq!(progress, vec![(1, 2)]);
    assert!(job.take_result().unwrap().is_none());
    assert!(view(&s).filled_zones().is_none());

    let mut retry = s.prepare_zone_fill(board, false).unwrap();
    progress.clear();
    assert!(retry.step_reporting(1, &mut |p| progress.push((p.completed, p.total))));
    assert_eq!(progress, vec![(1, 2), (2, 2)]);
    assert_eq!(retry.take_result().unwrap().unwrap().zones.len(), 2);
}

#[test]
fn drc_uses_current_fill_and_stricter_diagnostics_do_not_replace_it() {
    let s = fixture(Connection::Solid, 0);
    let v = view(&s);
    let normal = v.check_design(v.effective_rules().values);
    assert_eq!(normal.counts.get(&super::super::drc::Kind::ZoneFillUnavailable).copied().unwrap_or(0), 0, "{normal:#?}");
    let before = v.filled_zones().unwrap().unwrap();
    let stricter = v.check_design(super::super::rules::Values { clearance_nm: 500_000, ..v.effective_rules().values });
    assert!(stricter.counts.get(&super::super::drc::Kind::Clearance).copied().unwrap_or(0) > 0);
    assert!(Arc::ptr_eq(&before, &v.filled_zones().unwrap().unwrap()));
}

#[test]
fn higher_priority_foreign_fill_reserves_the_larger_zone_clearance() {
    let mut s = fixture(Connection::Solid, 0);
    let (net, layer) = {
        let v = view(&s);
        (v.zones()[0].net, v.zones()[0].layer)
    };
    let geometry = BoardGeometry::new(view(&s));
    let other = obstacles(view(&s), &geometry).unwrap().into_iter().find(|o| o.pad && o.net.is_some() && o.net != Some(net)).unwrap();
    let target = view(&s).net_class_target_for_copper(&other.object.name).unwrap().unwrap();
    let points =
        [[8_000_000, 5_000_000], [16_000_000, 5_000_000], [16_000_000, 15_000_000], [8_000_000, 15_000_000]].map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    s.apply(ADD_ZONE,&json!({"target":target,"layer":layer,"points":points,"options":Options{priority:1,connection:Connection::Solid,clearance_nm:Some(800_000),..Options::default()},"expected_revision":s.revision()})).unwrap();
    let result = compute(view(&s)).unwrap();
    assert_eq!(result.zones.len(), 2);
    for high in &result.zones[0].islands {
        for low in &result.zones[1].islands {
            assert!(clip(&low.region, &high.region.offset(800_000).unwrap(), Operation::Intersection).unwrap().is_empty());
        }
    }
}

/// Add authored intent through production commands to the accepted physical fixture.
fn fixture(connection: Connection, min_area: u64) -> Session {
    let mut s = routed_fixture();
    super::super::drc::tests::outline(&mut s);
    let pad = {
        let v = view(&s);
        let line = v.ratsnest().unwrap().lines[0].clone();
        format!("{}:{}", line.from.footprint, line.from.pad.number.as_str())
    };
    let target = view(&s).net_class_target_for_copper(&pad).unwrap().unwrap();
    let layer = copper_layers(&s)[0];
    let points = [[-5_000_000, -5_000_000], [25_000_000, -5_000_000], [25_000_000, 35_000_000], [-5_000_000, 35_000_000]]
        .map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    s.apply(ADD_ZONE,&json!({"layer":layer,"target":target,"points":points,"options":Options{connection,min_island_area_nm2:min_area,..Options::default()},"expected_revision":s.revision()})).unwrap();
    s
}

#[test]
fn solid_fill_obeys_foreign_clearance_and_joins_same_net_pads() {
    let s = fixture(Connection::Solid, 0);
    let v = view(&s);
    let before = v.ratsnest().unwrap().lines.len();
    let result = compute(v).unwrap();
    assert!(result.problems.is_empty(), "{:?}", result.problems);
    assert!(!result.zones[0].islands.is_empty());
    let geometry = BoardGeometry::new(v);
    let objects = obstacles(v, &geometry).unwrap();
    for island in &result.zones[0].islands {
        for other in objects.iter().filter(|o| o.net != Some(result.zones[0].net) && o.object.layers.contains(&result.zones[0].layer)) {
            assert!(
                clip(&island.region, &region(&other.object.contour, v.effective_rules().values.clearance_nm).unwrap(), Operation::Intersection)
                    .unwrap()
                    .is_empty(),
                "foreign clearance to {}",
                other.object.name
            );
        }
    }
    assert!(v.ratsnest_with_copper(&result.copper).unwrap().lines.len() < before);
}

#[test]
fn thermal_spokes_connect_and_no_connect_style_does_not() {
    let thermal = fixture(Connection::Thermal, 0);
    let v = view(&thermal);
    let filled = compute(v).unwrap();
    assert!(v.ratsnest_with_copper(&filled.copper).unwrap().lines.len() < v.ratsnest().unwrap().lines.len());
    let isolated = fixture(Connection::None, 0);
    let v = view(&isolated);
    let filled = compute(v).unwrap();
    assert_eq!(v.ratsnest_with_copper(&filled.copper).unwrap().lines.len(), v.ratsnest().unwrap().lines.len());
}

/// A contact-preserving move checks only direct pad, track and via contacts. Pour connectivity
/// is derived from a fill, so a move that takes the zone off pads it joined is accepted as one
/// edit, and the next refill shows the pads unjoined again.
#[test]
fn contact_preserving_move_leaves_pour_connectivity_to_refill() {
    let mut s = fixture(Connection::Solid, 0);
    let unfilled = view(&s).ratsnest().unwrap().lines.len();
    let refilled = |s: &Session| {
        let v = view(s);
        v.ratsnest_with_copper(&compute(v).unwrap().copper).unwrap().lines.len()
    };
    assert!(refilled(&s) < unfilled, "the pour joins same-net pads");
    let (board, zone) = (view(&s).target.board, view(&s).zones()[0].id);
    let revision = s.revision();
    // The pads sit at y = 2 mm; 30 mm up leaves the zone over none of them.
    let moved = s.apply(
        crate::board::movement::MOVE_ITEMS,
        &json!({"board":board,"items":{"zones":[zone]},"dx_nm":0,"dy_nm":30_000_000,"preserve_contacts":true,"expected_revision":revision}),
    );
    assert!(moved.is_ok(), "{moved:?}");
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(refilled(&s), unfilled, "the refill no longer joins the pads");
}

#[test]
fn dropping_material_does_not_prove_connections_and_saved_intent_is_unchanged() {
    let s = fixture(Connection::Solid, 5_000_000_000_000_000);
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let v = view(&s);
    let filled = compute(v).unwrap();
    assert!(filled.zones[0].islands.is_empty());
    assert!(filled.zones[0].dropped > 0);
    assert!(!filled.problems.is_empty());
    assert_eq!(v.ratsnest_with_copper(&filled.copper).unwrap().lines.len(), v.ratsnest().unwrap().lines.len());
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

#[test]
fn a_completed_source_slot_updates_connectivity_without_a_document_edit() {
    let s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let unfilled = view(&s).ratsnest().unwrap().lines.len();
    let mut job = s.prepare_zone_fill(target, false).unwrap();
    assert!(view(&s).filled_zones().is_none());
    assert!(job.step(1));
    assert!(job.take_result().unwrap().is_some());
    assert!(view(&s).filled_zones().unwrap().is_ok());
    assert!(view(&s).ratsnest().unwrap().lines.len() < unfilled);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

#[test]
fn disposable_preview_reads_do_not_evict_the_live_source_slot() {
    let s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let mut job = s.prepare_zone_fill(target, false).unwrap();
    let mut preview = s.snapshot().unwrap().as_ref().clone();
    Arc::make_mut(&mut preview.board).zones[0].options.priority += 1;
    assert!(preview.board_view(target).unwrap().filled_zones().is_none());
    assert!(job.step(1));
    let completed = job.take_result().unwrap().unwrap();
    assert!(Arc::ptr_eq(&view(&s).filled_zones().unwrap().unwrap(), &completed));
    assert!(preview.board_view(target).unwrap().filled_zones().is_none());
    assert!(Arc::ptr_eq(&view(&s).filled_zones().unwrap().unwrap(), &completed));
}

#[test]
fn cancellation_and_replaced_sources_never_publish_partial_or_current_copper() {
    let mut s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let mut canceled = s.prepare_zone_fill(target, false).unwrap();
    canceled.cancellation().store(true, Ordering::Release);
    assert!(canceled.step(1));
    assert!(canceled.take_result().unwrap().is_none());
    assert!(view(&s).filled_zones().is_none());
    let mut old = s.prepare_zone_fill(target, false).unwrap();
    let zone = view(&s).zones()[0].id;
    s.apply(
        super::super::zones::SET_ZONE,
        &json!({"zone":zone,"options":Options{connection:Connection::None,..Options::default()},"expected_revision":s.revision()}),
    )
    .unwrap();
    let new = s.prepare_zone_fill(target, false).unwrap();
    assert!(!old.source().is_current(&s));
    assert!(new.source().is_current(&s));
    assert!(old.step(1));
    assert!(old.take_result().unwrap().is_some());
    assert!(view(&s).filled_zones().is_none());
}

/// A cold browser check prepares all analytic zone work in isolation, then resumes the
/// ordinary incremental checker with identical findings and request provenance.
#[test]
fn isolated_zone_diagnostics_agree_with_the_complete_checker() {
    let s = fixture(Connection::Thermal, 0);
    let params = json!({"clearance_nm":500_000});
    let mut job = s.design_check_job(&params).unwrap();
    let request = job.browser_zone_request().unwrap().unwrap();
    let response = super::worker::execute(&request);
    job.accept_browser_zones(&response).unwrap();
    assert!(view(&s).filled_zones().unwrap().is_ok());
    while !job.step(7) {}
    assert_eq!(json!(job.take_report().unwrap()), s.check_design(&params).unwrap());
}

/// Authored changes, Undo/Redo and native reopening all select geometry from their own
/// immutable source; a saved document never revives a cached fill from another state.
#[test]
fn edits_undo_redo_and_reopen_invalidate_derived_material() {
    let mut s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let ready = s.execute(REFILL_ZONES, &json!({})).unwrap();
    assert_eq!(ready["zone_fill"]["result"]["state"], "ready");
    let connected = view(&s).ratsnest().unwrap().lines.len();
    let zone = view(&s).zones()[0].id;
    s.apply(
        super::super::zones::SET_ZONE,
        &json!({"zone":zone,"options":Options{connection:Connection::None,..Options::default()},"expected_revision":s.revision()}),
    )
    .unwrap();
    assert!(view(&s).filled_zones().is_none());
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    let disconnected = view(&s).ratsnest().unwrap().lines.len();
    assert!(disconnected > connected);
    s.apply("edit.undo", &json!(null)).unwrap();
    assert!(view(&s).filled_zones().is_none());
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), connected);
    s.apply("edit.redo", &json!(null)).unwrap();
    assert!(view(&s).filled_zones().is_none());
    let docs = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    s.open_snapshot(crate::persistence::decode_documents(&docs).unwrap());
    assert!(view(&s).filled_zones().is_none());
    s.execute(REFILL_ZONES, &json!({"board":target.board})).unwrap();
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), disconnected);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), docs);
}

/// Cancellation, altered worker zone ownership and malformed display indices cannot publish
/// apparently ready material even if the response repeats the right native-source fingerprint.
#[test]
fn worker_response_validation_and_cancellation_preserve_unavailable_state() {
    let s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let mut job = s.prepare_zone_fill(target, true).unwrap();
    let request = job.worker_request().unwrap();
    let response = super::worker::execute(&request);
    let wire: serde_json::Value = serde_json::from_str(&response).unwrap();
    for field in ["identity", "mesh"] {
        let mut bad = wire.clone();
        if field == "identity" {
            bad["result"]["Ok"]["zones"][0]["net"] = json!(Uuid::from_u128(u128::MAX));
        } else {
            bad["result"]["Ok"]["zones"][0]["islands"][0]["mesh"]["triangles"][0] = json!([u32::MAX, u32::MAX, u32::MAX]);
        }
        let mut candidate = s.prepare_zone_fill(target, true).unwrap();
        candidate.worker_request().unwrap();
        assert!(candidate.accept_worker_response(&bad.to_string()).is_err());
        assert!(!view(&s).filled_zones().is_some_and(|r| r.is_ok()));
    }
    let mut canceled = s.prepare_zone_fill(target, true).unwrap();
    canceled.worker_request().unwrap();
    canceled.cancellation().store(true, Ordering::Release);
    assert!(canceled.accept_worker_response(&response).is_err());
    assert!(view(&s).filled_zones().is_none());
}

/// A foreign route across a plane splits actual physical material; moving that same route
/// clears the barrier and drops the newly satisfied airwire after refill. Native reopening
/// regenerates identical analytic islands without saving any derived copper.
#[test]
fn moving_a_track_and_refilling_updates_material_and_airwires_deterministically() {
    let mut s = fixture(Connection::Solid, 0);
    let layer = copper_layers(&s)[0];
    s.apply(crate::ADD_TRACKS,&json!({"layer":layer,"width_nm":250_000,"points":[{"x_nm":12_000_000,"y_nm":-10_000_000},{"x_nm":12_000_000,"y_nm":40_000_000}],"expected_revision":s.revision()})).unwrap();
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    let split = view(&s).ratsnest().unwrap().lines.len();
    assert!(view(&s).filled_zones().unwrap().unwrap().zones[0].islands.len() >= 2);
    let track = view(&s).tracks()[0].id;
    s.apply(crate::MOVE_TRACKS, &json!({"tracks":[track],"dx_nm":30_000_000,"dy_nm":0,"expected_revision":s.revision()})).unwrap();
    assert!(view(&s).filled_zones().is_none());
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    assert!(view(&s).ratsnest().unwrap().lines.len() < split);
    let original = view(&s).filled_zones().unwrap().unwrap();
    let profiles = original.zones[0].islands.iter().map(|island| island.region.worker_bytes().unwrap()).collect::<Vec<_>>();
    let docs = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    s.open_snapshot(crate::persistence::decode_documents(&docs).unwrap());
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    let reopened = view(&s).filled_zones().unwrap().unwrap();
    assert_eq!(profiles, reopened.zones[0].islands.iter().map(|island| island.region.worker_bytes().unwrap()).collect::<Vec<_>>());
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), docs);
}

/// Terminal host failure is visible to every reader, while an obsolete source cannot mark
/// the replacement board unavailable or overwrite a successfully published result.
#[test]
fn host_failure_is_explicit_and_stale_failures_cannot_poison_new_work() {
    let mut s = fixture(Connection::Solid, 0);
    let old = s.prepare_zone_fill(view(&s).target, false).unwrap();
    assert!(s.fail_zone_fill(old.source(), "Worker could not load".into()));
    assert!(view(&s).filled_zones().unwrap().unwrap_err().contains("could not load"));
    let zone = view(&s).zones()[0].id;
    s.apply(
        super::super::zones::SET_ZONE,
        &json!({"zone":zone,"options":Options{connection:Connection::None,..Options::default()},"expected_revision":s.revision()}),
    )
    .unwrap();
    let current = s.prepare_zone_fill(view(&s).target, false).unwrap();
    assert!(!s.fail_zone_fill(old.source(), "late failure".into()));
    assert!(view(&s).filled_zones().is_none());
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    assert!(!s.fail_zone_fill(current.source(), "late duplicate".into()));
    assert!(view(&s).filled_zones().unwrap().is_ok());
}

/// A new schematic merge does not rewrite the board's accepted net history. If current zone
/// intent bridges those formerly distinct nets, the physical short report must see that fill.
#[test]
fn fill_bridges_are_visible_to_accepted_net_short_diagnostics() {
    let mut s = fixture(Connection::Solid, 0);
    let zone = view(&s).zones()[0].id;
    let geometry = BoardGeometry::new(view(&s));
    let objects = obstacles(view(&s), &geometry).unwrap();
    let net = view(&s).zones()[0].net;
    let a = objects.iter().find(|o| o.pad && o.net == Some(net)).unwrap();
    let b = objects.iter().find(|o| o.pad && o.net.is_some() && o.net != Some(net)).unwrap();
    let terminals = |object: &Obstacle| {
        let (footprint, pad) = object.object.name.split_once(':').unwrap();
        let pad = super::super::Pad::parse(footprint, pad).unwrap();
        let (component, pin) = view(&s).project().board.component_pin_for(pad).unwrap();
        json!({"symbol":view(&s).project().all_symbols().find(|symbol|symbol.component_id()==format!("component-{component}")).unwrap().id(),"terminal":pin})
    };
    let (from, to) = (terminals(a), terminals(b));
    let address = a.object.name.clone();
    s.apply(crate::CONNECT_PINS, &json!({"from":from,"to":to,"expected_revision":s.revision()})).unwrap();
    let target = view(&s).net_class_target_for_copper(&address).unwrap().unwrap();
    s.apply(super::super::zones::SET_ZONE, &json!({"zone":zone,"target":target,"expected_revision":s.revision()})).unwrap();
    let report = s.check_design(&json!({})).unwrap();
    assert!(report["counts"]["short"].as_u64().unwrap_or(0) > 0, "{report:#}");
    let mut job = s.design_check_job(&json!({})).unwrap();
    while !job.step(7) {}
    assert_eq!(json!(job.take_report().unwrap()), report);
}

/// Explicit removal persists across authored edits/checks while revoking previously captured
/// work; an explicit pour restores physical connections without changing saved intent/history.
#[test]
fn remove_pours_revokes_jobs_and_stays_unfilled_until_explicit_pour() {
    let mut s = fixture(Connection::Solid, 0);
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    let connected = view(&s).ratsnest().unwrap().lines.len();
    let bytes = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let target = view(&s).target;
    let mut old = s.prepare_zone_fill(target, true).unwrap();
    let response = super::worker::execute(&old.worker_request().unwrap());
    let check = s.design_check_job(&json!({})).unwrap();
    let route = s.copper_preparation(target).unwrap();
    let result = s.execute(UNFILL_ZONES, &json!({})).unwrap();
    assert_eq!(result["zone_fill"]["result"]["state"], "unfilled");
    assert!(!old.source().is_current(&s));
    assert!(!check.source().is_current(&s));
    assert!(!route.source().is_current(&s));
    assert!(old.accept_worker_response(&response).is_err());
    old.step(1);
    assert!(old.take_result().unwrap().is_none());
    assert!(view(&s).filled_zones().is_none());
    assert!(view(&s).ratsnest().unwrap().lines.len() > connected);
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    let mut check = s.design_check_job(&json!({})).unwrap();
    assert!(check.browser_zone_request().is_none());
    while !check.step(7) {}
    let report = check.take_report().unwrap();
    assert!(report.counts.get(&super::super::drc::Kind::ZoneFillUnavailable).copied().unwrap_or(0) > 0);
    assert!(!view(&s).zone_fill_enabled());
    let zone = view(&s).zones()[0].id;
    s.apply(super::super::zones::SET_ZONE, &json!({"zone":zone,"options":Options{priority:1,..Options::default()},"expected_revision":s.revision()}))
        .unwrap();
    assert!(s.prepare_zone_fill(target, false).is_err());
    assert!(s.execute(REFILL_ZONES, &json!({"board":Uuid::nil()})).is_err());
    assert!(!view(&s).zone_fill_enabled());
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert_eq!(s.execute(REFILL_ZONES, &json!({})).unwrap()["zone_fill"]["result"]["state"], "ready");
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), connected);
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// The explicit Pour copper command clips oversize zone intent to the board's authored edge
/// limit. A numeric boundary sample checks the 2 nm reserve beside the allowed inset.
#[test]
fn explicit_pour_respects_board_edge_clearance_after_removal() {
    let mut s = fixture(Connection::Solid, 0);
    let zone = view(&s).zones()[0].id;
    let points = [[-8_000_000, -8_000_000], [28_000_000, -8_000_000], [28_000_000, 38_000_000], [-8_000_000, 38_000_000]]
        .map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    s.apply(super::super::zones::SET_ZONE, &json!({"zone":zone,"points":points,"expected_revision":s.revision()})).unwrap();
    s.apply(crate::SET_RULES,&json!({"board":view(&s).target.board,"rules":super::super::rules::Values{edge_clearance_nm:600_000,..super::super::rules::Values::default()},"expected_revision":s.revision()})).unwrap();
    s.execute(UNFILL_ZONES, &json!({})).unwrap();
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    let filled = view(&s).filled_zones().unwrap().unwrap();
    let inset = board_region(view(&s)).unwrap().offset(-600_000).unwrap();
    assert!(!filled.zones[0].islands.is_empty());
    for island in &filled.zones[0].islands {
        assert!(clip(&island.region, &inset, Operation::Difference).unwrap().is_empty());
    }
    let material = &filled.zones[0].islands[0].region;
    assert_eq!(material.contains([-4_400_001, 0]).unwrap(), Some(false));
    assert_eq!(material.contains([-4_399_997, 0]).unwrap(), Some(true));
}

/// Current part-group records and adopted assembly profiles survive pouring/removal and the
/// same native-document worker transport without becoming another connectivity graph.
#[test]
fn current_project_metadata_survives_pour_remove_and_worker_transport() {
    let mut s = fixture(Connection::Solid, 0);
    s.apply(crate::part_groups::CREATE, &json!({"name":"Power section","expected_revision":s.revision()})).unwrap();
    let group = s.project().unwrap().part_groups().groups[0].id;
    let part = s.project().unwrap().symbols()[0].component_id();
    s.apply(crate::part_groups::SET_GROUPS, &json!({"part_instance":part,"groups":[group],"expected_revision":s.revision()})).unwrap();
    let profile = crate::fresh_id().unwrap();
    s.apply(
        crate::SET_ASSEMBLY_PROFILE,
        &json!({"profile":profile,"content":{"revision":1,"name":"Assembly","provenance":{"kind":"authored"}},"expected_revision":s.revision()}),
    )
    .unwrap();
    let mut adoption = json!({"board":view(&s).target.board,"profile":profile,"revision":1,"expected_revision":s.revision()});
    let preview = s.execute(crate::PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap();
    adoption["acceptance"] = preview["acceptance"].clone();
    s.apply(crate::ADOPT_ASSEMBLY_PROFILE, &adoption).unwrap();
    let native = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    s.execute(REFILL_ZONES, &json!({})).unwrap();
    s.execute(UNFILL_ZONES, &json!({})).unwrap();
    let mut job = s.prepare_zone_fill(view(&s).target, true).unwrap();
    let response = super::worker::execute(&job.worker_request().unwrap());
    assert!(job.accept_worker_response(&response).unwrap().is_some());
    assert_eq!(s.project().unwrap().part_groups().groups[0].id, group);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), native);
    assert_eq!(s.revision(), revision);
    let reopened = crate::persistence::decode_documents(&native).unwrap();
    assert_eq!(reopened.part_groups(), s.project().unwrap().part_groups());
    assert_eq!(reopened.assembly_profiles, s.project().unwrap().assembly_profiles);
    assert_eq!(reopened.board.assembly_profiles, s.project().unwrap().board.assembly_profiles);
    assert_eq!(crate::persistence::encode_documents(reopened).unwrap(), native);
}

/// Recovery observers record authored edits, while shared pour/remove commands only change
/// derived authority. Both command entry points leave the journal baseline and revision intact.
#[test]
fn derived_pour_commands_do_not_report_recoverable_edits() {
    #[derive(Debug, Default)]
    struct Log(std::sync::Mutex<Vec<String>>);
    impl crate::edits::EditObserver for Log {
        /// A snapshot notification is separate from the command log checked here.
        fn opened(&self, _project: Arc<crate::Project>) {}

        /// Retain command identities so an unexpected derived edit cannot hide behind a count.
        fn committed(&self, edit: crate::edits::CommittedEdit) {
            self.0.lock().unwrap().push(edit.command);
        }
    }
    let mut s = fixture(Connection::Solid, 0);
    let log = Arc::new(Log::default());
    s.observe_edits(Some(log.clone()));
    let native = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for command in [REFILL_ZONES, UNFILL_ZONES] {
        s.apply(command, &json!({})).unwrap();
        s.execute(command, &json!({})).unwrap();
    }
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), native);
    assert!(log.0.lock().unwrap().is_empty());
    s.apply("project.rename", &json!({"name":"Recovery still observes edits"})).unwrap();
    assert_eq!(*log.0.lock().unwrap(), ["project.rename"]);
}

/// A valid-source kernel refusal is an unavailable-zone finding, while malformed, foreign,
/// missing-diagnostics and invalid-geometry responses cannot resume a browser check.
#[test]
fn browser_fill_failure_continues_other_checks_but_invalid_responses_fail() {
    let mut s = fixture(Connection::Solid, 0);
    let ids: Vec<_> = view(&s).drawings().iter().map(|d| d.id).collect();
    for id in ids {
        s.apply(crate::DELETE_DRAWING, &json!({"drawing":id,"expected_revision":s.revision()})).unwrap();
    }
    let params = json!({});
    let mut job = s.design_check_job(&params).unwrap();
    let request = job.browser_zone_request().unwrap().unwrap();
    let response = super::worker::execute(&request);
    let wire: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(wire["result"]["Err"].as_str().unwrap().contains("closed Board Outline"));
    for invalid in ["not JSON".to_owned(), {
        let mut other = wire.clone();
        other["source"] = json!("foreign");
        other.to_string()
    }] {
        assert!(job.accept_browser_zones(&invalid).is_err());
    }
    job.accept_browser_zones(&response).unwrap();
    while !job.step(7) {}
    let report = job.take_report().unwrap();
    assert!(report.counts.get(&super::super::drc::Kind::ZoneFillUnavailable).copied().unwrap_or(0) > 0);
    assert!(report.counts.get(&super::super::drc::Kind::Unconnected).copied().unwrap_or(0) > 0);
    assert_eq!(json!(report), s.check_design(&params).unwrap());

    let s = fixture(Connection::Solid, 0);
    let mut job = s.design_check_job(&params).unwrap();
    let request = job.browser_zone_request().unwrap().unwrap();
    let response = super::worker::execute(&request);
    let wire: serde_json::Value = serde_json::from_str(&response).unwrap();
    let mut missing = wire.clone();
    missing["diagnostics"] = serde_json::Value::Null;
    assert!(job.accept_browser_zones(&missing.to_string()).unwrap_err().contains("omitted checker diagnostics"));
    let mut broken = wire.clone();
    broken["result"]["Ok"]["members"] = json!([[999999999]]);
    assert!(job.accept_browser_zones(&broken.to_string()).is_err());
    s.prepare_zone_fill(view(&s).target, true).unwrap();
    assert!(job.accept_browser_zones(&response).is_err());
}

/// Incremental phases use the material accepted with zone diagnostics even after the shared
/// slot is replaced. Hosts still reject jobs whose live authority has been revoked.
#[test]
fn incremental_checks_retain_the_fill_used_for_zone_diagnostics() {
    for browser in [false, true] {
        let s = fixture(Connection::Solid, 0);
        let params = json!({});
        let mut job = s.design_check_job(&params).unwrap();
        if browser {
            let request = job.browser_zone_request().unwrap().unwrap();
            job.accept_browser_zones(&super::worker::execute(&request)).unwrap();
        }
        while job.progress().stage != "Copper clearance" {
            assert!(!job.step(32));
        }
        let expected = s.check_design(&params).unwrap();
        s.prepare_zone_fill(view(&s).target, true).unwrap();
        assert!(view(&s).filled_zones().is_none());
        while !job.step(32) {}
        assert_eq!(json!(job.take_report().unwrap()), expected, "browser={browser}");
    }
}

/// A stricter diagnostic threshold checks retained foreign-zone material without regenerating it.
#[test]
fn stricter_diagnostics_detect_foreign_zone_island_clearance() {
    let mut s = fixture(Connection::Solid, 0);
    let (net, layer) = (view(&s).zones()[0].net, view(&s).zones()[0].layer);
    let geometry = BoardGeometry::new(view(&s));
    let other = obstacles(view(&s), &geometry).unwrap().into_iter().find(|o| o.pad && o.net.is_some() && o.net != Some(net)).unwrap();
    let target = view(&s).net_class_target_for_copper(&other.object.name).unwrap().unwrap();
    let points =
        [[8_000_000, 5_000_000], [16_000_000, 5_000_000], [16_000_000, 15_000_000], [8_000_000, 15_000_000]].map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    s.apply(ADD_ZONE,&json!({"target":target,"layer":layer,"points":points,"options":Options{priority:1,connection:Connection::Solid,..Options::default()},"expected_revision":s.revision()})).unwrap();
    let v = view(&s);
    let fill = v.fill_for_check().unwrap();
    let native = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let minimum = diagnostics(v, &BoardGeometry::new(v), v.effective_rules().values);
    assert!(!minimum.iter().any(|p| p.message.contains("another zone's clearance")));
    let stricter = diagnostics(v, &BoardGeometry::new(v), super::super::rules::Values { clearance_nm: 500_000, ..v.effective_rules().values });
    let pair = stricter
        .iter()
        .find(|p| p.message.contains("another zone's clearance"))
        .unwrap_or_else(|| panic!("foreign material gap must be checked: {stricter:?}"));
    assert!(matches!(pair.kind, ProblemKind::Clearance));
    assert!(v.zones().iter().any(|z| z.id == pair.zone));
    assert!(v.zones().iter().any(|z| pair.items.contains(&z.id.to_string())));
    assert!(Arc::ptr_eq(&fill, &v.filled_zones().unwrap().unwrap()));
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), native);
}

/// Main's synthetic via membership stays electrically and physically valid through pours,
/// native material-report format 25 and isolated fill transport.
#[test]
fn assigned_via_only_net_joins_its_zone_and_survives_report_worker_transport() {
    let mut s = fixture(Connection::Solid, 0);
    let zone = view(&s).zones()[0].clone();
    s.apply(super::super::zones::DELETE_ZONES, &json!({"zones":[zone.id],"expected_revision":s.revision()})).unwrap();
    s.apply(crate::net_labels::ADD, &json!({"name":"Via only fill","style":"global","x_nm":-90_000_000,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    let registry = s.project().unwrap().inspect_net_registry().unwrap();
    let target =
        json!({"kind":"derived","anchor":registry["nets"].as_array().unwrap().iter().find(|n|n["name"]=="Via only fill").unwrap()["anchor"]});
    s.apply(crate::ADD_VIA, &json!({"at":{"x_nm":12_000_000,"y_nm":20_000_000},"target":target,"expected_revision":s.revision()})).unwrap();
    s.apply(ADD_ZONE,&json!({"layer":zone.layer,"target":target,"points":zone.points,"options":Options{connection:Connection::Solid,..Options::default()},"expected_revision":s.revision()})).unwrap();
    let via = view(&s).vias().last().unwrap().clone();
    let mut setup = serde_json::to_value(view(&s).setup().unwrap()).unwrap();
    let mut material = serde_json::to_value(super::super::materials::Material::new("Worker reports".into()).unwrap()).unwrap();
    material["cti"] = json!([{"cti_v":175,"material_group":"IIIa","basis":"minimum","source":{"document":"Synthetic report"}}]);
    setup.as_object_mut().unwrap().entry("materials").or_insert_with(|| json!([])).as_array_mut().unwrap().push(material);
    s.apply(crate::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":s.revision()})).unwrap();
    let bytes = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(bytes.values().any(|b| String::from_utf8_lossy(b).contains(crate::persistence::CURRENT_BOARD_FORMAT)));
    let mut job = s.prepare_zone_fill(view(&s).target, true).unwrap();
    let request = job.worker_request().unwrap();
    let fill = job.accept_worker_response(&super::worker::execute(&request)).unwrap().unwrap();
    let v = view(&s);
    let geometry = BoardGeometry::new(v);
    assert_eq!(
        obstacles(v, &geometry).unwrap().iter().find(|o| o.object.name == via.id.to_string()).unwrap().net,
        Some(via.net_assignment.unwrap().net)
    );
    let index = geometry.lands.len() + v.tracks().len() + v.vias().iter().position(|v| v.id == via.id).unwrap();
    assert!(fill.zones.iter().any(|z| z.islands.iter().any(|i| i.contacts.contains(&index))));
    assert_eq!(v.copper_nets().unwrap().get(&via.id).map(String::as_str), Some("Via only fill"));
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
}

/// The shared route command replaces tracks atomically. Refill uses the resulting copper
/// and keeps an explicitly assigned plane via connected throughout the authored history.
#[test]
fn shove_through_a_pour_rebuilds_its_clearance_and_keeps_the_plane_connection() {
    use crate::board::route::{Mode, Outcome, Request};

    let mut s = fixture(Connection::Solid, 0);
    let v = view(&s);
    let target = v.target;
    let zone = v.zones()[0].clone();
    let at = |x, y| Point { x_nm: x, y_nm: y };
    let via_at = at(18_000_000, 20_000_000);
    let pad = v.ratsnest().unwrap().lines[0].from.clone();
    let pad_key = (pad.footprint, pad.pad.number.as_str().to_owned());
    let pad_address = format!("{}:{}", pad_key.0, pad_key.1);
    let via_target = v.net_class_target_for_copper(&pad_address).unwrap().unwrap();
    s.apply(crate::ADD_VIA, &json!({"at":via_at,"target":via_target,"expected_revision":s.revision()})).unwrap();
    s.apply(
        crate::ADD_TRACKS,
        &json!({"layer":zone.layer,"width_nm":500_000,"points":[at(0,20_000_000),at(10_000_000,20_000_000)],"expected_revision":s.revision()}),
    )
    .unwrap();
    let original_track = view(&s).tracks()[0].id;
    let via = view(&s).vias()[0].id;
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let check_fill = |s: &Session| {
        let v = view(s);
        let filled = v.fill_for_check().unwrap();
        assert!(filled.problems.is_empty(), "{:?}", filled.problems);
        assert_eq!(filled.copper.vias[&via], filled.copper.pads[&pad_key]);
        for object in obstacles(v, &BoardGeometry::new(v)).unwrap().iter().filter(|o| o.net != Some(zone.net)) {
            let keepout = region(&object.object.contour, v.effective_rules().values.clearance_nm).unwrap();
            for island in &filled.zones[0].islands {
                assert!(clip(&island.region, &keepout, Operation::Intersection).unwrap().is_empty(), "clearance to {}", object.object.name);
            }
        }
        filled
    };
    let old_fill = check_fill(&s);
    let request = Request {
        head: at(2_000_000, 22_000_000),
        target: at(8_000_000, 20_100_000),
        origin: None,
        layer: zone.layer,
        width_nm: 250_000,
        posture: Default::default(),
        flipped: false,
        previous: Vec::new(),
        mode: Mode::Shove,
        merge_same_net: false,
        fixed: Vec::new(),
    };
    let q = s.clearance_query_with_rules(target, &json!({})).unwrap();
    let plan = s.plan_route(&q, &request, Mode::Shove.default_probe_budget(), &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.replacements.iter().any(|replacement| replacement.track == original_track), "{plan:?}");
    assert!(Arc::ptr_eq(&old_fill, &view(&s).filled_zones().unwrap().unwrap()));
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let revision = s.revision();
    s.apply(
        crate::ADD_TRACKS,
        &json!({"layer":zone.layer,"width_nm":request.width_nm,"points":plan.points,"replacements":plan.replacements,"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(s.revision(), revision + 1);
    assert!(!q.authority().is_current(&s));
    assert!(view(&s).filled_zones().is_none());
    let after = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let new_fill = check_fill(&s);
    assert!(!Arc::ptr_eq(&old_fill, &new_fill));
    s.apply("edit.undo", &serde_json::Value::Null).unwrap();
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    assert!(view(&s).filled_zones().is_none());
    check_fill(&s);
    s.apply("edit.redo", &serde_json::Value::Null).unwrap();
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), after);
    assert!(view(&s).filled_zones().is_none());
    check_fill(&s);
}

#[test]
fn explicit_pour_changes_revoke_route_and_edit_sources_at_the_same_revision() {
    let s = fixture(Connection::Solid, 0);
    let target = view(&s).target;
    let query = s.clearance_query_with_rules(target, &json!({})).unwrap();
    let planning = s.route_planning(target).unwrap();
    let editing = s.copper_preparation(target).unwrap();
    let revision = s.revision();
    s.unfill_zones(&json!({})).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(!query.authority().is_current(&s));
    assert!(!planning.source().is_current(&s));
    assert!(!editing.source().is_current(&s));

    let query = s.clearance_query_with_rules(target, &json!({})).unwrap();
    let job = s.prepare_zone_refill(&json!({})).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(!query.authority().is_current(&s));
    assert!(job.source().is_current(&s));
}

/// Local thermal clipping keeps a spoke only if it can reach the original body. A foreign
/// keepout that blocks one direction and a neighboring pad's gap exercise both subtractions.
#[test]
fn local_thermal_calculation_matches_the_whole_plane_boolean() {
    let rect = |x0, y0, x1, y1| Regions::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]).unwrap();
    let stock = rect(0, 0, 10_000_000, 10_000_000);
    let foreign = vec![rect(5_800_000, 3_000_000, 6_000_000, 7_000_000)];
    let gaps = vec![rect(4_000_000, 4_000_000, 5_900_000, 6_000_000), rect(2_000_000, 4_000_000, 3_200_000, 6_000_000)];
    let spokes = [
        rect(3_500_000, 4_900_000, 5_000_000, 5_100_000),
        rect(4_900_000, 4_900_000, 5_700_000, 5_100_000),
        rect(4_900_000, 4_900_000, 5_100_000, 6_300_000),
        rect(2_500_000, 4_900_000, 4_500_000, 5_100_000),
    ];
    let base = clip(&stock, &Regions::union_all(&foreign).unwrap(), Operation::Difference).unwrap();
    let body = clip(&base, &Regions::union_all(&gaps).unwrap(), Operation::Difference).unwrap();
    let accepted: Vec<_> = spokes
        .iter()
        .map(|s| clip(s, &base, Operation::Intersection).unwrap())
        .filter(|s| !clip(s, &body, Operation::Intersection).unwrap().is_empty())
        .collect();
    let expected = clip(&body, &Regions::union_all(&accepted).unwrap(), Operation::Union).unwrap();
    let mut keepouts = cuts::Cuts::new(foreign);
    let remaining = cuts::thermal_gaps(Some(&stock), &keepouts, gaps, spokes.into_iter().map(Ok), None).unwrap();
    keepouts.shapes.extend(remaining);
    let found = Cuts::new(keepouts.shapes).subtract(stock, None).unwrap();
    for (a, b) in [(&expected, &found), (&found, &expected)] {
        assert!(clip(a, b, Operation::Difference).unwrap().is_empty());
    }
}

/// A track reaching through a zone edge must leave the interior hole batch intact.
#[test]
fn boundary_cut_preserves_the_separate_interior_holes() {
    let rect = |x0, y0, x1, y1| Regions::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]).unwrap();
    let stock = rect(0, 0, 25_000_000, 8_000_000);
    let centers: Vec<_> = (1..=20).map(|i| [i * 1_000_000, 2_000_000]).collect();
    let mut holes: Vec<_> = centers.iter().map(|center| Regions::circle(*center, 500_000).unwrap()).collect();
    holes.push(rect(22_000_000, -1_000_000, 24_000_000, 3_000_000));
    let expected = clip(&stock, &Regions::union_all(&holes).unwrap(), Operation::Difference).unwrap();
    let found = Cuts::new(holes).subtract(stock, None).unwrap();
    assert!((found.area().unwrap().value_nm2 - expected.area().unwrap().value_nm2).abs() < 100.);
    assert_eq!(found.contains([23_000_000, 2_000_000]).unwrap(), Some(false));
    assert_eq!(found.contains([23_000_000, 4_000_000]).unwrap(), Some(true));
    for center in centers {
        assert_eq!(found.contains(center).unwrap(), Some(false));
    }
}

/// Two disjoint cutters can have overlapping envelopes after their local union.
/// Their exact subtraction and an outline-crossing cut must leave the independent
/// holes on the large-face path, including when the remaining plane has two islands.
#[test]
fn complex_clearance_does_not_send_all_the_planes_holes_through_the_boolean_budget() {
    let stock = Regions::polygon(&[[0, 0], [20_000_000, 0], [20_000_000, 11_000_000], [0, 11_000_000]]).unwrap();
    let pad = Regions::rounded_core(&[[0, 0], [150_000, 0], [150_000, 150_000], [0, 150_000]], 25_000.).unwrap();
    let mut cuts: Vec<_> = (0..1050).map(|i| pad.place_profile([500_000 + (i % 50) * 300_000, 500_000 + (i / 50) * 300_000]).unwrap()).collect();
    cuts.push(
        Regions::polygon(&[
            [16_000_000, 7_000_000],
            [18_000_000, 7_000_000],
            [18_000_000, 7_300_000],
            [16_300_000, 7_300_000],
            [16_300_000, 9_000_000],
            [16_000_000, 9_000_000],
        ])
        .unwrap(),
    );
    cuts.push(Regions::polygon(&[[16_600_000, 7_600_000], [17_000_000, 7_600_000], [17_000_000, 8_000_000], [16_600_000, 8_000_000]]).unwrap());
    cuts.push(Regions::polygon(&[[15_500_000, -1_000_000], [15_650_000, -1_000_000], [15_650_000, 12_000_000], [15_500_000, 12_000_000]]).unwrap());

    let material = Cuts::new(cuts).subtract(stock, None).unwrap();
    assert_eq!(material.region_count(), 2);
    for point in [[550_000, 550_000], [16_150_000, 8_000_000], [16_800_000, 7_800_000], [15_600_000, 5_000_000]] {
        assert_eq!(material.contains(point).unwrap(), Some(false));
    }
    assert_eq!(material.contains([17_100_000, 8_000_000]).unwrap(), Some(true));
    for island in material.islands() {
        assert!(!island.display_mesh(5_000).unwrap().triangles.is_empty());
    }
}
