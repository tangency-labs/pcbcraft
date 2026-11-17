use super::*;
use crate::{
    Session,
    board::{
        clipboard::{BoardFragment, BoardPasteOptions, BoardSelection},
        drc::{Kind, Rules},
        track::tests::{copper_layers, routed_fixture, view},
    },
    persistence::{decode_documents, encode_documents},
    schematic::Point,
};
use std::sync::Arc;

/// Production commands keep fixture histories and targets identical to ordinary clients.
fn run(s: &mut Session, command: &str, mut params: Value) -> Result<u64, CommandError> {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params)
}
/// Pick two distinct meaningful nets from the shared identity inspection, never display text.
fn targets(s: &Session) -> Vec<Value> {
    s.project().unwrap().inspect_net_registry().unwrap()["nets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| json!({"kind":"derived","anchor":row["anchor"]}))
        .collect()
}
/// Place a via well outside the fixture's pad envelope, returning its fresh identity.
fn via(s: &mut Session, target: Option<&Value>, x: i64) -> Uuid {
    let before: BTreeSet<_> = view(s).vias().iter().map(|v| v.id).collect();
    let mut params = json!({"at":{"x_nm":x,"y_nm":-30_000_000}});
    if let Some(target) = target {
        params["target"] = target.clone();
    }
    run(s, crate::ADD_VIA, params).unwrap();
    view(s).vias().iter().find(|v| !before.contains(&v.id)).unwrap().id
}

/// Authored identity survives disconnection, relocation, native codecs and exact Undo/Redo.
#[test]
fn explicit_via_isolated_intent_persistence_and_history() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let id = via(&mut s, Some(&target), -30_000_000);
    let assigned = view(&s).vias()[0].net_assignment.unwrap();
    let inspect = view(&s).inspect_vias();
    assert_eq!(inspect[0]["assignment_mode"], "explicit");
    assert_eq!(inspect[0]["net_state"], "active");
    assert_eq!(inspect[0]["connected_to_pad"], false);
    assert!(inspect[0]["net"].is_string());
    assert!(view(&s).copper_nets().contains_key(&id));
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.board.vias[0].net_assignment, Some(assigned));
    assert!(saved.values().any(|b| String::from_utf8_lossy(b).contains("pcbcraft-board/24")));
    run(&mut s, crate::MOVE_TRACKS, json!({"vias":[id],"dx_nm":3_000_000,"dy_nm":0})).unwrap();
    assert_eq!(view(&s).vias()[0].net_assignment, Some(assigned));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Automatic mode stays unassigned in free space and clearing intent is one revision-checked edit.
#[test]
fn automatic_clear_noop_and_rejected_requests_are_atomic() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let id = via(&mut s, None, -30_000_000);
    assert_eq!(view(&s).inspect_vias()[0]["net"], Value::Null);
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"target":target})).unwrap();
    let address = view(&s).vias()[0].net_assignment.unwrap();
    let persistent = json!({"kind":"persistent","project":address.project,"scope":address.scope,"net":address.net});
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"target":persistent})).unwrap();
    assert_eq!(s.revision(), revision);
    for params in [
        json!({"vias":[id]}),
        json!({"vias":[id,id],"automatic":true}),
        json!({"vias":[id],"automatic":false}),
        json!({"vias":[id],"target":persistent,"automatic":true}),
        json!({"vias":[id],"target":{"kind":"persistent","project":Uuid::from_u128(22),"scope":address.scope,"net":address.net}}),
    ] {
        assert!(run(&mut s, SET_VIA_NET, params).is_err());
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
    assert!(s.apply(SET_VIA_NET, &json!({"vias":[id],"automatic":true,"expected_revision":revision-1})).is_err());
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"automatic":true})).unwrap();
    assert!(view(&s).vias()[0].net_assignment.is_none());
    assert!(!view(&s).copper_nets().contains_key(&id));
}

/// Two differently assigned via-only islands must refuse joins even without any pad in either.
#[test]
fn via_only_shorts_and_reassignment_are_refused_without_publication() {
    let mut s = routed_fixture();
    let targets = targets(&s);
    assert!(targets.len() >= 2);
    let a = via(&mut s, Some(&targets[0]), -30_000_000);
    let b = via(&mut s, Some(&targets[1]), -28_000_000);
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    for (command, params) in [
        (crate::MOVE_TRACKS, json!({"vias":[b],"dx_nm":-1_600_000,"dy_nm":0})),
        (
            crate::ADD_TRACKS,
            json!({"layer":copper_layers(&s)[0],"points":[{"x_nm":-30_000_000,"y_nm":-30_000_000},{"x_nm":-28_000_000,"y_nm":-30_000_000}]}),
        ),
    ] {
        assert!(run(&mut s, command, params).is_err());
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
    // Existing conflicting geometry can be reopened and diagnosed, without choosing either net.
    let mut invalid = snapshot.as_ref().clone();
    Arc::make_mut(&mut invalid.board).vias.iter_mut().find(|v| v.id == b).unwrap().at.x_nm = -29_600_000;
    s.open_snapshot(Arc::new(invalid));
    assert!(!view(&s).copper_nets().contains_key(&a));
    let report = view(&s).check_design(Rules::default());
    let short = report.violations.iter().find(|v| v.kind == Kind::Short).unwrap();
    assert!(short.items.contains(&a.to_string()) && short.items.contains(&b.to_string()));
    assert_eq!(short.nets.len(), 2);
    assert!(short.nets.iter().all(|n| n.terminals.is_empty()));
    assert!(run(&mut s, SET_VIA_NET, json!({"vias":[a],"target":targets[0]})).is_ok()); // unchanged intent is a no-op
}

/// Pad and routed-track contacts inherit automatically; explicit incompatible assignment fails.
#[test]
fn automatic_pad_and_track_inheritance_and_explicit_conflict() {
    let mut s = routed_fixture();
    let pad = view(&s).footprints().flat_map(|f| f.pads).find(|p| p.net.is_some()).unwrap();
    run(&mut s, crate::ADD_VIA, json!({"at":pad.center})).unwrap();
    let id = view(&s).vias()[0].id;
    assert_eq!(view(&s).copper_nets().get(&id), pad.net.as_ref());
    let current = view(&s).net_class_target_for_copper(&id.to_string()).unwrap().unwrap();
    let target = targets(&s)
        .into_iter()
        .find(|t| {
            let (p, a) = s.project().unwrap().with_via_target(t).unwrap();
            let _ = p;
            let (_, addressed) = s.project().unwrap().with_via_target(&json!(current)).unwrap();
            a.net != addressed.net
        })
        .unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(run(&mut s, SET_VIA_NET, json!({"vias":[id],"target":target})).is_err());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// The new vocabulary cannot be smuggled into legacy headers or represented by explicit null.
#[test]
fn native_assignment_version_and_null_guards_preserve_legacy_bytes() {
    let mut s = routed_fixture();
    let id = via(&mut s, None, -30_000_000);
    let legacy = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&legacy).unwrap()).unwrap(), legacy);
    let target = targets(&s).remove(0);
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"target":target})).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let key = saved.iter().find(|(_, b)| String::from_utf8_lossy(b).contains("pcbcraft-board/24")).unwrap().0.clone();
    for format in ["pcbcraft-board/13", "pcbcraft-board/19", "pcbcraft-board/21", "pcbcraft-board/22", "pcbcraft-board/23"] {
        let mut bad = saved.clone();
        bad.insert(key.clone(), String::from_utf8(saved[&key].clone()).unwrap().replace("pcbcraft-board/24", format).into_bytes());
        assert!(decode_documents(&bad).is_err());
    }
    let mut value: Value = serde_json::from_slice(&saved[&key]).unwrap();
    value["vias"][0]["net_assignment"] = Value::Null;
    let mut bad = saved.clone();
    bad.insert(key, serde_json::to_vec(&value).unwrap());
    assert!(decode_documents(&bad).is_err());
}

/// Via format 24 retains optional extended stack fields; clearing either feature preserves
/// exact bytes and the header required by the remaining authored vocabulary.
#[test]
fn native_assignment_accepts_optional_extended_stack_in_format_24() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let id = via(&mut s, Some(&target), -30_000_000);
    let original_setup = serde_json::to_value(view(&s).setup().unwrap()).unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut stack = original_setup.clone();
    stack["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    run(&mut s, crate::SET_BOARD_SETUP, json!({"setup":stack})).unwrap();
    let combined = encode_documents(s.snapshot().unwrap()).unwrap();
    let key = combined.iter().find(|(_, b)| String::from_utf8_lossy(b).contains("pcbcraft-board/24")).unwrap().0.clone();
    let reopened = decode_documents(&combined).unwrap();
    assert_eq!(reopened.board_setup().unwrap().stackup().finished_thickness_nm, Some(1_600_000));
    assert_eq!(reopened.board.vias[0].net_assignment, view(&s).vias()[0].net_assignment);
    assert_eq!(encode_documents(reopened).unwrap(), combined);
    for format in ["pcbcraft-board/21", "pcbcraft-board/22", "pcbcraft-board/23"] {
        let mut bad = combined.clone();
        bad.insert(key.clone(), String::from_utf8(combined[&key].clone()).unwrap().replace("pcbcraft-board/24", format).into_bytes());
        assert!(decode_documents(&bad).is_err(), "{format} must not accept both vocabularies");
    }
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"automatic":true})).unwrap();
    let stacked = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&stacked[&key]).contains("pcbcraft-board/23"));
    assert_eq!(encode_documents(decode_documents(&stacked).unwrap()).unwrap(), stacked);
    let mut missing_assignment = stacked.clone();
    missing_assignment
        .insert(key.clone(), String::from_utf8(stacked[&key].clone()).unwrap().replace("pcbcraft-board/23", "pcbcraft-board/24").into_bytes());
    assert!(decode_documents(&missing_assignment).is_err());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), combined);
    run(&mut s, crate::SET_BOARD_SETUP, json!({"setup":original_setup})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    assert_eq!(encode_documents(decode_documents(&original).unwrap()).unwrap(), original);
}

/// Same-board clipboard copies retain intent; foreign destinations cannot silently reuse IDs.
#[test]
fn clipboard_same_board_preserves_assignment_and_foreign_paste_is_atomic() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let id = via(&mut s, Some(&target), -30_000_000);
    let fragment = BoardFragment::capture(s.project().unwrap(), &BoardSelection { vias: BTreeSet::from([id]), ..Default::default() }, false).unwrap();
    let text = fragment.to_text().unwrap();
    assert!(text.contains("pcbcraft-clipboard/4"));
    let fragment = BoardFragment::from_text(&text).unwrap();
    let options = BoardPasteOptions::default();
    let (candidate, selected) = fragment.placed(s.project().unwrap(), Point { x_nm: -20_000_000, y_nm: -30_000_000 }, 10_000, &options).unwrap();
    let copy = candidate.board.vias.iter().find(|v| selected.vias.contains(&v.id)).unwrap();
    assert_eq!(copy.net_assignment, view(&s).vias()[0].net_assignment);
    let foreign = routed_fixture();
    assert!(fragment.placed(foreign.project().unwrap(), Point { x_nm: -20_000_000, y_nm: -30_000_000 }, 10_000, &options).is_err());
}

/// Renames retain identity, while retirement and missing/foreign references stay readable and diagnosed.
#[test]
fn rename_retirement_and_unresolved_native_targets_never_retarget() {
    let mut s = routed_fixture();
    run(&mut s, crate::net_labels::ADD, json!({"name":"GND","style":"global","x_nm":-50_000_000,"y_nm":0})).unwrap();
    let label = s.project().unwrap().net_labels().iter().find(|l| l.name == "GND").unwrap().id;
    let target = s.project().unwrap().inspect_net_registry().unwrap()["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "GND")
        .map(|r| json!({"kind":"derived","anchor":r["anchor"]}))
        .unwrap();
    let id = via(&mut s, Some(&target), -30_000_000);
    let address = view(&s).vias()[0].net_assignment.unwrap();
    run(&mut s, crate::net_labels::SET, json!({"id":format!("net-label-{label}"),"name":"GROUND","style":"global","x_nm":-50_000_000,"y_nm":0}))
        .unwrap();
    assert_eq!(view(&s).vias()[0].net_assignment, Some(address));
    assert_eq!(view(&s).copper_nets()[&id], "GROUND");
    run(&mut s, crate::net_labels::DELETE, json!({"id":format!("net-label-{label}")})).unwrap();
    assert_eq!(view(&s).inspect_vias()[0]["net_state"], "retired");
    assert!(!view(&s).copper_nets().contains_key(&id));
    let report = view(&s).check_design(Rules::default());
    assert!(report.violations.iter().any(|v| v.kind == Kind::ViaNetUnresolved && v.items.contains(&id.to_string())));
    let mut job = s.design_check_job(&json!({})).unwrap();
    while !job.step(3) {}
    assert_eq!(job.take_report().unwrap(), report);
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    for (address, status) in [
        (crate::nets::NetAddress { net: Uuid::from_u128(9876), ..address }, "missing"),
        (crate::nets::NetAddress { project: Uuid::from_u128(9876), ..address }, "foreign"),
    ] {
        let mut project = s.snapshot().unwrap().as_ref().clone();
        Arc::make_mut(&mut project.board).vias[0].net_assignment = Some(address);
        let docs = encode_documents(Arc::new(project)).unwrap();
        let p = decode_documents(&docs).unwrap();
        assert_eq!(p.via_net_state(p.board.vias[0].net_assignment.unwrap()), status);
        assert_eq!(encode_documents(p).unwrap(), docs);
    }
}

/// An assigned island receives class defaults and hard limits without requiring a pad contact.
#[test]
fn isolated_assignment_applies_class_defaults_targets_and_probe_rules() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    run(&mut s, crate::net_classes::CREATE, json!({"name":"Power via","rules":{"min_via_diameter_nm":800_000,"preferred_via_diameter_nm":1_000_000,"preferred_via_drill_nm":400_000,"clearance_nm":650_000}})).unwrap();
    let class = s.project().unwrap().net_classes().definitions.last().unwrap().id;
    run(&mut s, crate::nets::SET_CLASSES, json!({"target":target,"classes":[class]})).unwrap();
    let id = via(&mut s, Some(&target), -30_000_000);
    let v = &view(&s).vias()[0];
    assert_eq!((v.diameter_nm, v.drill_nm), (1_000_000, 400_000));
    let policy = view(&s).route_rules_at(v.at, None).unwrap().unwrap();
    assert_eq!(policy.nets[0], v.net_assignment.unwrap());
    assert_eq!(policy.clearance_nm().unwrap(), 650_000);
    let query = s.clearance_query(view(&s).target, 200_000).unwrap();
    assert!(query.authority().class_rules().unwrap().rules_for(&id.to_string()).is_some());
    let before = s.snapshot().unwrap();
    assert!(run(&mut s, crate::SET_VIA_SIZE, json!({"vias":[id],"via_diameter_nm":600_000,"via_drill_nm":300_000})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(view(&s).check_design(Rules::default()).violations.iter().any(|v| v.kind == Kind::Unconnected && v.items.contains(&id.to_string())));
}

/// Preparation can be dropped or finished after a peer revision/reopen without granting publication.
#[test]
fn prepared_assignment_cancellation_revision_and_replacement_keep_history() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let id = via(&mut s, None, -30_000_000);
    let snapshot = s.snapshot().unwrap();
    let params = json!({"vias":[id],"target":target,"expected_revision":s.revision()});
    let mut canceled = s.prepare_copper_edit(SET_VIA_NET, &params).unwrap();
    assert!(!canceled.step(1));
    drop(canceled);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    for reopen in [false, true] {
        let params = json!({"vias":[id],"target":target,"expected_revision":s.revision()});
        let mut job = s.prepare_copper_edit(SET_VIA_NET, &params).unwrap();
        while !job.step(1) {}
        let prepared = job.take_result().unwrap().unwrap();
        if reopen {
            s.open_snapshot(snapshot.clone());
        } else {
            s.apply("project.rename", &json!({"name":"Peer edit"})).unwrap();
        }
        let current = s.snapshot().unwrap();
        assert!(s.commit_copper_edit(prepared).is_err());
        assert!(Arc::ptr_eq(&current, &s.snapshot().unwrap()));
        assert!(view(&s).vias()[0].net_assignment.is_none());
    }
}

/// A pending split cannot alias explicit identity to old accepted copper by a surviving name.
#[test]
fn pending_schematic_split_keeps_accepted_copper_separate_until_review() {
    let mut s = routed_fixture();
    let pad = view(&s).footprints().flat_map(|f| f.pads.into_iter().map(move |p| (f.id.clone(), p))).find(|(_, p)| p.net.is_some()).unwrap();
    let selected = format!("{}:{}", pad.0, pad.1.number);
    let target = json!(view(&s).net_class_target_for_copper(&selected).unwrap().unwrap());
    let id = via(&mut s, Some(&target), -30_000_000);
    let assignment = view(&s).vias()[0].net_assignment;
    let wires: Vec<_> = s.project().unwrap().wires().iter().map(|w| w.id()).collect();
    for wire in wires {
        run(&mut s, crate::DELETE_WIRE, json!({"wire":wire})).unwrap();
    }
    assert_eq!(view(&s).vias()[0].net_assignment, assignment);
    if s.project().unwrap().via_net_state(assignment.unwrap()) == "active" {
        let p = s.project().unwrap();
        let geometry = super::super::clearance::BoardGeometry::new(view(&s));
        let via = geometry.items.iter().find(|i| i.name == id.to_string()).unwrap();
        let pad_item = geometry.items.iter().find(|i| i.name == selected).unwrap();
        assert_ne!(via.intended_nets, pad_item.intended_nets);
        assert!(p.board_intent_memberships().ids[&assignment.unwrap().net] >= p.board.baseline.as_ref().unwrap().nets.len());
    }
}

/// Assignment changes names/intent without changing the physical copper island projection.
#[test]
fn assignment_never_changes_physical_island_groups() {
    let mut s = routed_fixture();
    let mut project = s.snapshot().unwrap().as_ref().clone();
    let board = Arc::make_mut(&mut project.board);
    let first = board.footprints[0].clone();
    board.footprints[1].x_nm = first.x_nm;
    board.footprints[1].y_nm = first.y_nm;
    s.open_snapshot(Arc::new(project));
    let id = via(&mut s, None, -30_000_000);
    let copper = view(&s).copper();
    assert_eq!(view(&s).copper_with_nets().0, copper);
    let target = targets(&s).remove(0);
    run(&mut s, SET_VIA_NET, json!({"vias":[id],"target":target})).unwrap();
    assert_eq!(view(&s).copper(), copper);
    assert_eq!(view(&s).copper_with_nets().0, copper);
}

/// Tracks and routing-inserted/standalone automatic vias inherit a via-only island's explicit seed.
#[test]
fn route_and_automatic_vias_inherit_assigned_island_without_pads() {
    let mut s = routed_fixture();
    let target = targets(&s).remove(0);
    let seed = via(&mut s, Some(&target), -30_000_000);
    let net = view(&s).copper_nets()[&seed].clone();
    let layers = copper_layers(&s);
    run(&mut s,crate::ADD_TRACKS,json!({"layer":layers[0],"points":[{"x_nm":-30_000_000,"y_nm":-30_000_000},{"x_nm":-32_000_000,"y_nm":-30_000_000},{"x_nm":-34_000_000,"y_nm":-30_000_000}],"layer_changes":[{"at":1,"layer":layers[1]}]})).unwrap();
    run(&mut s, crate::ADD_VIA, json!({"at":{"x_nm":-34_000_000,"y_nm":-30_000_000}})).unwrap();
    let view = view(&s);
    let nets = view.copper_nets();
    assert_eq!(view.vias().len(), 3);
    assert!(view.vias().iter().filter(|v| v.id != seed).all(|v| v.net_assignment.is_none()));
    assert!(view.vias().iter().all(|v| nets[&v.id] == net));
    assert!(view.tracks().iter().all(|t| nets[&t.id] == net));
    assert!(view.inspect_vias().iter().all(|v| v["connected_to_pad"] == false));
}
