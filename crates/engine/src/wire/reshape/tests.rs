use serde_json::json;

mod attachment;

use super::*;
use crate::{
    CONNECT_PINS, DRAW_WIRE, MOVE_SYMBOL, PLACE_SYMBOL,
    persistence::{self, Encoding},
    schematic::{GRID_NM, Terminal},
};

/// A grid point as a model point.
fn grid(x: i64, y: i64) -> Point {
    Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
}

/// A grid point as a command value.
fn at(x: i64, y: i64) -> Value {
    json!(grid(x, y))
}

/// A project with one free wire (0,0) → (10,0) → (10,10), and its wire ID.
fn free_wire() -> (Session, String) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Reshape"})).unwrap();
    session.apply(DRAW_WIRE, &json!({"from": at(0, 0), "to": at(10, 10), "bends": [at(10, 0)], "expected_revision": session.revision()})).unwrap();
    let id = session.project().unwrap().wires()[0].id();
    (session, id)
}

/// Every wire's drawn route, in wire order.
fn routes(session: &Session) -> Vec<Vec<Point>> {
    let project = session.project().unwrap();
    project.wires().iter().map(|wire| project.wire_path(wire)).collect()
}

/// Run a reshape command at the current revision.
fn run(session: &mut Session, command: &str, params: Value) -> Result<u64, CommandError> {
    let mut params = params;
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)
}

/// The project's identities and connections, which no reshape may change.
fn identities(session: &Session) -> (Vec<[WireEndpoint; 2]>, Value, Vec<u64>) {
    let project = session.project().unwrap();
    let ends = project.wires().iter().map(|wire| wire.endpoints()).collect();
    let nets = session.inspect()["project"]["schematic"]["netlist"].clone();
    (ends, nets, project.junctions().map(|(id, _)| id).collect())
}

/// A segment slides across its direction; fixed ends gain an orthogonal leg, the wire keeps
/// its endpoints, and the edit undoes and redoes as one step.
#[test]
fn sliding_a_segment_keeps_the_ends_and_connections() {
    let (mut session, wire) = free_wire();
    let before = session.snapshot().unwrap();
    let kept = identities(&session);
    let revision = session.revision();
    run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire, "at": at(10, 5), "to": at(14, 5)})).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(14, 0), grid(14, 10), grid(10, 10)]]);
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(identities(&session), kept);

    // The first leg slides down from its fixed start.
    run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire, "at": at(5, 0), "to": at(5, -3)})).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(0, -3), grid(14, -3), grid(14, 10), grid(10, 10)]]);
    session.apply("edit.undo", &Value::Null).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(14, 0), grid(14, 10), grid(10, 10)]]);

    // Sliding a leg back over its neighbour collapses the corners it no longer needs.
    run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire, "at": at(14, 5), "to": at(10, 5)})).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(10, 0), grid(10, 10)]]);
}

/// Moving a corner slides both legs that meet there, adding a leg at each fixed end.
#[test]
fn moving_a_corner_reshapes_its_two_legs() {
    let (mut session, wire) = free_wire();
    let kept = identities(&session);
    run(&mut session, crate::MOVE_WIRE_CORNER, json!({"wire": wire, "at": at(10, 0), "to": at(6, 4)})).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(0, 4), grid(6, 4), grid(6, 10), grid(10, 10)]]);
    assert_eq!(identities(&session), kept);
    // An interior corner with interior neighbours moves them along their own legs.
    run(&mut session, crate::MOVE_WIRE_CORNER, json!({"wire": wire, "at": at(6, 4), "to": at(3, 7)})).unwrap();
    assert_eq!(routes(&session), vec![vec![grid(0, 0), grid(0, 7), grid(3, 7), grid(3, 10), grid(10, 10)]]);
}

/// A free end moves with its end leg; a junction drags every attached route; the far ends and
/// all identities stay put.
#[test]
fn moving_a_junction_reshapes_every_attached_wire() {
    let (mut session, _) = free_wire();
    let end = session.project().unwrap().wires()[0].endpoints()[1];
    let WireEndpoint::Junction(junction) = end else { panic!("free wires end on junctions") };
    // Branch from the free end: (10,10) → (20,10).
    run(&mut session, DRAW_WIRE, json!({"from": end.inspect(), "to": at(20, 10), "bends": []})).unwrap();
    let kept = identities(&session);
    run(&mut session, crate::MOVE_JUNCTION, json!({"junction": format!("junction-{junction}"), "to": at(12, 13)})).unwrap();
    assert_eq!(
        routes(&session),
        vec![vec![grid(0, 0), grid(12, 0), grid(12, 13)], vec![grid(12, 13), grid(20, 13), grid(20, 10)]],
        "the vertical end leg follows across, the one-leg branch gains a corner at its far end"
    );
    assert_eq!(identities(&session), kept);
    assert_eq!(session.project().unwrap().endpoint_position(end), Some(grid(12, 13)));
}

/// A pin-to-pin route reshapes without changing its terminals or net, and its end legs still
/// follow a symbol moved afterwards. An automatic legacy route is saved explicitly once edited.
#[test]
fn pin_routes_keep_their_terminals() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Pins"})).unwrap();
    for x in [0, 20] {
        run(&mut session, PLACE_SYMBOL, json!({"definition": "resistor", "x_nm": x * GRID_NM, "y_nm": x / 2 * GRID_NM})).unwrap();
    }
    let symbols: Vec<_> = session.project().unwrap().symbols().iter().map(|s| s.id()).collect();
    run(&mut session, CONNECT_PINS, json!({"from": {"symbol": symbols[0], "terminal": "2"}, "to": {"symbol": symbols[1], "terminal": "1"}})).unwrap();
    let project = session.project().unwrap();
    let wire = project.wires()[0].clone();
    assert!(wire.bends().is_none(), "Connect Pins keeps an automatic route");
    let route = project.wire_path(&wire);
    assert_eq!(route.len(), 4, "a diagonal pin link has a middle vertical leg");
    let middle = Point { x_nm: route[1].x_nm, y_nm: (route[1].y_nm + route[2].y_nm) / 2 };
    let kept = identities(&session);

    // Moves to the grabbed point keep the automatic route: no authored bends, revision or undo step.
    let (snapshot, revision) = (session.snapshot().unwrap(), session.revision());
    run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire.id(), "at": middle, "to": middle})).unwrap();
    run(&mut session, crate::MOVE_WIRE_CORNER, json!({"wire": wire.id(), "at": route[1], "to": route[1]})).unwrap();
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert!(session.project().unwrap().wires()[0].bends().is_none());
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(session.project().unwrap().wires().is_empty(), "the only undo step is still Connect Pins");
    session.apply("edit.redo", &Value::Null).unwrap();

    let to = Point { x_nm: middle.x_nm + 2 * GRID_NM, y_nm: middle.y_nm };
    run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire.id(), "at": middle, "to": to})).unwrap();
    assert_eq!(identities(&session), kept);
    let project = session.project().unwrap();
    let moved = project.wire_path(&project.wires()[0]);
    assert_eq!((moved[1].x_nm, moved[2].x_nm), (to.x_nm, to.x_nm));
    assert!(project.wires()[0].bends().is_some(), "the edited route is authored");
    let terminals: Vec<Terminal> = project.wires()[0].terminals();
    assert_eq!(terminals.len(), 2);

    // Save and reopen keep the authored route.
    let snapshot = session.snapshot().unwrap();
    for encoding in [Encoding::Json, Encoding::Zip] {
        let bytes = persistence::encode_file(snapshot.clone(), encoding).unwrap();
        let (loaded, _) = persistence::decode_file(&bytes).unwrap();
        assert_eq!(loaded.wire_path(&loaded.wires()[0]), moved);
    }

    // Moving a connected symbol afterwards still keeps the pins attached.
    let position = project.symbols()[1].position;
    run(&mut session, MOVE_SYMBOL, json!({"symbol": symbols[1], "x_nm": position.x_nm + 4 * GRID_NM, "y_nm": position.y_nm})).unwrap();
    assert_eq!(identities(&session), kept);
}

/// Points off the route, the wrong kind of point, moves along a segment, missing identities,
/// out-of-bounds targets and stale revisions are refused, changing nothing. A move to the
/// grabbed point changes nothing and adds no undo step.
#[test]
fn invalid_reshapes_change_nothing() {
    let (mut session, wire) = free_wire();
    let before = session.inspect();
    let segment = crate::MOVE_WIRE_SEGMENT;
    let corner = crate::MOVE_WIRE_CORNER;
    let cases = [
        (segment, json!({"wire": wire, "at": at(5, 5), "to": at(5, 7)}), "not on the wire"),
        (segment, json!({"wire": wire, "at": at(10, 0), "to": at(10, 3)}), "corner"),
        (segment, json!({"wire": wire, "at": at(10, 5), "to": at(12, 6)}), "across its direction"),
        (corner, json!({"wire": wire, "at": at(5, 0), "to": at(5, 3)}), "not a corner"),
        (corner, json!({"wire": wire, "at": at(0, 0), "to": at(1, 1)}), "an end of the wire"),
        (corner, json!({"wire": "wire-999", "at": at(10, 0), "to": at(1, 1)}), "does not exist"),
        (crate::MOVE_JUNCTION, json!({"junction": "junction-999", "to": at(1, 1)}), "does not exist"),
    ];
    for (command, params, reason) in cases {
        let error = run(&mut session, command, params).unwrap_err().to_string();
        assert!(error.contains(reason), "{command}: {error}");
        assert_eq!(session.inspect(), before);
    }
    let limit = json!({"x_nm": crate::schematic::MAX_CENTER_NM + 1, "y_nm": 0});
    assert!(run(&mut session, corner, json!({"wire": wire, "at": at(10, 0), "to": limit})).is_err());
    let edge = json!({"x_nm": crate::schematic::MAX_CENTER_NM, "y_nm": 0});
    run(&mut session, corner, json!({"wire": wire, "at": at(10, 0), "to": edge})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let stale = json!({"wire": wire, "at": at(10, 5), "to": at(14, 5), "expected_revision": session.revision() - 1});
    assert!(session.apply(segment, &stale).is_err());
    let revision = session.revision();
    let undo = session.available("edit.undo");
    run(&mut session, segment, json!({"wire": wire, "at": at(10, 5), "to": at(10, 5)})).unwrap();
    assert_eq!((session.revision(), session.available("edit.undo")), (revision, undo));
}

/// A reshape that would need more than the bend limit is refused as a whole.
#[test]
fn reshapes_stay_within_the_bend_limit() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Stairs"})).unwrap();
    // A staircase with exactly MAX_BENDS bends: right, down, right, down, …
    let bends: Vec<_> = (0..MAX_BENDS as i64).map(|k| at((k + 2) / 2, (k + 1) / 2)).collect();
    let last = MAX_BENDS as i64;
    let to = at((last + 2) / 2, (last + 1) / 2);
    run(&mut session, DRAW_WIRE, json!({"from": at(0, 0), "to": to, "bends": bends})).unwrap();
    let wire = session.project().unwrap().wires()[0].id();
    let route = routes(&session)[0].clone();
    assert_eq!(route.len(), MAX_BENDS + 2);
    // Sliding the first leg away from its fixed start adds a leg, one too many.
    let first = Point { x_nm: GRID_NM / 2, y_nm: 0 };
    let error = run(&mut session, crate::MOVE_WIRE_SEGMENT, json!({"wire": wire, "at": first, "to": {"x_nm": GRID_NM / 2, "y_nm": -GRID_NM}}));
    assert!(matches!(error, Err(CommandError::Unavailable(_))), "{error:?}");
    assert_eq!(routes(&session)[0], route);
}
