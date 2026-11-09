use super::*;
use crate::{CONNECT_PINS, DELETE_ITEMS, DRAW_WIRE, MOVE_ITEMS, PLACE_FOOTPRINT, PLACE_SYMBOL, persistence, schematic::GRID_NM};
use serde_json::json;

/// Two components with an authored wire and a footprint exercise every dependent collection.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,"name":"Group edits"})).unwrap();
    for x in [0, 20 * GRID_NM] {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(
        DRAW_WIRE,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(PLACE_FOOTPRINT, &json!({"symbol":"symbol-2","definition":crate::board::FOOTPRINT_DEFINITION,"pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    s
}

/// Select every authored object using identities from inspection, not hand-written project files.
fn all(s: &Session) -> Selection {
    let p = s.project().unwrap();
    Selection { symbols: p.symbols().iter().map(|s| s.id()).collect(), wires: p.wires().iter().map(|w| w.id()).collect(), ..Selection::default() }
}

/// Mixed moves preserve topology and relative routes, and both move/delete commit one undo step.
/// The result reopens and re-encodes without changing bytes or stable identities.
#[test]
fn mixed_move_and_delete_are_atomic_and_round_trip() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let items = all(&s);
    let revision = s.revision();
    let delta = Point { x_nm: 5 * GRID_NM, y_nm: 4 * GRID_NM };
    s.apply(MOVE_ITEMS, &json!({"items":items,"dx_nm":delta.x_nm,"dy_nm":delta.y_nm,"expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let moved = s.snapshot().unwrap();
    for (a, b) in before.symbols().iter().zip(moved.symbols()) {
        assert_eq!(a.id(), b.id());
        assert_eq!(b.position, translated(a.position, delta).unwrap());
    }
    let path = before.wire_path(&before.wires()[0]).into_iter().map(|p| translated(p, delta).unwrap()).collect::<Vec<_>>();
    assert_eq!(moved.wire_path(&moved.wires()[0]), path);
    assert_eq!(before.inspect_schematic()["netlist"], moved.inspect_schematic()["netlist"]);
    let encoded = persistence::encode(moved.clone()).unwrap();
    let reopened = persistence::decode(&encoded).unwrap();
    assert_eq!(persistence::encode(reopened).unwrap(), encoded);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&moved, &s.snapshot().unwrap()));
    s.apply(DELETE_ITEMS, &json!({"items":items,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    assert!(p.symbols().is_empty() && p.wires().is_empty());
    assert_eq!(p.components().count(), 0);
    assert!(p.board.footprints().is_empty());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&moved, &s.snapshot().unwrap()));
}

/// Moving a wire alone keeps component pins fixed and adds orthogonal end legs to its new route.
#[test]
fn wire_move_keeps_external_pins_and_moves_shared_junctions_once() {
    let mut s = fixture();
    let original = s.snapshot().unwrap();
    let wire = original.wires()[0].id();
    s.apply(MOVE_ITEMS, &json!({"items":Selection::one_wire(wire),"dx_nm":0,"dy_nm":8*GRID_NM,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    assert_eq!(original.symbols(), p.symbols());
    let path = p.wire_path(&p.wires()[0]);
    assert_eq!(path.first(), original.wire_path(&original.wires()[0]).first());
    assert_eq!(path.last(), original.wire_path(&original.wires()[0]).last());
    assert!(path.iter().any(|point| point.y_nm == 8 * GRID_NM));
    assert!(path.windows(2).all(|p| p[0].x_nm == p[1].x_nm || p[0].y_nm == p[1].y_nm));

    s.apply(
        DRAW_WIRE,
        &json!({"from":{"x_nm":0,"y_nm":20*GRID_NM},"to":{"x_nm":10*GRID_NM,"y_nm":20*GRID_NM},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    let joint = s.project().unwrap().wires().last().unwrap().endpoints()[1];
    s.apply(DRAW_WIRE, &json!({"from":joint.inspect(),"to":{"x_nm":10*GRID_NM,"y_nm":30*GRID_NM},"bends":[],"expected_revision":s.revision()}))
        .unwrap();
    let p = s.project().unwrap();
    let point = p.endpoint_position(joint).unwrap();
    let wires = p.wires().iter().skip(1).map(|w| w.id()).collect();
    s.apply(
        MOVE_ITEMS,
        &json!({"items":Selection { wires, ..Selection::default() },"dx_nm":GRID_NM,"dy_nm":GRID_NM,"expected_revision":s.revision()}),
    )
    .unwrap();
    assert_eq!(s.project().unwrap().endpoint_position(joint).unwrap(), translated(point, Point { x_nm: GRID_NM, y_nm: GRID_NM }).unwrap());
}

/// Unknown/duplicate IDs, overflow, stale revisions reject the whole edit.
/// A zero delta creates no revision, undo entry, saved-file change or normalization noise.
#[test]
fn invalid_group_edits_and_noop_moves_leave_everything_unchanged() {
    let mut s = fixture();
    let before = s.inspect();
    let items = all(&s);
    for params in [
        json!({"items":items,"dx_nm":crate::schematic::MAX_CENTER_NM,"dy_nm":0,"expected_revision":s.revision()}),
        json!({"items":items,"dx_nm":i64::MAX,"dy_nm":0,"expected_revision":s.revision()}),
        json!({"items":items,"dx_nm":0,"dy_nm":0,"expected_revision":0}),
        json!({"items":{"symbols":["symbol-2","symbol-999"],"wires":[]},"dx_nm":1,"dy_nm":0,"expected_revision":s.revision()}),
        json!({"items":{"symbols":["symbol-2","symbol-2"],"wires":[]},"dx_nm":1,"dy_nm":0,"expected_revision":s.revision()}),
        json!({"items":{"symbols":["symbol-02"],"wires":[]},"dx_nm":1,"dy_nm":0,"expected_revision":s.revision()}),
        json!({"items":{"symbols":[],"wires":[]},"dx_nm":1,"dy_nm":0,"expected_revision":s.revision()}),
    ] {
        assert!(s.apply(MOVE_ITEMS, &params).is_err(), "{params}");
        assert_eq!(s.inspect(), before);
    }
    for items in [json!({"symbols":["symbol-2"],"wires":["wire-999"]}), json!({"symbols":[],"wires":[],"extra":true})] {
        assert!(s.apply(DELETE_ITEMS, &json!({"items":items,"expected_revision":s.revision()})).is_err());
        assert_eq!(s.inspect(), before);
    }
    let saved = persistence::encode(s.snapshot().unwrap()).unwrap();
    s.apply(MOVE_ITEMS, &json!({"items":items,"dx_nm":0,"dy_nm":0,"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.inspect(), before);
    assert_eq!(persistence::encode(s.snapshot().unwrap()).unwrap(), saved);
    assert_eq!(crate::command(MOVE_ITEMS).unwrap().to_json()["params"]["properties"]["items"]["properties"]["symbols"]["uniqueItems"], true);
    // An implicit wire path can be moved by the same command.
    s.apply("edit.undo", &Value::Null).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(MOVE_ITEMS, &json!({"items":all(&s),"dx_nm":0,"dy_nm":GRID_NM,"expected_revision":s.revision()})).unwrap();
}

/// Displacements can span the entire valid sheet; bounds apply to the resulting positions.
#[test]
fn group_move_accepts_a_displacement_across_both_coordinate_limits() {
    let mut s = fixture();
    let limit = crate::schematic::MAX_CENTER_NM;
    s.apply(crate::MOVE_SYMBOL, &json!({"symbol":"symbol-2","x_nm":limit,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    s.apply(MOVE_ITEMS, &json!({"items":Selection::one_symbol("symbol-2".to_owned()),"dx_nm":-2*limit,"dy_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    assert_eq!(s.project().unwrap().symbols()[0].position.x_nm, -limit);
}

/// Named anchors participate in the same atomic moves/deletes as components and wires, including
/// attached endpoints, cache invalidation, old-client selections and rollback on invalid IDs.
#[test]
fn named_anchors_move_delete_and_undo_with_shared_selection() {
    let mut s = fixture();
    for (style, x) in [("ground", 30), ("power", 40), ("local", 50), ("global", 60)] {
        s.apply(crate::net_labels::ADD, &json!({"name":format!("N{x}"),"style":style,"x_nm":x*GRID_NM,"y_nm":0,"expected_revision":s.revision()}))
            .unwrap();
    }
    let labels = s.project().unwrap().net_labels().to_vec();
    s.apply(
        DRAW_WIRE,
        &json!({"from":labels[0].inspect()["endpoint"],"to":{"x_nm":30*GRID_NM,"y_nm":10*GRID_NM},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    let mut items = all(&s);
    items.net_labels = labels.iter().map(|label| label.object_id()).collect();
    assert!(s.project().unwrap().healed_wires(&items, s.revision()).is_err());
    let before = s.snapshot().unwrap();
    let old_netlist = before.inspect_schematic()["netlist"].clone();
    let delta = Point { x_nm: GRID_NM, y_nm: 2 * GRID_NM };
    let preview = items.moved(&before, delta).unwrap();
    let revision = s.revision();
    s.apply(MOVE_ITEMS, &json!({"items":items,"dx_nm":delta.x_nm,"dy_nm":delta.y_nm,"auto_heal":false,"expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let moved = s.snapshot().unwrap();
    assert_eq!(moved.net_labels(), preview.net_labels());
    for (old, new) in labels.iter().zip(moved.net_labels()) {
        assert_eq!(new.at, translated(old.at, delta).unwrap());
        assert_eq!(new.id, old.id);
    }
    assert_eq!(old_netlist, moved.inspect_schematic()["netlist"]);
    assert_eq!(moved.endpoint_position(WireEndpoint::NetLabel(labels[0].id)), Some(translated(labels[0].at, delta).unwrap()));
    assert!(s.apply(DELETE_ITEMS, &json!({"items":items,"expected_revision":revision})).is_err());
    let mut invalid = items.clone();
    invalid.net_labels.insert("net-label-99999".into());
    assert!(s.apply(DELETE_ITEMS, &json!({"items":invalid,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&moved, &s.snapshot().unwrap()));
    let labels_only = Selection { net_labels: items.net_labels.clone(), ..Selection::default() };
    s.apply(DELETE_ITEMS, &json!({"items":labels_only,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    assert!(p.net_labels().is_empty());
    assert_eq!(p.symbols(), moved.symbols());
    assert_eq!(p.wires().len(), moved.wires().len() - 1);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&moved, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(Selection::parse(&json!({"symbols":[],"wires":[]})).unwrap().is_empty());
    for bad in [json!(["net-label-1", "net-label-1"]), json!(["symbol-1"]), json!(null)] {
        assert!(Selection::parse(&json!({"symbols":[],"wires":[],"net_labels":bad})).is_none());
    }
}
