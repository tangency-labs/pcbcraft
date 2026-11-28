use serde_json::{Value, json};
use std::sync::Arc;

use crate::{
    CommandError, DRAW_WIRE, MOVE_WIRE_SEGMENT, PLACE_SYMBOL, Project, Session, net_labels,
    persistence::{self, Encoding},
    schematic::{GRID_NM as G, Point, Terminal, WireEndpoint},
};

const JOIN: &str = "schematic.join_wires";

/// A model point on the schematic grid.
fn p(x: i64, y: i64) -> Point {
    Point { x_nm: x * G, y_nm: y * G }
}

/// Run a command at the current revision and return its reply.
fn run(s: &mut Session, id: &str, mut params: Value) -> Result<Value, CommandError> {
    params["expected_revision"] = json!(s.revision());
    s.execute(id, &params)
}

/// Draw a free wire through `points` and return its ID.
fn wire(s: &mut Session, points: &[Point]) -> String {
    let bends = &points[1..points.len() - 1];
    run(s, DRAW_WIRE, json!({"from":points[0],"to":points[points.len() - 1],"bends":bends,"auto_heal":false})).unwrap();
    s.project().unwrap().wires().last().unwrap().id()
}

/// Place a resistor turned a quarter turn, so its pins lie on one vertical line, and return the
/// pin nearest `toward`.
fn resistor(s: &mut Session, at: Point, toward: Point) -> Terminal {
    run(s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":90})).unwrap();
    let project = s.project().unwrap();
    let symbol = project.symbols().last().unwrap();
    let pins = symbol.terminals().into_iter().zip(symbol.pins());
    pins.min_by_key(|(_, pin)| (pin.x_nm - toward.x_nm).abs() + (pin.y_nm - toward.y_nm).abs()).unwrap().0
}

/// Draw a vertical wire from a terminal straight down past `y` grid rows and return its ID.
fn from_pin(s: &mut Session, pin: Terminal, to_y: i64) -> String {
    let start = s.project().unwrap().terminal_position(pin).unwrap();
    run(s, DRAW_WIRE, json!({"from":pin.inspect(),"to":{"x_nm":start.x_nm,"y_nm":to_y * G},"bends":[],"auto_heal":false})).unwrap();
    s.project().unwrap().wires().last().unwrap().id()
}

/// The issue's sheet: horizontal wire `a` is crossed by `b` and, close by, by `c`. `b` and `c`
/// come from resistor pins, so their terminals carry identities the join must keep.
struct Crossings {
    s: Session,
    a: String,
    b: String,
    c: String,
    pins: [Terminal; 2],
}

/// Build [`Crossings`] with commands only. Nothing is connected yet.
fn crossings() -> Crossings {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"demo-join"})).unwrap();
    let a = wire(&mut s, &[p(-10, 0), p(20, 0)]);
    let pin_b = resistor(&mut s, p(0, -10), p(0, 0));
    let b = from_pin(&mut s, pin_b, 8);
    let pin_c = resistor(&mut s, p(6, -10), p(6, 0));
    let c = from_pin(&mut s, pin_c, 8);
    Crossings { s, a, b, c, pins: [pin_b, pin_c] }
}

/// The drawn routes of every wire, by ID.
fn routes(project: &Project) -> Vec<(String, Vec<Point>)> {
    project.wires().iter().map(|wire| (wire.id(), project.wire_path(wire))).collect()
}

/// Every terminal a wire ends on, in wire order.
fn terminals(project: &Project) -> Vec<Terminal> {
    let mut found: Vec<_> = project.wires().iter().flat_map(|wire| wire.terminals()).collect();
    found.sort();
    found
}

/// The x coordinate of a vertical wire.
fn x_of(project: &Project, id: &str) -> i64 {
    let wire = project.wires().iter().find(|wire| wire.id() == id).unwrap();
    project.wire_path(wire)[0].x_nm
}

/// Previewing and applying a join at the chosen crossing connects exactly those two wires at that
/// point. The routes keep their geometry, the nearby crossing stays separate, connectivity
/// inspection lists the junction, and undo/redo and save/reopen keep each state.
#[test]
fn joins_the_chosen_crossing_and_keeps_everything_else() {
    let Crossings { mut s, a, b, c, pins } = crossings();
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let (x_b, x_c) = (x_of(&before, &b), x_of(&before, &c));
    let at = Point { x_nm: x_b, y_nm: 0 };
    assert_ne!(before.connectivity().net_index_for(pins[0]), before.connectivity().net_index_for(pins[1]));

    let history = s.inspect()["can_redo"].clone();
    let preview = run(&mut s, JOIN, json!({"wire":a,"other":b,"dry_run":true})).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()), "a preview edits nothing");
    assert_eq!(s.revision(), revision);
    assert_eq!(s.inspect()["can_redo"], history);
    assert_eq!(preview["at"], json!(at));
    assert_eq!(preview["wires"], json!([a, b]));
    assert_eq!(preview["already_connected"], false);
    assert_eq!(preview["nets"][1]["terminals"], 1);
    assert_eq!(preview["revision"], revision);
    assert!(preview.get("junction").is_none());

    let applied = run(&mut s, JOIN, json!({"wire":a,"other":b})).unwrap();
    let after = s.snapshot().unwrap();
    assert!(s.revision() > revision);
    assert_eq!(applied["revision"], s.revision());
    for key in ["at", "wires", "nets", "already_connected"] {
        assert_eq!(applied[key], preview[key], "the preview describes the applied join: {key}");
    }
    let junction = WireEndpoint::parse(&applied["junction"]).unwrap();
    assert_eq!(after.endpoint_position(junction), Some(at));
    assert_eq!(after.wires().iter().filter(|wire| wire.endpoints().contains(&junction)).count(), 4, "four ends meet, so the dot shows");

    let graph = after.connectivity();
    assert_eq!(graph.net_index_for(pins[0]), graph.net_index_for_endpoint(after.wires()[0].endpoints()[0]));
    assert_eq!(graph.net_index_for_endpoint(junction), graph.net_index_for(pins[0]));
    assert_ne!(graph.net_index_for(pins[1]), graph.net_index_for(pins[0]), "the nearby crossing stays disconnected");
    let netlist = &s.inspect()["project"]["schematic"]["netlist"]["nets"];
    let junction_id = applied["junction"]["junction"].as_str().unwrap();
    let net = netlist.as_array().unwrap().iter().find(|net| net["terminals"].as_array().unwrap().contains(&pins[0].inspect())).unwrap();
    assert!(net["junctions"].as_array().unwrap().contains(&json!(junction_id)), "connectivity inspection shows the junction");

    // Geometry and terminal identities: the chosen wires keep their IDs for their first piece,
    // their pieces retrace the old routes, and the other wire is untouched.
    assert_eq!(terminals(&after), terminals(&before));
    assert_eq!(after.symbols(), before.symbols());
    let old: Vec<_> = routes(&before);
    let new: Vec<_> = routes(&after);
    for (id, route) in &old {
        let pieces: Vec<_> = new.iter().filter(|(other, _)| other == id || !old.iter().any(|(known, _)| known == other)).collect();
        let first = pieces.iter().find(|(other, _)| other == id).unwrap();
        if *id == c {
            assert_eq!(&first.1, route);
            continue;
        }
        let rest = pieces.iter().find(|(other, piece)| other != id && piece.first() == first.1.last() && piece.last() == route.last()).unwrap();
        let joined: Vec<_> = first.1.iter().chain(&rest.1[1..]).copied().collect();
        assert!(joined.contains(&at) && !route.contains(&at));
        let retraced: Vec<_> = joined.into_iter().filter(|point| *point != at).collect();
        assert_eq!(&retraced, route, "{id} keeps its drawn route, plus the junction point on its leg");
    }
    assert_eq!(new.len(), old.len() + 2);

    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));

    for (state, joined) in [(&before, false), (&after, true)] {
        for encoding in [Encoding::Json, Encoding::Zip] {
            let bytes = persistence::diagnostic_test_file(state.clone(), encoding).unwrap();
            let (loaded, _) = persistence::decode_diagnostic_file(&bytes).unwrap();
            assert_eq!(persistence::diagnostic_test_file(loaded.clone(), encoding).unwrap(), bytes);
            assert_eq!(routes(&loaded), routes(state));
            let graph = loaded.connectivity();
            assert_eq!(graph.net_index_for(pins[0]) == graph.net_index_for_endpoint(loaded.wires()[0].endpoints()[0]), joined);
            assert_ne!(graph.net_index_for(pins[0]), graph.net_index_for(pins[1]));
        }
    }
    assert_ne!(x_b, x_c);
}

/// Every refusal comes before an edit: the project, revision and history stay as they were.
#[test]
fn refusals_leave_the_project_unchanged() {
    let Crossings { mut s, a, b, c, .. } = crossings();
    // A wire that crosses `a` twice and one that ends on it.
    let twice = wire(&mut s, &[p(10, -4), p(10, 4), p(14, 4), p(14, -4)]);
    let ends = wire(&mut s, &[p(17, -4), p(17, 0)]);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let x_b = x_of(&before, &b);
    let refused = |s: &mut Session, params: Value| {
        let error = run(s, JOIN, params).unwrap_err();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
        error.to_string()
    };
    assert!(refused(&mut s, json!({"wire":b,"other":c})).contains("do not cross"));
    assert!(refused(&mut s, json!({"wire":a,"other":a})).contains("two different wires"));
    assert!(refused(&mut s, json!({"wire":a,"other":"wire-999"})).contains("does not exist"));
    assert!(refused(&mut s, json!({"wire":a,"other":twice})).contains("cross at 2 points"));
    assert!(refused(&mut s, json!({"wire":a,"other":ends})).contains("not at an ordinary crossing"));
    assert!(refused(&mut s, json!({"wire":a,"other":b,"at":p(3, 0)})).contains("do not cross at that point"));
    let stale = s.execute(JOIN, &json!({"wire":a,"other":twice,"at":p(10, 0),"dry_run":true,"expected_revision":revision - 1}));
    assert!(matches!(stale, Err(CommandError::StaleRevision { .. })));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));

    // With `at`, a wire that crosses twice joins only at the chosen point.
    let applied = run(&mut s, JOIN, json!({"wire":a,"other":twice,"at":p(14, 0)})).unwrap();
    assert_eq!(applied["at"], json!(p(14, 0)));
    let graph = s.project().unwrap().connectivity();
    let joined = |id: &str| {
        s.project()
            .unwrap()
            .wires()
            .iter()
            .filter(|wire| wire.endpoints().contains(&WireEndpoint::parse(&applied["junction"]).unwrap()))
            .any(|wire| wire.id() == id)
    };
    assert!(joined(&a) && joined(&twice));
    assert_eq!(graph.nets.len(), before.connectivity().nets.len() - 1);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());

    // A third wire ending at the a/b crossing would look joined under the dot: ambiguous.
    wire(&mut s, &[Point { x_nm: x_b, y_nm: 0 }, Point { x_nm: x_b + 2 * G, y_nm: 0 }]);
    let busy = s.snapshot().unwrap();
    let error = run(&mut s, JOIN, json!({"wire":a,"other":b})).unwrap_err().to_string();
    assert!(error.contains("another wire or pin"), "{error}");
    assert!(Arc::ptr_eq(&busy, &s.snapshot().unwrap()));
}

/// A stale preview is refused on Apply, and a cancelled preview (one never applied) leaves no trace.
#[test]
fn a_stale_preview_is_refused_without_an_edit() {
    let Crossings { mut s, a, b, .. } = crossings();
    let preview = run(&mut s, JOIN, json!({"wire":a,"other":b,"dry_run":true})).unwrap();
    let seen = preview["revision"].as_u64().unwrap();
    wire(&mut s, &[p(-10, 12), p(10, 12)]);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let error = s.execute(JOIN, &json!({"wire":a,"other":b,"expected_revision":seen})).unwrap_err();
    assert!(matches!(error, CommandError::StaleRevision { .. }));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
}

/// Differently named nets are not joined; a shared name or an unnamed side is fine.
#[test]
fn differently_named_nets_are_refused() {
    let Crossings { mut s, a, b, .. } = crossings();
    let label = |s: &mut Session, name: &str, at: Point| {
        run(s, net_labels::ADD, json!({"name":name,"style":"local","x_nm":at.x_nm,"y_nm":at.y_nm})).unwrap();
    };
    let x_b = x_of(&s.snapshot().unwrap(), &b);
    label(&mut s, "demo-b", Point { x_nm: x_b, y_nm: 8 * G });
    let unnamed = run(&mut s, JOIN, json!({"wire":a,"other":b,"dry_run":true})).unwrap();
    assert_eq!(unnamed["nets"][1]["names"], json!(["demo-b"]));
    label(&mut s, "demo-a", p(-10, 0));
    let before = s.snapshot().unwrap();
    let error = run(&mut s, JOIN, json!({"wire":a,"other":b,"dry_run":true})).unwrap_err().to_string();
    assert!(error.contains("demo-a") && error.contains("demo-b") && error.contains("two names"), "{error}");
    assert!(run(&mut s, JOIN, json!({"wire":a,"other":b})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Joining never changes how ordinary drawing and moving treat crossings: a new wire drawn
/// across the joined wires, and a wire moved across them, stay separate.
#[test]
fn drawn_and_moved_crossings_stay_disconnected_after_a_join() {
    let Crossings { mut s, a, b, c, pins } = crossings();
    run(&mut s, JOIN, json!({"wire":a,"other":b})).unwrap();
    let nets = s.project().unwrap().connectivity().nets.len();
    wire(&mut s, &[p(-4, -6), p(-4, 6)]);
    assert_eq!(s.project().unwrap().connectivity().nets.len(), nets + 1, "a drawn crossing adds its own net");
    let x_c = x_of(&s.snapshot().unwrap(), &c);
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":c,"at":{"x_nm":x_c,"y_nm":4 * G},"to":{"x_nm":x_c + 2 * G,"y_nm":4 * G},"auto_heal":false}))
        .unwrap();
    let graph = s.project().unwrap().connectivity();
    assert_ne!(graph.net_index_for(pins[1]), graph.net_index_for(pins[0]), "a moved crossing stays separate");
}
