//! Stable native node identities and final-island locality through real commands.
use super::{
    commands::PREVIEW,
    islands::Key,
    probes::{Op, trace, trace_final_pads},
    tests::{Observe, add, bytes, origin, pick, run, unchanged},
};
use crate::{
    Session,
    board::{
        drc::short_tests::{Bridge, bridged_split, bridged_split_with, shorts},
        track::tests::{copper_layers, route, routed_fixture, view},
        update as eco,
    },
    schematic::{Point, WireEndpoint},
};
use serde_json::{Value, json};
use std::sync::Arc;

fn fp(s: &Session, symbol: usize) -> usize {
    let p = s.project().unwrap();
    p.board.footprints().iter().position(|f| f.component == p.symbols[symbol].component).unwrap()
}

fn pad(s: &Session, i: usize, number: &str) -> (String, Point) {
    let f = view(s).footprints().nth(i).unwrap();
    (f.id, f.pads.iter().find(|p| p.number == number).unwrap().center)
}

fn name_terminal(s: &mut Session, symbol: usize, terminal: usize) {
    let p = s.project().unwrap();
    let at = p.terminal_position(p.symbols[symbol].terminals()[terminal]).unwrap();
    run(s, crate::net_labels::ADD, json!({"name":"SEPARATE","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm})).unwrap();
}

/// Name a terminal through a short stub so the label stays off any wire passing over the
/// terminal: a label on a wire path joins that wire.
fn name_terminal_off_wire(s: &mut Session, symbol: usize, terminal: usize) {
    let p = s.project().unwrap();
    let term = p.symbols[symbol].terminals()[terminal];
    let at = p.terminal_position(term).unwrap();
    let end = Point { x_nm: at.x_nm, y_nm: at.y_nm - 4 * crate::schematic::GRID_NM };
    run(s, crate::DRAW_WIRE, json!({"from":term.inspect(),"to":end,"bends":[]})).unwrap();
    run(s, crate::net_labels::ADD, json!({"name":"SEPARATE","style":"global","x_nm":end.x_nm,"y_nm":end.y_nm})).unwrap();
}

fn remote_split(s: &mut Session) {
    let p = s.project().unwrap();
    let term = p.symbols[2].terminals()[0];
    let wire = p.wires.iter().find(|w| w.endpoints().contains(&WireEndpoint::Terminal(term))).unwrap().id();
    run(s, crate::DELETE_WIRE, json!({"wire":wire})).unwrap();
    name_terminal(s, 2, 0);
}

fn prepare(s: &mut Session) -> Value {
    run(s, eco::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone()
}

fn apply(s: &mut Session, report: &Value) {
    let candidate = s.checked_eco_candidate().unwrap().clone();
    let encoded = crate::persistence::encode_documents(candidate.clone()).unwrap();
    run(s, eco::APPLY_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
    assert!(Arc::ptr_eq(&candidate, &s.snapshot().unwrap()));
    assert_eq!(bytes(s), encoded);
}

#[test]
fn a_net_locality_remote_peer_and_ordinal_reindex_leave_existing_short_unmarked() {
    for reindex in [false, true] {
        let mut s = bridged_split(Bridge::Track);
        let a = fp(&s, 1);
        let old_short = shorts(&s, 0);
        assert_eq!(old_short.len(), 1);
        let track = view(&s).tracks()[0].id;
        let (aid, _) = pad(&s, a, "2");
        let (cid, _) = pad(&s, fp(&s, 2), "2");
        let copper = view(&s).copper().unwrap();
        assert_eq!(copper.tracks[&track], copper.pads[&(aid.clone(), "2".into())]);
        assert_ne!(copper.tracks[&track], copper.pads[&(cid, "2".into())]);
        let reference = pick(&mut s, a, "pad.center", Some("2"));
        add(&mut s, "horizontal", origin(), reference);
        let component = s.project().unwrap().symbols[1].component;
        let remote = s.project().unwrap().symbols[2].component;
        let n = |s: &Session, c, t: &str| s.project().unwrap().board.baseline.as_ref().unwrap().terminal_net_index()[&(c, t)];
        let old_other = n(&s, component, "2");
        assert_eq!(n(&s, component, "1"), n(&s, remote, "1"));
        assert_ne!(n(&s, component, "1"), old_other);
        if reindex {
            name_terminal_off_wire(&mut s, 0, 1);
        } else {
            remote_split(&mut s);
        }
        let before = s.snapshot().unwrap();
        let (report, ops) = trace(|| prepare(&mut s));
        assert!(!ops.iter().any(|op| matches!(op, Op::Marked { .. })), "{ops:?}");
        apply(&mut s, &report);
        if reindex {
            assert_ne!(n(&s, component, "2"), old_other);
            assert_eq!(n(&s, component, "1"), n(&s, remote, "1"));
        } else {
            assert_ne!(n(&s, component, "1"), n(&s, remote, "1"));
        }
        let final_short = shorts(&s, 0);
        assert_eq!(final_short.len(), 1);
        assert_eq!(final_short[0].items, old_short[0].items);
        assert_eq!(s.project().unwrap().board.tracks, before.board.tracks);
        assert_eq!(s.project().unwrap().board.vias, before.board.vias);
        assert_eq!(s.project().unwrap().board.footprints(), before.board.footprints());
    }
}

#[test]
fn a_net_locality_vanished_contact_not_remote_same_net() {
    for touches in [false, true] {
        let mut s = bridged_split_with(Bridge::Track, |s, at| {
            let at = if touches { at } else { pad(s, fp(s, 2), "2").1 };
            let layer = copper_layers(s)[0];
            route(s, layer, &[at, Point { x_nm: at.x_nm, y_nm: at.y_nm + 2_000_000 }]).unwrap();
        });
        let a = fp(&s, 1);
        let reference = pick(&mut s, a, "pad.center", Some("2"));
        add(&mut s, "horizontal", origin(), reference);
        let (aid, _) = pad(&s, a, "2");
        let branch = view(&s).tracks().iter().find(|t| t.start.x_nm == t.end.x_nm).unwrap().id;
        let bridge = view(&s).tracks().iter().find(|t| t.start.x_nm != t.end.x_nm).unwrap().id;
        let copper = view(&s).copper().unwrap();
        assert_eq!(copper.tracks[&branch] == copper.tracks[&bridge], touches);
        let observer: Arc<Observe> = Arc::default();
        s.observe_edits(Some(observer.clone()));
        let command = |s: &mut Session| run(s, PREVIEW, json!({"intent":{"command":"pcb.delete_items","params":{"items":{"tracks":[branch]}}}}));
        if touches {
            let (error, ops) = trace(|| unchanged(&mut s, command));
            assert_eq!(error.details()["reason"], "affected_short");
            assert!(ops.iter().any(|op| matches!(op, Op::Marked {nodes,physical:true,electrical:false} if nodes.contains(&Key::Track(bridge)) && nodes.contains(&Key::Land(aid.clone(), "2".into())) && !nodes.contains(&Key::Track(branch)))));
            assert!(ops.iter().any(|op| matches!(op, Op::VanishedCandidates { removed: 1, candidates: 1, .. })));
            assert!(observer.0.lock().unwrap().is_empty());
        } else {
            let (report, ops) = trace(|| command(&mut s).unwrap());
            assert!(!ops.iter().any(|op| matches!(op, Op::Marked { .. })));
            let review = &report["relation_preview"];
            run(&mut s, super::commands::APPLY, super::tests::decision(review)).unwrap();
            assert_eq!(observer.0.lock().unwrap().len(), 1);
            assert_eq!(shorts(&s, 0).len(), 1);
        }
    }
}

#[test]
fn a_net_locality_explicit_via_alias_changes_without_authored_copper_edit() {
    let mut s = routed_fixture();
    let a = fp(&s, 0);
    let (aid, at) = pad(&s, a, "2");
    let target = view(&s).net_class_target_for_copper(&format!("{aid}:2")).unwrap().unwrap();
    run(&mut s, crate::ADD_VIA, json!({"at":at,"target":target})).unwrap();
    let via = view(&s).vias()[0].clone();
    let address = via.net_assignment.unwrap();
    let component = s.project().unwrap().symbols[0].component;
    let is_aliased = |s: &Session| {
        let p = s.project().unwrap();
        assert_eq!(p.via_net_state(address), "active");
        let accepted = p.board.baseline.as_ref().unwrap().terminal_net_index()[&(component, "1")];
        p.board_intent_memberships().ids[&address.net] == accepted
    };
    assert!(is_aliased(&s));
    remote_split(&mut s);
    assert!(!is_aliased(&s));
    let reference = pick(&mut s, a, "pad.center", Some("2"));
    add(&mut s, "horizontal", origin(), reference);
    let before = s.snapshot().unwrap();
    let (report, ops) = trace(|| prepare(&mut s));
    assert!(ops.iter().any(|op| matches!(op, Op::Marked {nodes,physical:false,electrical:true} if nodes.contains(&Key::Via(via.id)) && nodes.contains(&Key::Land(aid.clone(), "2".into())))), "{ops:?}");
    apply(&mut s, &report);
    assert!(is_aliased(&s));
    assert_eq!(view(&s).vias(), &[via]);
    assert_eq!(s.project().unwrap().board.footprints(), before.board.footprints());
    assert_eq!(s.project().unwrap().board.tracks, before.board.tracks);
    assert!(shorts(&s, 0).is_empty());
    let saved = bytes(&s);
    s.open_snapshot(crate::persistence::decode_documents(&saved).unwrap());
    assert_eq!(bytes(&s), saved);
    assert!(is_aliased(&s));
    assert_eq!(run(&mut s, super::commands::LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
}

#[test]
fn a_net_locality_layer_reorder_is_not_membership_change() {
    use crate::board::{definition::CopperBinding, setup::BoardSetup};
    let mut s = bridged_split(Bridge::Track);
    let a = fp(&s, 1);
    let mut p = s.project().unwrap().clone();
    let f = &p.board.footprints()[a];
    let component = format!("component-{}", f.component);
    let mut d = p.board.definition_for(f).unwrap().clone();
    d.revision += 1;
    let land = d.pads.iter_mut().find(|pad| pad.number == "2").unwrap();
    land.copper = CopperBinding::AllCopper;
    land.drill_nm = Some(100_000);
    let (library, name, revision) = d.key();
    Arc::make_mut(&mut p.board).definitions.insert(d.key(), Arc::new(d));
    s.project = Some(Arc::new(p));
    run(
        &mut s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":component,"library":library,"definition":name,"definition_revision":revision,"mapping":{"1":"2","2":"1"}}),
    )
    .unwrap();
    let legacy = prepare(&mut s);
    run(&mut s, eco::APPLY_UPDATE, json!({"plan_token":legacy["plan_token"]})).unwrap();
    let two = serde_json::to_value(view(&s).setup().unwrap()).unwrap();
    let four = serde_json::to_value(BoardSetup::preset(4).unwrap()).unwrap();
    let mut setup = two.clone();
    setup["layers"]
        .as_array_mut()
        .unwrap()
        .extend(four["layers"].as_array().unwrap().iter().filter(|l| l["purpose"] == "copper" && l.get("side").is_none()).cloned());
    setup["stackup"]["strata"].as_array_mut().unwrap().splice(2..2, four["stackup"]["strata"].as_array().unwrap()[2..6].iter().cloned());
    run(&mut s, crate::SET_BOARD_SETUP, json!({"setup":setup,"resolve_existing_footprints":true})).unwrap();
    let reference = pick(&mut s, a, "pad.center", Some("2"));
    add(&mut s, "horizontal", origin(), reference);
    let before = s.snapshot().unwrap();
    let short = shorts(&s, 0);
    assert_eq!(short.len(), 1);
    let old_order = setup["stackup"]["strata"].clone();
    setup["stackup"]["strata"].as_array_mut().unwrap().swap(2, 4);
    assert_ne!(setup["stackup"]["strata"], old_order);
    let (r, ops) = trace(|| run(&mut s, crate::SET_BOARD_SETUP, json!({"setup":setup,"resolve_existing_footprints":true})));
    r.unwrap();
    assert!(!ops.iter().any(|op| matches!(op, Op::Marked { .. })));
    assert_eq!(shorts(&s, 0)[0].items, short[0].items);
    assert_eq!(s.project().unwrap().board.tracks, before.board.tracks);
    assert_eq!(s.project().unwrap().board.footprints(), before.board.footprints());
    assert_eq!(serde_json::to_value(view(&s).setup().unwrap().stackup()).unwrap()["strata"], setup["stackup"]["strata"]);
    let (e, ops) = trace(|| unchanged(&mut s, |s| run(s, crate::SET_BOARD_SETUP, json!({"setup":two,"resolve_existing_footprints":true}))));
    assert_eq!(e.details()["reason"], "affected_short");
    assert!(ops.iter().any(|op| matches!(op, Op::Marked { physical: true, electrical: false, .. })));
}

/// A long single island with only adjacent chain contacts, opposed by a disjoint grid
/// of original copper removed through the production Delete Tracks command.
#[test]
fn a_net_locality_bulk_remote_tracks_use_vanished_broad_phase() {
    use crate::board::{geometry::Contour, track::Track};
    use uuid::Uuid;
    let mut s = bridged_split(Bridge::Track);
    let a = fp(&s, 1);
    let (aid, at) = pad(&s, a, "2");
    let reference = pick(&mut s, a, "pad.center", Some("2"));
    add(&mut s, "horizontal", origin(), reference);
    let layer = copper_layers(&s)[0];
    let mut p = s.project().unwrap().clone();
    let tracks = &mut Arc::make_mut(&mut p.board).tracks;
    let mut removed = Vec::new();
    let mut end = at;
    let mut chain = Vec::new();
    for i in 0..10_000 {
        let start = end;
        end.y_nm += 50_000; // Pitch exceeds the 10,000 nm copper diameter: skip-one pairs cannot touch.
        let id = Uuid::from_u128(1 + i as u128);
        chain.push(id);
        tracks.push(Track { id, layer, locked: false, width_nm: 10_000, start, end });
        let id = Uuid::from_u128(10_001 + i as u128);
        let x_nm = -300_000_000 + (i % 100) * 50_000;
        let y_nm = -300_000_000 + (i / 100) * 50_000;
        removed.push(id);
        tracks.push(Track { id, layer, locked: false, width_nm: 10_000, start: Point { x_nm, y_nm }, end: Point { x_nm: x_nm + 20_000, y_nm } });
    }
    tracks.sort_by_key(|t| t.id);
    let saved = crate::persistence::encode_documents(Arc::new(p)).unwrap();
    s.open_snapshot(crate::persistence::decode_documents(&saved).unwrap());
    assert_eq!(bytes(&s), saved);
    let source = view(&s);
    let indexed = |id| &source.tracks()[source.tracks().binary_search_by_key(&id, |track| track.id).unwrap()];
    let contour = |track: &Track| Contour::capsule(track.start, track.end, track.width_nm as f64 / 2.);
    // Native contour oracles on the round-tripped records. Monotone equal pitch makes
    // skip-one the nearest nonadjacent pair; all farther apart have a larger gap.
    for ids in chain.windows(3) {
        let [a, b, c] = [contour(indexed(ids[0])), contour(indexed(ids[1])), contour(indexed(ids[2]))];
        assert!(a.touches(&b) && b.touches(&c) && !a.touches(&c));
    }
    let old_short = shorts(&s, 0);
    assert_eq!(old_short.len(), 1);
    let copper = source.copper().unwrap();
    let island = copper.pads[&(aid, "2".into())];
    assert!(chain.iter().all(|id| copper.tracks[id] == island));
    assert!(removed.iter().all(|id| copper.tracks[id] != island));
    let before = source.tracks().len();
    let (r, ops) = trace(|| run(&mut s, crate::DELETE_TRACKS, json!({"tracks":removed})));
    r.unwrap();
    assert_eq!(view(&s).tracks().len(), before - 10_000);
    assert!(!ops.iter().any(|op| matches!(op, Op::Marked { .. })));
    assert!(ops.iter().any(|op| matches!(op, Op::VanishedCandidates{removed:10_000, queries, candidates:0} if *queries >= 10_000)), "{ops:?}");
    let final_short = shorts(&s, 0);
    assert_eq!(final_short.len(), 1);
    assert_eq!(final_short[0].items, old_short[0].items);
}

/// Add native placements of one pinned synthetic 128-land definition. Only install the
/// immutable fixture asset privately; assignment, optional ECO and placement are public.
fn add_pad_columns(s: &mut Session, count: i64) {
    use crate::board::definition::{CopperBinding, PadShape, RectPad, basic_components};
    let mut d = basic_components("resistor-0402").unwrap().as_ref().clone();
    d.id = "relation-work-fixture".into();
    d.pads = (1..=128)
        .map(|n| RectPad {
            number: n.to_string(),
            center: Point { x_nm: 0, y_nm: (n - 1) * 5_000 },
            width_nm: 2_000,
            height_nm: 2_000,
            rotation: 0,
            shape: PadShape::Circle,
            copper: CopperBinding::ComponentSide,
            corner_radius_nm: 0,
            drill_nm: None,
        })
        .collect();
    d.validate().unwrap();
    let key = d.key();
    let d = Arc::new(d);
    let unconnected: Vec<_> = (3..=128).map(|n| n.to_string()).collect();
    for i in 0..count {
        run(s, crate::PLACE_RESISTOR, json!({"x_nm":50_000_000 + i * 10_000_000,"y_nm":100_000_000})).unwrap();
        let mut p = s.project().unwrap().clone();
        let component = format!("component-{}", p.components.keys().next_back().unwrap());
        Arc::make_mut(&mut p.board).definitions.insert(key.clone(), d.clone());
        s.project = Some(Arc::new(p));
        run(
            s,
            crate::ASSIGN_FOOTPRINT,
            json!({"component":component,"library":key.0,"definition":key.1,"definition_revision":key.2,
            "mapping":{"1":"1","2":"2"},"unconnected_pads":unconnected}),
        )
        .unwrap();
        if s.project().unwrap().board.baseline.is_some() {
            let report = prepare(s);
            run(s, eco::APPLY_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
        }
        run(s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":50_000_000,"y_nm":100_000_000 + i * 1_000_000})).unwrap();
    }
}

/// A direct public removal has no added-copper job and must not pay the old all-X sweep
/// during final preparation. Bound the actual walk, not just surviving pair emissions.
#[test]
fn a_net_locality_remote_delete_bounds_final_pad_walk() {
    let mut s = Session::default();
    run(&mut s, "project.new", json!({"create_board":true})).unwrap();
    run(&mut s, crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true})).unwrap();
    add_pad_columns(&mut s, 32);
    let layer = copper_layers(&s)[0];
    route(&mut s, layer, &[Point { x_nm: -50_000_000, y_nm: -50_000_000 }, Point { x_nm: -49_000_000, y_nm: -50_000_000 }]).unwrap();
    let remote = view(&s).tracks()[0].id;
    let reference = pick(&mut s, 0, "pad.center", Some("1"));
    add(&mut s, "horizontal", origin(), reference);
    let saved = bytes(&s);
    s.open_snapshot(crate::persistence::decode_documents(&saved).unwrap());
    assert_eq!(bytes(&s), saved);
    let before = s.snapshot().unwrap();
    let lands = crate::board::track::lands(view(&s));
    let n = lands.len();
    assert_eq!(n, 32 * 128);
    let mut bounds: Vec<_> = lands.iter().map(|land| land.contour().bounds()).collect();
    bounds.sort_by(|a, b| a[1].total_cmp(&b[1]));
    assert!(bounds.iter().all(|b| b[0] == bounds[0][0] && b[2] == bounds[0][2]));
    assert!(bounds.windows(2).all(|b| b[0][3] < b[1][1]));
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let revision = s.revision();
    let (result, ops) = trace_final_pads(|| run(&mut s, crate::DELETE_TRACKS, json!({"tracks":[remote]})));
    result.unwrap();
    assert!(
        ops.iter().any(|op| matches!(op, Op::FinalPadWork { pads, units, candidates: 0, contacts: 0 }
        if *pads == n && (2*n..=32*n).contains(units))),
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| matches!(op, Op::VanishedCandidates { removed: 1, queries: 128, candidates: 0 })), "{ops:?}");
    assert!(!ops.iter().any(|op| matches!(op, Op::Marked { .. })));
    assert_eq!(ops.iter().filter(|op| matches!(op, Op::Publish(super::RelationStage::Outer))).count(), 1);
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    assert!(view(&s).tracks().is_empty());
    assert_eq!(s.project().unwrap().board.footprints(), before.board.footprints());
    assert_eq!(s.project().unwrap().board.relations, before.board.relations);
}

/// The same indexed final pass must retain true direct pad contact even after the last
/// track disappears. A short on that physically marked two-land island is not approvable.
#[test]
fn a_net_locality_final_pad_walk_keeps_native_direct_contact() {
    let mut s = bridged_split_with(Bridge::Pads, |s, at| {
        let layer = copper_layers(s)[0];
        route(s, layer, &[at, Point { x_nm: at.x_nm, y_nm: at.y_nm + 2_000_000 }]).unwrap();
    });
    add_pad_columns(&mut s, 1);
    let branch = view(&s).tracks()[0].id;
    let violations = shorts(&s, 0);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].items.len(), 3);
    let named: Vec<_> = violations[0].items.iter().filter_map(|item| item.split_once(':')).collect();
    assert_eq!(named.len(), 2);
    let index = view(&s).footprints().position(|f| f.id == named[0].0).unwrap();
    let reference = pick(&mut s, index, "fp.origin", None);
    add(&mut s, "aligned", origin(), reference);
    let saved = bytes(&s);
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let (error, ops) = trace_final_pads(|| unchanged(&mut s, |s| run(s, crate::DELETE_TRACKS, json!({"tracks":[branch]}))));
    assert_eq!(error.details()["reason"], "affected_short");
    assert!(
        ops.iter().any(|op| matches!(op, Op::FinalPadWork { pads: 134, units, candidates, contacts }
        if *units > *candidates && *candidates >= *contacts && *contacts >= 1)),
        "{ops:?}"
    );
    assert!(
        ops.iter().any(|op| matches!(op, Op::Marked { nodes, physical:true, electrical:false }
        if nodes.len() == 2 && named.iter().all(|&(fp, pad)| nodes.contains(&Key::Land(fp.into(),pad.into()))))),
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| matches!(op, Op::VanishedCandidates { removed: 1, candidates: 1, .. })));
    assert!(observer.0.lock().unwrap().is_empty());
    assert_eq!(bytes(&s), saved);
}

#[test]
fn a_net_locality_native_eco_changed_land_short_and_clean_control() {
    for touches in [false, true] {
        let mut s = routed_fixture();
        let i = fp(&s, 1);
        let p = s.project().unwrap();
        let f = &p.board.footprints()[i];
        let id = f.id;
        let component = f.component;
        let (other, _) = pad(&s, i, "1");
        let mut def = p.board.definitions[&f.key().unwrap()].as_ref().clone();
        let sibling = def.pads.iter().find(|p| p.number == "1").unwrap().center;
        let moved = def.pads.iter_mut().find(|p| p.number == "2").unwrap();
        moved.center = Point { x_nm: sibling.x_nm, y_nm: sibling.y_nm + if touches { 0 } else { 3_000_000 } };
        def.revision += 1;
        let (library, name, revision) = def.key();
        let reference = pick(&mut s, i, "fp.origin", None);
        add(&mut s, "horizontal", origin(), reference);
        // Install only the fixture-owned definition; mapping and desired intent still pass through native assignment.
        // Include untouched native copper, not a geometry-only board with an empty track table.
        let front = copper_layers(&s)[0];
        route(&mut s, front, &[Point { x_nm: 30_000_000, y_nm: 30_000_000 }, Point { x_nm: 31_000_000, y_nm: 30_000_000 }]).unwrap();
        let before = s.snapshot().unwrap();
        let mut p = before.as_ref().clone();
        Arc::make_mut(&mut p.board).definitions.insert(def.key(), Arc::new(def));
        s.project = Some(Arc::new(p));
        run(&mut s, crate::ASSIGN_FOOTPRINT, json!({"component":format!("component-{component}"),"library":library,"definition":name,"definition_revision":revision,"mapping":{"1":"2","2":"1"}})).unwrap();
        let action = |s: &mut Session| run(s, eco::PREVIEW_UPDATE, json!({}));
        if touches {
            let (err, ops) = trace(|| unchanged(&mut s, action));
            assert_eq!(err.details()["reason"], "affected_short");
            assert!(ops.iter().any(|op| matches!(op, Op::Marked {nodes,physical:true,electrical:false} if nodes.contains(&Key::Land(other.clone(), "1".into())) && nodes.contains(&Key::Land(other.clone(), "2".into())))));
        } else {
            let (r, ops) = trace(|| action(&mut s).unwrap());
            assert!(ops.iter().any(|op| matches!(op, Op::Marked { physical: true, .. })));
            let r = &r["board_update_preview"];
            let remap = &r["relation_detail"]["remaps"][0]["id"];
            run(&mut s, eco::APPLY_UPDATE, json!({"plan_token":r["plan_token"],"accept_loss":[],"confirm_remaps":[remap]})).unwrap();
            assert_eq!(s.project().unwrap().board.footprints()[i].id, id);
            assert!(shorts(&s, 0).is_empty());
        }
        assert_eq!(s.project().unwrap().board.tracks, before.board.tracks);
        assert_eq!(s.project().unwrap().board.vias, before.board.vias);
    }
}
