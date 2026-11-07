use super::*;

/// An existing upper-right L, with a free lead at the left and a component pin below it.
fn upper_right_route() -> (Session, String) {
    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,12).x_nm,"y_nm":p(14,12).y_nm}));
    let pin = s.project().unwrap().symbols[0].terminals()[0];
    run(&mut s, DRAW_WIRE, json!({"from":p(-10,0),"to":WireEndpoint::Terminal(pin).inspect(),"bends":[p(10,0)]}));
    let wire = s.project().unwrap().wires[0].id();
    (s, wire)
}

/// Finishing the photographed lower-left L onto the same net must keep the new gesture.
/// Equal Manhattan length cannot make it disappear in favor of the older upper-right L.
#[test]
fn an_equal_length_redraw_replaces_the_old_alternative() {
    let (mut s, wire) = upper_right_route();
    let before = s.snapshot().unwrap();
    let pin = before.symbols[0].terminals()[0];
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(-4,0)},"to":{"wire":wire,"at":p(10,10)},"bends":[p(-4,10)]}));
    let project = s.project().unwrap();
    assert!(project.wires.iter().any(|wire| project.wire_path(wire) == vec![p(-4, 0), p(-4, 10), p(10, 10)]), "keep the route the user just drew");
    assert!(
        !project.wires.iter().any(|wire| project.wire_path(wire).contains(&p(10, 0))),
        "remove the branch-free old corner, rather than the new route"
    );
    assert_eq!(project.terminal_position(pin), Some(p(10, 12)));
    assert_eq!(project.connectivity().nets.len(), 1);
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// A deliberate detour between the same endpoints replaces an older straight route even
/// when longer. Splitting the new path to satisfy native edge uniqueness must retain intent.
#[test]
fn a_longer_redraw_keeps_the_complete_new_path_and_both_endpoints() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    let ends = s.project().unwrap().wires[0].endpoints;
    run(&mut s, DRAW_WIRE, json!({"from":ends[0].inspect(),"to":ends[1].inspect(),"bends":[p(0,10),p(20,10)]}));
    let project = s.project().unwrap();
    assert!(project.wires.iter().any(|wire| wire.endpoints.contains(&ends[0])));
    assert!(project.wires.iter().any(|wire| wire.endpoints.contains(&ends[1])));
    assert!(project.wires.iter().any(|wire| project.wire_path(wire).windows(2).any(|leg| leg == [p(0, 10), p(20, 10)])));
    assert!(!project.wires.iter().any(|wire| project.wire_path(wire) == vec![p(0, 0), p(20, 0)]));
    assert_eq!(project.connectivity().nets.len(), 1);
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
}

/// An intermediate branch prevents deleting the old alternative; the new gesture remains
/// present as a loop, with the branch and terminal still connected at their original positions.
#[test]
fn a_redraw_does_not_remove_an_intermediate_branch() {
    let (mut s, wire) = upper_right_route();
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(10,0)},"to":p(10,-6),"bends":[]}));
    let project = s.project().unwrap();
    let right = project.wires.iter().find(|wire| project.wire_path(wire).windows(2).any(|leg| on_segment(p(10, 10), leg[0], leg[1]))).unwrap().id();
    let branch = project.junctions().find(|(_, point)| *point == p(10, -6)).unwrap().0;
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(-4,0)},"to":{"wire":right,"at":p(10,10)},"bends":[p(-4,10)]}));
    let project = s.project().unwrap();
    assert!(project.wires.iter().any(|wire| project.wire_path(wire) == vec![p(-4, 0), p(-4, 10), p(10, 10)]));
    assert!(project.wires.iter().any(|wire| project.wire_path(wire).contains(&p(10, 0))));
    assert_eq!(project.endpoint_position(WireEndpoint::Junction(branch)), Some(p(10, -6)));
    assert_eq!(project.connectivity().nets.len(), 1);
}

/// Replacing the old alternative must not connect a separate wire crossed by the new path.
#[test]
fn a_redraw_keeps_unrelated_crossings_disconnected() {
    let (mut s, wire) = upper_right_route();
    draw(&mut s, p(-8, 8), p(0, 8));
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(-4,0)},"to":{"wire":wire,"at":p(10,10)},"bends":[p(-4,10)]}));
    let project = s.project().unwrap();
    assert!(project.wires.iter().any(|wire| project.wire_path(wire) == vec![p(-8, 8), p(0, 8)]));
    assert_eq!(project.connectivity().nets.len(), 2);
}
