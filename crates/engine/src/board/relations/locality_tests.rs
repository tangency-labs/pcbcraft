//! Stable native node identities and final-island locality through real commands.
use super::{
    commands::PREVIEW,
    islands::Key,
    probes::{Op, trace},
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
            name_terminal(&mut s, 0, 1);
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
