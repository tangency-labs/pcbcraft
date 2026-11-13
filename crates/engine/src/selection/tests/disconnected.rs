//! Move cuts the selection boundary; connected translation remains the Grab contract.
use super::*;
use std::collections::BTreeMap;

/// Apply an ordinary shared command at the session's current revision.
fn run(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Preview and command use the same candidate, with native roundtrip and one Undo/Redo step.
fn commit(s: &mut Session, items: &Selection, delta: Point, auto_heal: bool) -> Arc<Project> {
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let preview = items.transformed(&before, delta, true, revision, auto_heal).unwrap();
    run(s, MOVE_ITEMS, json!({"items":items,"dx_nm":delta.x_nm,"dy_nm":delta.y_nm,"disconnect":true,"auto_heal":auto_heal}));
    let after = s.snapshot().unwrap();
    assert!(s.revision() > revision);
    crate::assert_same_content(&Arc::new(preview), &after);
    let docs = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&after, &s.snapshot().unwrap());
    after
}

/// A wire moves as an intact route, leaving both external pins fixed rather than growing
/// connecting legs. Omitting disconnect still retains the pre-existing connected behavior.
#[test]
fn wire_move_disconnects_external_pins_while_grab_keeps_them() {
    for auto_heal in [false, true] {
        let mut s = fixture();
        let before = s.snapshot().unwrap();
        let items = Selection::one_wire(before.wires()[0].id());
        let delta = Point { x_nm: 0, y_nm: 8 * GRID_NM };
        let grab = items.transformed(&before, delta, false, s.revision(), auto_heal).unwrap();
        assert_eq!(grab.wires()[0].endpoints(), before.wires()[0].endpoints());
        let after = commit(&mut s, &items, delta, auto_heal);
        assert_eq!(after.symbols(), before.symbols());
        assert!(after.wires()[0].terminals().is_empty());
        assert_eq!(
            after.wire_path(&after.wires()[0]),
            before.wire_path(&before.wires()[0]).into_iter().map(|at| translated(at, delta).unwrap()).collect::<Vec<_>>()
        );
    }
}

/// A selected symbol/wire retains their internal terminal while both directions of the
/// boundary are cut: the unselected branch stays fixed and the other unselected pin stays fixed.
#[test]
fn mixed_move_preserves_internal_pin_and_freezes_unselected_branch() {
    for auto_heal in [false, true] {
        let mut s = fixture();
        let p = s.snapshot().unwrap();
        let terminal = p.wires()[0].terminals()[0];
        let at = p.endpoint_position(WireEndpoint::Terminal(terminal)).unwrap();
        run(&mut s, DRAW_WIRE, json!({"from":terminal.inspect(),"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12*GRID_NM},"bends":[],"auto_heal":false}));
        let before = s.snapshot().unwrap();
        let items =
            Selection { symbols: [format!("symbol-{}", terminal.symbol)].into(), wires: [before.wires()[0].id()].into(), ..Selection::default() };
        let delta = Point { x_nm: 5 * GRID_NM, y_nm: 8 * GRID_NM };
        let after = commit(&mut s, &items, delta, auto_heal);
        assert!(after.wires()[0].endpoints().contains(&WireEndpoint::Terminal(terminal)));
        assert_eq!(after.wires()[0].terminals(), vec![terminal]);
        assert_eq!(
            after.wire_path(&after.wires()[0]),
            before.wire_path(&before.wires()[0]).into_iter().map(|at| translated(at, delta).unwrap()).collect::<Vec<_>>()
        );
        assert_eq!(after.wire_path(&after.wires()[1]), before.wire_path(&before.wires()[1]));
        assert!(!after.wires()[1].endpoints().contains(&WireEndpoint::Terminal(terminal)));
        assert_eq!(after.symbols()[1], before.symbols()[1]);
    }
}

/// One boundary cut partitions a shared junction once; selected wires keep their common ID
/// and endpoint intent while the stationary branch and its endpoint marker remain fixed.
#[test]
fn shared_junction_move_partitions_once_and_preserves_marker_owners() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    run(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":10*GRID_NM,"y_nm":0},"bends":[],"auto_heal":false}));
    let shared = s.project().unwrap().wires()[0].endpoints()[1];
    for at in [Point { x_nm: 10 * GRID_NM, y_nm: 10 * GRID_NM }, Point { x_nm: 20 * GRID_NM, y_nm: 0 }] {
        run(&mut s, DRAW_WIRE, json!({"from":shared.inspect(),"to":at,"bends":[],"auto_heal":false}));
    }
    let mut targets = Vec::new();
    for x in [0, 20 * GRID_NM] {
        let target = s.project().unwrap().wire_end_targets().find(|(_, at)| *at == Point { x_nm: x, y_nm: 0 }).unwrap().0;
        run(&mut s, crate::no_connect::SET, json!({"target":target,"marked":true}));
        targets.push(crate::no_connect::Attachment::WireEnd(target));
    }
    let before = s.snapshot().unwrap();
    let items = Selection {
        wires: before.wires()[..2].iter().map(|wire| wire.id()).collect(),
        no_connects: [targets[0].clone()].into(),
        ..Selection::default()
    };
    let delta = Point { x_nm: 0, y_nm: 5 * GRID_NM };
    let after = commit(&mut s, &items, delta, false);
    assert_eq!(after.wires()[0].endpoints()[1], shared);
    assert_eq!(after.wires()[1].endpoints()[0], shared);
    assert_ne!(after.wires()[2].endpoints()[0], shared);
    for (i, wire) in before.wires().iter().enumerate() {
        let path = before.wire_path(wire).into_iter().map(|at| if i < 2 { translated(at, delta).unwrap() } else { at }).collect::<Vec<_>>();
        assert_eq!(after.wire_path(&after.wires()[i]), path);
    }
    let marks: BTreeMap<_, _> = after.no_connect_appearances().collect();
    assert_eq!(marks[&targets[0]], Point { x_nm: 0, y_nm: 5 * GRID_NM });
    assert_eq!(marks[&targets[1]], Point { x_nm: 20 * GRID_NM, y_nm: 0 });
}

/// Selected and stationary named anchors obey the same boundary partition as component pins.
#[test]
fn named_anchor_move_cuts_only_unselected_contacts() {
    for mode in ["label", "wire", "both"] {
        let mut s = fixture();
        run(&mut s, crate::net_labels::ADD, json!({"name":"BOUNDARY","style":"local","x_nm":0,"y_nm":20*GRID_NM}));
        let label = s.project().unwrap().net_labels()[0].clone();
        run(
            &mut s,
            DRAW_WIRE,
            json!({"from":{"net_label":label.object_id()},"to":{"x_nm":10*GRID_NM,"y_nm":20*GRID_NM},"bends":[],"auto_heal":false}),
        );
        let before = s.snapshot().unwrap();
        let mut items = Selection::default();
        if mode != "wire" {
            items.net_labels.insert(label.object_id());
        }
        if mode != "label" {
            items.wires.insert(before.wires()[1].id());
        }
        let delta = Point { x_nm: 0, y_nm: 5 * GRID_NM };
        let after = commit(&mut s, &items, delta, true);
        assert_eq!(after.wire_path(&after.wires()[0]), before.wire_path(&before.wires()[0]));
        assert_eq!(after.net_labels()[0].at, if mode == "wire" { label.at } else { translated(label.at, delta).unwrap() });
        assert_eq!(after.wires()[1].endpoints().contains(&WireEndpoint::NetLabel(label.id)), mode == "both");
        let path = before
            .wire_path(&before.wires()[1])
            .into_iter()
            .map(|at| if mode == "label" { at } else { translated(at, delta).unwrap() })
            .collect::<Vec<_>>();
        assert_eq!(after.wire_path(&after.wires()[1]), path);
    }
}

/// No-op moves allocate nothing; invalid parameters, stale revisions, exhausted IDs and
/// out-of-bounds displacements cannot publish a partially detached graph or change history.
#[test]
fn disconnected_group_rejections_and_zero_move_are_atomic() {
    let mut s = fixture();
    let items = all(&s);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    run(&mut s, MOVE_ITEMS, json!({"items":items,"dx_nm":0,"dy_nm":0,"disconnect":true}));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    for params in [
        json!({"items":items,"dx_nm":GRID_NM,"dy_nm":0,"disconnect":"yes","expected_revision":revision}),
        json!({"items":items,"dx_nm":GRID_NM,"dy_nm":0,"disconnect":true,"expected_revision":revision-1}),
        json!({"items":items,"dx_nm":i64::MAX,"dy_nm":0,"disconnect":true,"expected_revision":revision}),
        json!({"items":{"symbols":[],"wires":["wire-999999"]},"dx_nm":GRID_NM,"dy_nm":0,"disconnect":true,"expected_revision":revision}),
    ] {
        assert!(s.apply(MOVE_ITEMS, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
    }
    let mut exhausted = (*before).clone();
    exhausted.numeric_id_high_water = Some(crate::MAX_OBJECT_ID);
    let wire = Selection::one_wire(before.wires()[0].id());
    assert!(wire.transformed(&exhausted, Point { x_nm: 0, y_nm: GRID_NM }, true, revision, false).is_err());
    assert_eq!(crate::command(MOVE_ITEMS).unwrap().to_json()["params"]["properties"]["disconnect"]["default"], false);
}
