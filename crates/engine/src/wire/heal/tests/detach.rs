use super::*;
use crate::{BREAK_WIRE, MOVE_WIRE_SEGMENT};

/// Break a middle leg through the shared command, retaining three independent pieces.
fn broken_route() -> (Session, String) {
    let mut s = session();
    run(&mut s, DRAW_WIRE, json!({"from":p(-8,0),"to":p(16,6),"bends":[p(8,0),p(8,6)]}));
    let wire = s.project().unwrap().wires[0].id();
    run(&mut s, BREAK_WIRE, json!({"wire":wire,"at":p(8,3)}));
    let wire = s.project().unwrap().wires.iter().find(|w| s.project().unwrap().wire_path(w) == vec![p(8, 0), p(8, 6)]).unwrap().id();
    (s, wire)
}

/// A broken straight piece carries its free ends away instead of growing overlapping tails
/// back to its former neighbors. Both cleanup modes preserve the electrical break.
#[test]
fn broken_segment_moves_away_with_its_free_ends() {
    for auto_heal in [false, true] {
        for (dx, dy) in [(4, 0), (0, 4), (4, 4), (-4, -4)] {
            let (mut s, wire) = broken_route();
            let before = s.snapshot().unwrap();
            run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(8,3),"to":p(8+dx,3+dy),"auto_heal":auto_heal}));
            let after = s.snapshot().unwrap();
            let moved = after.wires.iter().find(|w| w.id() == wire).unwrap();
            assert_eq!(after.wire_path(moved), vec![p(8 + dx, dy), p(8 + dx, 6 + dy)]);
            assert_eq!(after.connectivity().nets.len(), 3);
            for old in before.wires.iter().filter(|w| w.id() != wire) {
                assert_eq!(after.wire_path(after.wires.iter().find(|w| w.id() == old.id()).unwrap()), before.wire_path(old));
            }
            s.apply("edit.undo", &Value::Null).unwrap();
            assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
            s.apply("edit.redo", &Value::Null).unwrap();
            assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));
        }
    }
}

/// Returning a free piece to its original location can explicitly join both old contacts.
/// A no-travel drop with no connection request remains the unchanged broken snapshot.
#[test]
fn broken_segment_can_rejoin_its_old_contacts_when_requested() {
    let (mut s, wire) = broken_route();
    let before = s.snapshot().unwrap();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(8,3),"to":p(8,3)}));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(8,3),"to":p(20,3)}));
    let ends: Vec<_> =
        s.project().unwrap().wires.iter().find(|w| w.id() == wire).unwrap().endpoints.iter().map(|end| end.inspect()["junction"].clone()).collect();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(20,3),"to":p(8,3),"connect_ends":ends}));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), 1);
}

/// Disconnect-and-move leaves every old path in place, retaining branch connectivity at
/// each vacated pin. A replacement at the old location joins the preserved wire ends.
#[test]
fn disconnecting_a_symbol_keeps_wiring_for_a_replacement() {
    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.project().unwrap().symbols[0].clone();
    let pins = symbol.terminals();
    let (left, right) = (WireEndpoint::Terminal(pins[0]), WireEndpoint::Terminal(pins[1]));
    for (pin, to) in [(left, p(-12, 0)), (left, p(-4, -8)), (right, p(12, 0))] {
        run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":to,"bends":[]}));
    }
    let before = s.snapshot().unwrap();
    let paths: Vec<_> = before.wires.iter().map(|w| before.wire_path(w)).collect();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol.id(),"x_nm":0,"y_nm":p(0,16).y_nm,"disconnect":true}));
    let after = s.snapshot().unwrap();
    assert_eq!(after.wires.iter().map(|w| after.wire_path(w)).collect::<Vec<_>>(), paths);
    assert!(after.wires.iter().all(|w| !w.terminals().iter().any(|pin| pin.symbol == symbol.ordinal)));
    assert_eq!(after.wires[0].endpoints[0], after.wires[1].endpoints[0]);
    let saved = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let replacement = s.project().unwrap().symbols[1].terminals();
    assert!(s.project().unwrap().wires.iter().any(|w| w.endpoints.contains(&WireEndpoint::Terminal(replacement[0]))));
    assert!(s.project().unwrap().wires.iter().any(|w| w.endpoints.contains(&WireEndpoint::Terminal(replacement[1]))));
}

/// Explicit drop contacts still connect new stationary targets, including two points on one
/// route. Pin endpoints and shared junctions stay anchored when a straight leg slides.
#[test]
fn straight_piece_drop_keeps_normal_joining_and_fixed_connections() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(0, 6));
    let wire = s.project().unwrap().wires[0].id();
    let ends: Vec<_> = s.project().unwrap().wires[0].endpoints.iter().map(|end| end.inspect()["junction"].clone()).collect();
    run(&mut s, DRAW_WIRE, json!({"from":p(10,-4),"to":p(10,10),"bends":[]}));
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,3),"to":p(10,3),"connect_ends":ends}));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), 1);

    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let pin = WireEndpoint::Terminal(s.project().unwrap().symbols[0].terminals()[1]);
    run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":p(12,0),"bends":[]}));
    let wire = s.project().unwrap().wires[0].id();
    let anchored = s.snapshot().unwrap();
    assert!(!anchored.wire_moves_freely(&wire));
    assert!(s.apply(MOVE_WIRE_SEGMENT, &json!({"wire":wire,"at":p(8,0),"to":p(10,2),"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&anchored, &s.snapshot().unwrap()));
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(8,0),"to":p(8,4)}));
    assert_eq!(s.project().unwrap().wire_path(&s.project().unwrap().wires[0]), vec![p(4, 0), p(4, 4), p(12, 4)]);
    assert_eq!(s.project().unwrap().wires[0].endpoints[0], pin);

    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    let joint = s.project().unwrap().wires[0].endpoints[0];
    let wire = s.project().unwrap().wires[0].id();
    run(&mut s, DRAW_WIRE, json!({"from":joint.inspect(),"to":p(0,-6),"bends":[]}));
    run(&mut s, DRAW_WIRE, json!({"from":joint.inspect(),"to":p(-6,0),"bends":[]}));
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(5,0),"to":p(5,4)}));
    assert_eq!(s.project().unwrap().endpoint_position(joint), Some(p(0, 0)));
    assert_eq!(s.project().unwrap().wire_path(&s.project().unwrap().wires[0]), vec![p(0, 0), p(0, 4), p(10, 4)]);
}

/// Bad drop intents and stale disconnections fail atomically; an unchanged symbol position
/// adds no disconnect or history step. Strict discovery publishes the optional modes.
#[test]
fn invalid_drop_and_disconnect_requests_leave_history_unchanged() {
    let (mut s, wire) = broken_route();
    let before = s.snapshot().unwrap();
    let unrelated = before.wires.iter().find(|w| w.id() != wire).unwrap().endpoints[0].inspect()["junction"].clone();
    for ends in [json!([unrelated]), json!(["junction-999"]), json!(["junction-2", "junction-2"]), json!(["junction-2", "junction-3", "junction-4"])]
    {
        let result = s.apply(MOVE_WIRE_SEGMENT, &json!({"wire":wire,"at":p(8,3),"to":p(20,3),"connect_ends":ends,"expected_revision":s.revision()}));
        assert!(result.is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.project().unwrap().symbols[0].id();
    let before = s.snapshot().unwrap();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":0,"disconnect":true}));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(
        s.apply(MOVE_SYMBOL, &json!({"symbol":symbol,"x_nm":0,"y_nm":p(0,16).y_nm,"disconnect":true,"expected_revision":s.revision()-1})).is_err()
    );
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(crate::command(MOVE_SYMBOL).unwrap().to_json()["params"]["properties"]["disconnect"]["default"], false);
    assert_eq!(crate::command(MOVE_WIRE_SEGMENT).unwrap().to_json()["params"]["properties"]["connect_ends"]["maxItems"], 2);
}

/// An exact wire-end drop cannot bypass ambiguous coincident pins just because one of those
/// pins already owns the target wire. All partial candidate merges are discarded on refusal.
#[test]
fn wire_drop_rejects_coincident_pins_atomically() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(0, 6));
    let wire = s.project().unwrap().wires[0].id();
    let ends: Vec<_> = s.project().unwrap().wires[0].endpoints.iter().map(|end| end.inspect()["junction"].clone()).collect();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,0).x_nm,"y_nm":0}));
    let pin = WireEndpoint::Terminal(s.project().unwrap().symbols[0].terminals()[0]);
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,0).x_nm,"y_nm":0}));
    run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":p(20,0),"bends":[]}));
    let before = s.snapshot().unwrap();
    let result = s.apply(MOVE_WIRE_SEGMENT, &json!({"wire":wire,"at":p(0,3),"to":p(10,3),"connect_ends":ends,"expected_revision":s.revision()}));
    assert!(result.unwrap_err().to_string().contains("coincident component pins"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Break detaches both ends of a straight pin-to-pin wire, preserving the symbols and
/// geometry. A deliberate return can join both original pins without a stationary wire there.
#[test]
fn break_detaches_both_pin_ends_and_can_rejoin_them() {
    let mut s = session();
    for x in [0, 20] {
        run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(x,0).x_nm,"y_nm":0}));
    }
    let pins = [
        WireEndpoint::Terminal(s.project().unwrap().symbols[0].terminals()[1]),
        WireEndpoint::Terminal(s.project().unwrap().symbols[1].terminals()[0]),
    ];
    run(&mut s, DRAW_WIRE, json!({"from":pins[0].inspect(),"to":pins[1].inspect(),"bends":[]}));
    let original = s.snapshot().unwrap();
    let wire = original.wires[0].id();
    run(&mut s, BREAK_WIRE, json!({"wire":wire,"at":p(10,0)}));
    let broken = s.snapshot().unwrap();
    assert_eq!(broken.wire_path(&broken.wires[0]), original.wire_path(&original.wires[0]));
    assert!(broken.wires[0].terminals().is_empty());
    assert!(broken.wire_moves_freely(&wire));
    let ends: Vec<_> = broken.wires[0].endpoints.iter().map(|end| end.inspect()["junction"].clone()).collect();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(10,0),"to":p(14,8),"connect_ends":ends}));
    assert!(s.project().unwrap().wires[0].terminals().is_empty());
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(14,8),"to":p(10,0),"connect_ends":ends}));
    assert_eq!(s.project().unwrap().wires[0].endpoints, pins);
    s.apply("edit.undo", &Value::Null).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&broken, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&original, &s.snapshot().unwrap()));
}

/// The selected straight wire separates from shared junctions at both ends without moving
/// or disconnecting any neighboring wire from its own junction.
#[test]
fn break_detaches_both_shared_junction_ends() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    let wire = s.project().unwrap().wires[0].id();
    let ends = s.project().unwrap().wires[0].endpoints;
    for (end, to) in [(ends[0], p(0, -6)), (ends[1], p(10, 6))] {
        run(&mut s, DRAW_WIRE, json!({"from":end.inspect(),"to":to,"bends":[]}));
    }
    let before = s.snapshot().unwrap();
    run(&mut s, BREAK_WIRE, json!({"wire":wire,"at":p(5,0)}));
    let after = s.snapshot().unwrap();
    assert!(after.wire_moves_freely(&wire));
    assert_ne!(after.wires[0].endpoints[0], ends[0]);
    assert_ne!(after.wires[0].endpoints[1], ends[1]);
    assert_eq!(after.wires[1..], before.wires[1..]);
    assert_eq!(after.connectivity().nets.len(), 3);
}

/// A single connected side is enough to offer Break. Its existing free end stays free and
/// keeps its identity, while the attached pin is released without changing the route.
#[test]
fn break_releases_one_connected_end_and_preserves_the_free_end() {
    for reverse in [false, true] {
        let mut s = session();
        run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
        let pin = WireEndpoint::Terminal(s.project().unwrap().symbols[0].terminals()[1]);
        let (from, to) = if reverse { (json!(p(12, 0)), pin.inspect()) } else { (pin.inspect(), json!(p(12, 0))) };
        run(&mut s, DRAW_WIRE, json!({"from":from,"to":to,"bends":[]}));
        let before = s.snapshot().unwrap();
        let wire = before.wires[0].id();
        let free_index = usize::from(!reverse);
        let free = before.wires[0].endpoints[free_index];
        assert!(before.wire_segment_can_break(&wire, p(8, 0)));
        run(&mut s, BREAK_WIRE, json!({"wire":wire,"at":p(8,0)}));
        let after = s.snapshot().unwrap();
        assert!(after.wires[0].terminals().is_empty());
        assert_eq!(after.wires[0].endpoints[free_index], free);
        assert_eq!(after.wire_path(&after.wires[0]), before.wire_path(&before.wires[0]));
        assert!(after.wire_moves_freely(&wire));
        assert!(!after.wire_segment_can_break(&wire, p(8, 0)));
        let saved = persistence::encode_documents(after.clone()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}
