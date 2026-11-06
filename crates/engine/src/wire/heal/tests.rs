use super::*;
use crate::{ATTACH_JUNCTION, DRAW_WIRE, HEAL_WIRES, MOVE_ITEMS, MOVE_SYMBOL, PLACE_SYMBOL, Session, persistence};
use serde_json::{Value, json};

fn p(x: i64, y: i64) -> Point {
    Point { x_nm: x * 1_270_000, y_nm: y * 1_270_000 }
}
fn run(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Healing"})).unwrap();
    s
}
fn draw(s: &mut Session, from: Point, to: Point) {
    run(s, DRAW_WIRE, json!({"from":from,"to":to,"bends":[]}));
}

/// A moved part at the exact dangling-wire coordinate replaces the free identity with its pin;
/// a new two-pin part spanning a wire is inserted without shorting its two terminals.
#[test]
fn part_drops_connect_endpoints_and_insert_into_straight_wires() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(20,10).x_nm,"y_nm":p(20,10).y_nm}));
    let symbol = s.project().unwrap().symbols()[0].clone();
    let before = s.snapshot().unwrap();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol.id(),"x_nm":p(14,0).x_nm,"y_nm":0}));
    assert_eq!(s.project().unwrap().wires()[0].endpoints[1], WireEndpoint::Terminal(symbol.terminals()[0]));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));

    let mut s = session();
    draw(&mut s, p(0, 0), p(30, 0));
    let before = s.snapshot().unwrap();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(15,0).x_nm,"y_nm":0}));
    let project = s.project().unwrap();
    let pins = project.symbols()[0].terminals();
    assert_eq!(project.wires.len(), 2);
    assert_eq!(project.connectivity().nets.len(), 2);
    assert_eq!(project.wire_path(&project.wires[0]), vec![p(0, 0), p(11, 0)]);
    assert_eq!(project.wire_path(&project.wires[1]), vec![p(19, 0), p(30, 0)]);
    assert_ne!(project.connectivity().net_for(pins[0]), project.connectivity().net_for(pins[1]));
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().wires.len(), 2);
}

/// Whole-route moves attach their free ends to pins just as handle drops do, through the same
/// engine path used by GUI group previews and MCP execution.
#[test]
fn whole_wire_drop_onto_pin_connects() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,8).x_nm,"y_nm":p(14,8).y_nm}));
    let wire = s.project().unwrap().wires[0].id();
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    run(&mut s, MOVE_ITEMS, json!({"items":{"symbols":[],"wires":[wire]},"dx_nm":0,"dy_nm":p(0,8).y_nm}));
    assert_eq!(s.project().unwrap().wires[0].endpoints[1], WireEndpoint::Terminal(pin));
}

/// Explicit segment targets split at the branch; an ordinary perpendicular crossing adds no
/// connection. Preview, commit, undo and serialization retain the same graph.
#[test]
fn drawing_or_dropping_into_a_segment_creates_a_branch() {
    for attach in [false, true] {
        let mut s = session();
        draw(&mut s, p(0, 0), p(20, 0));
        let target = s.project().unwrap().wires[0].id();
        if attach {
            draw(&mut s, p(10, -10), p(10, -5));
        }
        let before = s.snapshot().unwrap();
        if attach {
            let source = before.wires[1].endpoints[1].inspect();
            run(&mut s, ATTACH_JUNCTION, json!({"junction":source["junction"],"to":{"wire":target,"at":p(10,0)}}));
        } else {
            run(&mut s, DRAW_WIRE, json!({"from":p(10,-10),"to":{"wire":target,"at":p(10,0)},"bends":[]}));
        }
        let project = s.project().unwrap();
        assert_eq!(project.wires.len(), 3);
        assert_eq!(project.connectivity().nets.len(), 1);
        let node = project.junctions().find(|(_, point)| *point == p(10, 0)).unwrap().0;
        assert_eq!(project.wires.iter().filter(|w| w.endpoints.contains(&WireEndpoint::Junction(node))).count(), 3);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    draw(&mut s, p(10, -10), p(10, 10));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), 2);
}

/// Partial positive overlaps become one run with no duplicate strokes or redundant intermediate
/// junctions. Repeated healing is a byte-stable no-op and adds no history entry.
#[test]
fn overlaps_and_doubled_back_legs_heal_locally() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    let before = s.snapshot().unwrap();
    draw(&mut s, p(10, 0), p(30, 0));
    let project = s.project().unwrap();
    assert_eq!(project.wires.len(), 1);
    assert_eq!(project.junctions().count(), 2);
    assert_eq!(project.wire_path(&project.wires[0]), vec![p(0, 0), p(30, 0)]);
    let wire = project.wires[0].id();
    let revision = s.revision();
    run(&mut s, HEAL_WIRES, json!({"items":{"symbols":[],"wires":[wire]}}));
    assert_eq!(s.revision(), revision);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let mut s = session();
    run(&mut s, DRAW_WIRE, json!({"from":p(0,0),"to":p(5,5),"bends":[p(10,0),p(5,0)]}));
    assert_eq!(s.project().unwrap().wire_path(&s.project().unwrap().wires[0]), vec![p(0, 0), p(5, 0), p(5, 5)]);
}

/// Healing is bounded and atomic, and it never silently rewrites a project merely on reopening.
#[test]
fn healing_rejects_stale_or_ambiguous_requests_without_changes() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    draw(&mut s, p(10, -10), p(10, 10));
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    assert!(s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":p(14,0).x_nm,"y_nm":0,"expected_revision":revision})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(s.apply(HEAL_WIRES, &json!({"items":{"symbols":[],"wires":[before.wires[0].id()]},"expected_revision":revision-1})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let mut edit = Edit::new(&before, revision);
    edit.budget = 0;
    edit.touched.insert(before.wires[0].ordinal);
    assert!(edit.heal().is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}
