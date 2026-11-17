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
    let response = super::worker::execute(&request);
    let filled = job.accept_worker_response(&response).unwrap().unwrap();
    assert!(view(&s).ratsnest_with_copper(&filled.copper).unwrap().lines.len() < 3);
    let mut wrong: serde_json::Value = serde_json::from_str(&response).unwrap();
    wrong["source"] = json!("another immutable source");
    let mut job = s.prepare_zone_fill(view(&s).target, true).unwrap();
    let _ = job.worker_request().unwrap();
    assert!(job.accept_worker_response(&wrong.to_string()).is_err());
    assert!(view(&s).filled_zones().is_none());
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
