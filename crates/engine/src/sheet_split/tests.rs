use super::*;
use crate::{
    CONNECT_PINS, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, error_codes::ErrorCode, library::sources::symbol_digest, net_labels,
    persistence::encode_documents, schematic::GRID_NM,
};
use std::sync::Arc;

/// Submit an authored change through the same revision-checked path as the GUI and MCP.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap_or_else(|error| panic!("{command}: {error}"));
}

/// Place a resistor and return its symbol ID.
fn resistor(s: &mut Session, x: i64, y: i64) -> String {
    apply(s, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":y}));
    s.project().unwrap().symbols().last().unwrap().id()
}

/// Place the bundled ground symbol with the value that names its global net.
fn ground(s: &mut Session, x: i64, y: i64) -> String {
    let d = s.libraries().latest_symbol("power", "gnd").unwrap();
    let digest = symbol_digest(&d).unwrap();
    apply(
        s,
        PLACE_SYMBOL,
        json!({"library":"power","definition":"gnd","definition_revision":d.revision(),"definition_digest":digest,"value":"GND","x_nm":x,"y_nm":y}),
    );
    s.project().unwrap().symbols().last().unwrap().id()
}

/// A terminal reference as commands take it.
fn pin(symbol: &str, terminal: &str) -> Value {
    json!({"symbol":symbol,"terminal":terminal})
}

/// Join two terminals with a wire.
fn connect(s: &mut Session, from: Value, to: Value) {
    apply(s, CONNECT_PINS, json!({"from":from,"to":to}));
}

/// Six resistors on one sheet: R1 to R3 are selected, R4 to R6 stay. Three nets cross the
/// boundary (one of them named by a local label outside), R1 and R2 share a net inside,
/// R4 and R5 share a net outside, and a selected and an unselected ground symbol join GND.
struct Fixture {
    session: Session,
    inside: [String; 3],
    outside: [String; 3],
    grounds: [String; 2],
}

/// Build the six-resistor sheet with its wiring, ground symbols and the SDA label.
fn fixture() -> Fixture {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let row = 10 * GRID_NM;
    let far = 40 * GRID_NM;
    let inside = [resistor(&mut s, 0, 0), resistor(&mut s, 0, row), resistor(&mut s, 0, 2 * row)];
    let outside = [resistor(&mut s, far, 0), resistor(&mut s, far, row), resistor(&mut s, far, 2 * row)];
    let grounds = [ground(&mut s, -10 * GRID_NM, 4 * row), ground(&mut s, far + 10 * GRID_NM, 4 * row)];
    for (a, b) in inside.iter().zip(&outside) {
        connect(&mut s, pin(a, "2"), pin(b, "1"));
    }
    connect(&mut s, pin(&inside[0], "1"), pin(&inside[1], "1"));
    connect(&mut s, pin(&outside[0], "2"), pin(&outside[1], "2"));
    connect(&mut s, pin(&inside[2], "1"), pin(&grounds[0], "1"));
    connect(&mut s, pin(&outside[2], "2"), pin(&grounds[1], "1"));
    let project = s.project().unwrap();
    let label_at = project.terminal_position(crate::schematic::wiring::Terminal::parse(&pin(&outside[0], "1")).unwrap()).unwrap();
    apply(&mut s, net_labels::ADD, json!({"name":"SDA","style":"local","x_nm":label_at.x_nm,"y_nm":label_at.y_nm}));
    Fixture { session: s, inside, outside, grounds }
}

/// The selection on the fixture: three resistors and the ground symbol wired to them.
fn selection(f: &Fixture) -> Value {
    json!({"symbols":[f.inside[0], f.inside[1], f.inside[2], f.grounds[0]],"wires":[]})
}

/// The preview lists the selected parts, exactly the three boundary nets as proposed ports
/// and ground as needing none, through both dispatch paths, and changes nothing: the
/// revision, history, snapshot and encoded project documents are untouched.
#[test]
fn preview_lists_moved_parts_and_boundary_ports_without_changing_the_project() {
    let mut f = fixture();
    let items = selection(&f);
    let s = &mut f.session;
    let project = s.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let before = s.inspect();
    let revision = s.revision();
    let params = json!({"items":items,"name":"Filter","expected_revision":revision});

    let report = s.execute(PREVIEW, &params).unwrap();
    assert_eq!(report["revision"], json!(revision));
    let preview = &report["split_preview"];
    assert_eq!(preview["name"], "Filter");
    assert_eq!(preview["sheet"], json!(project.sheet_target().id()));
    let moved: Vec<_> = preview["symbols"].as_array().unwrap().iter().map(|row| row["symbol"].as_str().unwrap().to_owned()).collect();
    assert_eq!(moved, [f.inside[0].clone(), f.inside[1].clone(), f.inside[2].clone(), f.grounds[0].clone()]);
    assert_eq!(preview["part_instances"].as_array().unwrap().len(), 4);
    assert_eq!(preview["split_part_instances"], json!([]));

    let ports = preview["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 3, "{preview}");
    let named: Vec<_> = ports.iter().filter(|port| port["generated"] == false).collect();
    assert_eq!(named.len(), 1);
    assert_eq!(named[0]["name"], "SDA");
    assert_eq!(named[0]["inside"], json!([pin(&f.inside[0], "2")]));
    for port in ports.iter().filter(|port| port["generated"] == true) {
        assert!(port["name"].as_str().unwrap().starts_with("Net-"), "{port}");
    }
    assert!(ports.iter().all(|port| port["direction"] == "passive"));
    let crossing: Vec<_> = ports.iter().flat_map(|port| port["outside"].as_array().unwrap().iter().cloned()).collect();
    for b in &f.outside {
        assert!(crossing.contains(&pin(b, "1")), "{b}");
    }

    let globals = preview["global_nets"].as_array().unwrap();
    assert_eq!(globals.len(), 1);
    assert_eq!(globals[0]["name"], "GND");
    assert!(globals[0]["inside"].as_array().unwrap().contains(&pin(&f.grounds[0], "1")));
    assert!(globals[0]["outside"].as_array().unwrap().contains(&pin(&f.grounds[1], "1")));

    // The inner R1-R2 wire and the wire to the selected ground travel; the three crossing
    // wires are cut at the boundary.
    assert_eq!(preview["wires"].as_array().unwrap().len(), 2);
    assert_eq!(preview["cut_wires"].as_array().unwrap().len(), 3);

    assert_eq!(s.apply(PREVIEW, &params).unwrap(), revision);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.inspect(), before);
    assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
}

/// A ground symbol left outside the selection does not carry the name inside, so the net
/// it joins needs a port like any other.
#[test]
fn a_ground_left_outside_makes_its_net_a_port() {
    let mut f = fixture();
    let items = json!({"symbols":[f.inside[0], f.inside[1], f.inside[2]],"wires":[]});
    let report = f.session.execute(PREVIEW, &json!({"items":items,"name":"Filter"})).unwrap();
    let ports = report["split_preview"]["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 4);
    let ground = ports.iter().find(|port| port["name"] == "GND").unwrap();
    assert_eq!(ground["generated"], false);
    assert_eq!(ground["inside"], json!([pin(&f.inside[2], "1")]));
    assert_eq!(report["split_preview"]["global_nets"], json!([]));
}

/// Refusals name the problem and leave the exact snapshot: an empty, unknown or malformed
/// selection, a blank or taken sheet name, and a stale revision.
#[test]
fn invalid_requests_are_refused_without_changing_the_project() {
    let mut f = fixture();
    let items = selection(&f);
    let s = &mut f.session;
    let project = s.snapshot().unwrap();
    let revision = s.revision();
    let taken = project.sheets().next().unwrap().name.to_uppercase();
    let cases = [
        (json!({"items":{"symbols":[],"wires":[]},"name":"Filter"}), ErrorCode::InvalidParams, "selection must contain"),
        (json!({"items":{"symbols":["symbol-999999"],"wires":[]},"name":"Filter"}), ErrorCode::InvalidParams, "selection must contain"),
        (json!({"items":{"symbols":["R1"],"wires":[]},"name":"Filter"}), ErrorCode::InvalidParams, ""),
        (json!({"items":items,"name":"  "}), ErrorCode::InvalidParams, "can't be empty"),
        (json!({"items":items,"name":taken}), ErrorCode::InvalidParams, "already exists"),
        (json!({"items":items,"name":"Filter","expected_revision":revision + 1}), ErrorCode::StaleRevision, "revision"),
        (json!({"items":items}), ErrorCode::InvalidParams, "needs `name`"),
    ];
    for (params, code, reason) in cases {
        for error in [s.execute(PREVIEW, &params).unwrap_err(), s.apply(PREVIEW, &params).unwrap_err()] {
            assert_eq!(error.code(), code, "{params}: {error}");
            assert!(error.to_string().contains(reason), "{params}: {error}");
        }
        assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
    }
}

/// More boundary nets than one sheet interface can hold are refused with the count.
#[test]
fn more_than_sixty_four_boundary_nets_are_refused() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let mut inside = Vec::new();
    for index in 0..=MAX_LABELS as i64 {
        let a = resistor(&mut s, 0, index * 10 * GRID_NM);
        let b = resistor(&mut s, 40 * GRID_NM, index * 10 * GRID_NM);
        connect(&mut s, pin(&a, "2"), pin(&b, "1"));
        inside.push(a);
    }
    let params = json!({"items":{"symbols":inside,"wires":[]},"name":"Wide"});
    let error = s.execute(PREVIEW, &params).unwrap_err();
    assert_eq!(error.code(), ErrorCode::CapExceeded);
    assert!(error.to_string().contains(&format!("crosses {} nets", MAX_LABELS + 1)), "{error}");
    let mut fewer = params.clone();
    fewer["items"]["symbols"].as_array_mut().unwrap().pop();
    assert_eq!(s.execute(PREVIEW, &fewer).unwrap()["split_preview"]["ports"].as_array().unwrap().len(), MAX_LABELS);
}

/// Units of one part may sit on different sheets, so moving only some of them is allowed
/// and reported with the units that stay.
#[test]
fn a_part_whose_units_are_split_is_reported() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let symbol = include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json");
    s.execute(
        "library.load_source",
        &json!({"source":"units","documents":{
            "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
            "symbols/dual.json":symbol,
        }}),
    )
    .unwrap();
    let definition = s.libraries().symbol("units", "dual-amplifier", 1).unwrap();
    let digest = symbol_digest(&definition).unwrap();
    apply(
        &mut s,
        PLACE_SYMBOL,
        json!({"library":"units","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let first = s.project().unwrap().symbols()[0].clone();
    apply(&mut s, PLACE_UNIT, json!({"component":first.component_id(),"unit":"b","x_nm":40 * GRID_NM,"y_nm":0}));
    let second = s.project().unwrap().symbols()[1].id();
    let report = s.execute(PREVIEW, &json!({"items":{"symbols":[first.id()],"wires":[]},"name":"Half"})).unwrap();
    let split = &report["split_preview"]["split_part_instances"];
    assert_eq!(split.as_array().map(Vec::len), Some(1));
    assert_eq!(split[0]["part_instance"], json!(first.component_id()));
    assert_eq!(split[0]["moved"], json!([first.id()]));
    assert_eq!(split[0]["staying"], json!([second]));
}

/// Proposed directions follow the drivers on each side of the boundary.
#[test]
fn port_direction_follows_the_drivers_on_each_side() {
    use ElectricalType::*;
    assert_eq!(direction(&[Passive], &[Passive]), Direction::Passive);
    assert_eq!(direction(&[Output], &[Input]), Direction::Output);
    assert_eq!(direction(&[TriState], &[Input]), Direction::TriState);
    assert_eq!(direction(&[TriState, Output], &[Input]), Direction::Output);
    assert_eq!(direction(&[Input], &[PowerOut]), Direction::Input);
    assert_eq!(direction(&[Output], &[OpenCollector]), Direction::Bidirectional);
    assert_eq!(direction(&[Passive], &[Bidirectional]), Direction::Bidirectional);
}
