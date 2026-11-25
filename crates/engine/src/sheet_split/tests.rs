use super::*;
use crate::{
    CONNECT_PINS, DRAW_WIRE, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, SheetTarget,
    error_codes::ErrorCode,
    hierarchy::{ADD_LABEL, CREATE_CHILD},
    library::sources::symbol_digest,
    net_labels,
    persistence::encode_documents,
    schematic::{GRID_NM, Point},
    sheets::CREATE_SHEET,
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

/// A symbol terminal as a wire endpoint.
fn terminal(symbol: &str, number: &str) -> WireEndpoint {
    WireEndpoint::Terminal(Terminal::parse(&pin(symbol, number)).unwrap())
}

/// The symbol most recently placed on a sheet.
fn last_symbol(s: &Session, sheet: SheetTarget) -> String {
    s.project().unwrap().for_sheet(sheet).unwrap().symbols().last().unwrap().id()
}

/// Draw one wire between two existing anchors on a sheet, horizontal first when they are
/// not aligned, without healing so each wire keeps its own identity.
fn route(s: &mut Session, sheet: SheetTarget, from: WireEndpoint, to: WireEndpoint) {
    let (a, b) = {
        let project = s.project().unwrap().for_sheet(sheet).unwrap();
        (project.endpoint_position(from).unwrap(), project.endpoint_position(to).unwrap())
    };
    let bends = if a.x_nm != b.x_nm && a.y_nm != b.y_nm { vec![Point { x_nm: b.x_nm, y_nm: a.y_nm }] } else { vec![] };
    apply(s, DRAW_WIRE, json!({"sheet":sheet.id(),"from":from.inspect(),"to":to.inspect(),"bends":bends,"auto_heal":false}));
}

/// Name a terminal's net with a local label at the terminal.
fn local_label(s: &mut Session, sheet: SheetTarget, at: WireEndpoint, name: &str) {
    let point = s.project().unwrap().for_sheet(sheet).unwrap().endpoint_position(at).unwrap();
    apply(s, net_labels::ADD, json!({"sheet":sheet.id(),"name":name,"style":"local","x_nm":point.x_nm,"y_nm":point.y_nm}));
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
    let wire = project.wires().first().unwrap().id();
    let cases = [
        (json!({"items":{"symbols":[],"wires":[wire]},"name":"Filter"}), ErrorCode::InvalidParams, "at least one symbol or net label"),
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

/// A junction travels only when every branch stays inside. Three selected resistors meeting
/// at a T move whole with their junction and need no port. A branch to an outside resistor
/// keeps the junction outside: every inside wire is cut, a selected one included, and the
/// net becomes a port.
#[test]
fn junctions_travel_only_when_every_branch_stays_inside() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let sheet = s.project().unwrap().sheet_target();
    let row = 10 * GRID_NM;
    let parts = [resistor(&mut s, 0, 0), resistor(&mut s, 0, row), resistor(&mut s, 0, 2 * row)];
    let start = terminal(&parts[1], "2");
    let at = s.project().unwrap().endpoint_position(start).unwrap();
    let node = Point { x_nm: at.x_nm + 30 * GRID_NM, y_nm: at.y_nm };
    apply(&mut s, DRAW_WIRE, json!({"from":start.inspect(),"to":node,"bends":[],"auto_heal":false}));
    let first = s.project().unwrap().wires().last().unwrap().id();
    let id = s.project().unwrap().junctions().find(|(_, point)| *point == node).unwrap().0;
    let junction = WireEndpoint::Junction(id);
    route(&mut s, sheet, terminal(&parts[0], "2"), junction);
    route(&mut s, sheet, terminal(&parts[2], "2"), junction);

    let report = s.execute(PREVIEW, &json!({"items":{"symbols":parts,"wires":[]},"name":"Tee"})).unwrap();
    let preview = &report["split_preview"];
    assert_eq!(preview["junctions"], json!([format!("junction-{id}")]));
    assert_eq!(preview["wires"].as_array().unwrap().len(), 3);
    assert_eq!(preview["cut_wires"], json!([]));
    assert_eq!(preview["ports"], json!([]));

    let outside = resistor(&mut s, at.x_nm + 60 * GRID_NM, row);
    route(&mut s, sheet, terminal(&outside, "1"), junction);
    let report = s.execute(PREVIEW, &json!({"items":{"symbols":parts,"wires":[first]},"name":"Tee"})).unwrap();
    let preview = &report["split_preview"];
    assert_eq!(preview["junctions"], json!([]));
    assert_eq!(preview["wires"], json!([]));
    let cut = preview["cut_wires"].as_array().unwrap();
    assert_eq!(cut.len(), 3);
    assert!(cut.iter().all(|wire| wire["outside"] == junction.inspect()), "{preview}");
    assert!(cut.iter().any(|wire| wire["wire"] == json!(first)));
    assert_eq!(preview["staying_wires"], json!([]));
    let ports = preview["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 1, "{preview}");
    assert_eq!(ports[0]["inside"].as_array().unwrap().len(), 3);
    assert_eq!(ports[0]["outside"], json!([pin(&outside, "1")]));
}

/// Wires and no-connect markers follow what they attach to. A selected wire whose ends both
/// stay, and the marker on its dangling end, are reported as staying rather than moving.
#[test]
fn selected_wires_and_markers_that_stay_are_reported() {
    let mut f = fixture();
    let s = &mut f.session;
    let stub = resistor(s, 80 * GRID_NM, 0);
    let start = terminal(&stub, "1");
    let at = s.project().unwrap().endpoint_position(start).unwrap();
    let end = Point { x_nm: at.x_nm - 5 * GRID_NM, y_nm: at.y_nm };
    apply(s, DRAW_WIRE, json!({"from":start.inspect(),"to":end,"bends":[],"auto_heal":false}));
    let wire = s.project().unwrap().wires().last().unwrap().id();
    let id = s.project().unwrap().junctions().find(|(_, point)| *point == end).unwrap().0;
    let marker = json!({"junction":format!("junction-{id}")});
    apply(s, crate::no_connect::SET, json!({"target":marker,"marked":true}));

    let items = json!({"symbols":[f.inside[0]],"wires":[wire],"no_connects":[marker]});
    let report = s.execute(PREVIEW, &json!({"items":items,"name":"Stub"})).unwrap();
    let preview = &report["split_preview"];
    assert_eq!(preview["staying_wires"], json!([wire]));
    assert_eq!(preview["staying_no_connects"], json!([marker]));
    assert_eq!(preview["no_connects"], json!([]));
    assert!(!preview["wires"].as_array().unwrap().contains(&json!(wire)));
}

/// A port takes the net's authored name even when the same name on another sheet makes the
/// net's displayed name longer. Two boundary nets with the same authored name would give
/// the new sheet two equal labels, so that is refused.
#[test]
fn ports_take_the_authored_name_and_a_repeated_name_is_refused() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    let first = resistor(&mut s, 0, 0);
    let across = resistor(&mut s, 40 * GRID_NM, 0);
    connect(&mut s, pin(&first, "2"), pin(&across, "1"));
    local_label(&mut s, main, terminal(&across, "1"), "EN");
    apply(&mut s, CREATE_SHEET, json!({"name":"Other"}));
    let other = s.project().unwrap().sheets().find(|sheet| sheet.name == "Other").unwrap().target;
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":other.id(),"x_nm":0,"y_nm":0}));
    let remote = last_symbol(&s, other);
    local_label(&mut s, other, terminal(&remote, "1"), "EN");
    {
        let project = s.project().unwrap().for_sheet(main).unwrap();
        let graph = project.borrowed_electrical_connectivity();
        let index = graph.net_index_for_endpoint(terminal(&across, "1")).unwrap();
        assert_ne!(graph.borrowed_net_name(index), Some("EN"), "the displayed name is qualified");
    }
    let report = s.execute(PREVIEW, &json!({"sheet":main.id(),"items":{"symbols":[first],"wires":[]},"name":"Split"})).unwrap();
    let ports = report["split_preview"]["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0]["name"], "EN");
    assert_eq!(ports[0]["generated"], false);

    // A second boundary net reaches a child sheet, where a local label also names it EN.
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":main.id(),"x_nm":0,"y_nm":10 * GRID_NM}));
    let second = last_symbol(&s, main);
    apply(&mut s, CREATE_CHILD, json!({"sheet":main.id(),"name":"Child","x_nm":40 * GRID_NM,"y_nm":20 * GRID_NM}));
    let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"LINK","direction":"input","x_nm":0,"y_nm":0}));
    let link = s.project().unwrap().for_sheet(child).unwrap().labels[0].id;
    route(&mut s, main, terminal(&second, "2"), WireEndpoint::SheetPin(link));
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":child.id(),"x_nm":10 * GRID_NM,"y_nm":0}));
    let inner = last_symbol(&s, child);
    route(&mut s, child, terminal(&inner, "1"), WireEndpoint::Label(link));
    local_label(&mut s, child, terminal(&inner, "1"), "EN");

    let project = s.snapshot().unwrap();
    let params = json!({"sheet":main.id(),"items":{"symbols":[first, second],"wires":[]},"name":"Split"});
    let error = s.execute(PREVIEW, &params).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams);
    assert!(error.to_string().contains("share the port name EN"), "{error}");
    assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
}

/// A project that already has the most sheets cannot take a new child sheet.
#[test]
fn a_full_project_is_refused() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    let part = resistor(&mut s, 0, 0);
    for index in 1..MAX_SHEETS {
        apply(&mut s, CREATE_SHEET, json!({"name":format!("Extra {index}")}));
    }
    let project = s.snapshot().unwrap();
    let error = s.execute(PREVIEW, &json!({"sheet":main.id(),"items":{"symbols":[part],"wires":[]},"name":"Split"})).unwrap_err();
    assert_eq!(error.code(), ErrorCode::CapExceeded);
    assert!(error.to_string().contains(&format!("already has {MAX_SHEETS} sheets")), "{error}");
    assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
}

/// The net row of an endpoint in the whole-project graph.
fn net_of(s: &Session, end: WireEndpoint) -> Option<usize> {
    s.project().unwrap().borrowed_electrical_connectivity().net_index_for_endpoint(end)
}

/// The sheet with this name, as an address.
fn sheet_named(s: &Session, name: &str) -> SheetTarget {
    s.project().unwrap().sheets().find(|sheet| sheet.name == name).unwrap().target
}

/// A third sheet's GND label cannot reconnect the parent resistor after its wire is cut.
#[test]
fn a_third_sheet_ground_needs_a_port_for_the_cut_parent_wire() -> Result<(), CommandError> {
    let mut s = Session::default();
    s.apply("project.new", &json!({}))?;
    let part = resistor(&mut s, 0, 0);
    let gnd = ground(&mut s, 40 * GRID_NM, 0);
    let parent = s.open_project()?.sheet_target();
    connect(&mut s, pin(&part, "2"), pin(&gnd, "1"));
    apply(&mut s, CREATE_SHEET, json!({"name":"Other"}));
    let other = sheet_named(&s, "Other");
    apply(&mut s, net_labels::ADD, json!({"sheet":other.id(),"name":"GND","style":"global","x_nm":0,"y_nm":0}));

    let Some(before) = s.snapshot() else { panic!("expected an open project") };
    let params = json!({"sheet":parent.id(),"items":{"symbols":[gnd],"wires":[]},"name":"Split","expected_revision":s.revision()});
    let report = s.execute(PREVIEW, &params)?;
    let preview = &report["split_preview"];
    assert_eq!(preview["cut_wires"].as_array().map(Vec::len), Some(1));
    let Some([port]) = preview["ports"].as_array().map(Vec::as_slice) else { panic!("expected one GND port: {preview}") };
    assert_eq!(port["name"], "GND");
    assert_eq!(port["generated"], false);
    assert_eq!(preview["global_nets"], json!([]));

    s.apply(SPLIT, &params)?;
    let Some(after) = s.snapshot() else { panic!("expected the split project") };
    let Some(child) = after.for_sheet(sheet_named(&s, "Split")) else { panic!("expected the new child sheet") };
    let [label] = child.hierarchical_labels() else { panic!("expected the previewed GND port") };
    assert_eq!(port["name"], label.name.as_str());
    let resistor_net = net_of(&s, terminal(&part, "2"));
    assert!(resistor_net.is_some());
    assert_eq!(resistor_net, net_of(&s, WireEndpoint::SheetPin(label.id)));
    assert_eq!(resistor_net, net_of(&s, terminal(&gnd, "1")));
    assert!(same_netlist(before.borrowed_electrical_connectivity(), after.borrowed_electrical_connectivity()));
    Ok(())
}

/// Splitting the fixture moves exactly the previewed symbols onto a new child sheet with their
/// IDs, references and positions, adds one hierarchical label and sheet pin per previewed port,
/// keeps every terminal on its net, and is one undo step that undo and redo replay exactly.
#[test]
fn split_moves_the_previewed_objects_and_keeps_every_net() {
    let mut f = fixture();
    let items = selection(&f);
    let s = &mut f.session;
    let before = s.snapshot().unwrap();
    let main = before.sheet_target();
    let preview = s.execute(PREVIEW, &json!({"items":items,"name":"Filter"})).unwrap()["split_preview"].clone();
    let moved: Vec<_> =
        before.symbols().iter().filter(|symbol| items["symbols"].as_array().unwrap().contains(&json!(symbol.id()))).cloned().collect();
    let pairs = [
        (terminal(&f.inside[0], "2"), terminal(&f.outside[0], "1")),
        (terminal(&f.inside[1], "2"), terminal(&f.outside[1], "1")),
        (terminal(&f.inside[2], "2"), terminal(&f.outside[2], "1")),
        (terminal(&f.inside[0], "1"), terminal(&f.inside[1], "1")),
        (terminal(&f.outside[0], "2"), terminal(&f.outside[1], "2")),
        (terminal(&f.grounds[0], "1"), terminal(&f.grounds[1], "1")),
        (terminal(&f.inside[2], "1"), terminal(&f.grounds[0], "1")),
    ];
    for (a, b) in pairs {
        assert_eq!(net_of(s, a), net_of(s, b));
    }
    let crossings = [terminal(&f.inside[0], "2"), terminal(&f.inside[1], "2"), terminal(&f.inside[2], "2")];
    assert_ne!(net_of(s, crossings[0]), net_of(s, crossings[1]));
    let revision = s.revision();

    let next = s.apply(SPLIT, &json!({"items":items,"name":"Filter","expected_revision":revision})).unwrap();
    assert_ne!(next, revision);
    let project = s.project().unwrap();
    let child = sheet_named(s, "Filter");
    let parent_view = project.for_sheet(main).unwrap();
    let child_view = project.for_sheet(child).unwrap();
    assert_eq!(project.parent_sheet(child), Some(main));
    assert_eq!(parent_view.sheet_symbols().len(), 1);
    for symbol in &moved {
        let placed = child_view.symbols().iter().find(|other| other.id() == symbol.id()).unwrap();
        assert_eq!(placed.position, symbol.position);
        assert_eq!(placed.component_id(), symbol.component_id());
        assert_eq!(project.reference(placed), before.reference(symbol));
        assert!(!parent_view.symbols().iter().any(|other| other.id() == symbol.id()));
    }
    assert_eq!(child_view.symbols().len(), moved.len());
    assert_eq!(parent_view.symbols().len(), before.symbols().len() - moved.len());
    for wire in preview["wires"].as_array().unwrap() {
        assert!(child_view.wires().iter().any(|other| json!(other.id()) == *wire), "{wire}");
    }

    let ports = preview["ports"].as_array().unwrap();
    let labels = child_view.hierarchical_labels();
    assert_eq!(labels.len(), ports.len());
    for port in ports {
        let label = labels.iter().find(|label| json!(label.name) == port["name"]).unwrap();
        assert_eq!(json!(label.direction), port["direction"]);
        let pin = WireEndpoint::SheetPin(label.id);
        for member in port["inside"].as_array().unwrap() {
            assert_eq!(net_of(s, WireEndpoint::parse(member).unwrap()), net_of(s, WireEndpoint::Label(label.id)), "{port}");
        }
        for member in port["outside"].as_array().unwrap() {
            assert_eq!(net_of(s, WireEndpoint::parse(member).unwrap()), net_of(s, pin), "{port}");
        }
    }
    for (a, b) in pairs {
        assert_eq!(net_of(s, a), net_of(s, b));
    }
    assert_ne!(net_of(s, crossings[0]), net_of(s, crossings[1]));
    assert_ne!(net_of(s, crossings[1]), net_of(s, crossings[2]));
    let sda = net_of(s, terminal(&f.outside[0], "1")).unwrap();
    assert!(s.project().unwrap().borrowed_electrical_connectivity().net_names(sda).contains(&"SDA".to_owned()));

    let after = s.snapshot().unwrap();
    s.execute("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.execute("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&after, &s.snapshot().unwrap());
}

/// A selection meeting an outside branch at a junction: the inside wires move with their
/// ends on the new label, the junction stays and reaches the sheet pin, and all four
/// resistors stay on one net.
#[test]
fn a_junction_that_stays_is_wired_to_the_sheet_pin() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let sheet = s.project().unwrap().sheet_target();
    let row = 10 * GRID_NM;
    let parts = [resistor(&mut s, 0, 0), resistor(&mut s, 0, row), resistor(&mut s, 0, 2 * row)];
    let start = terminal(&parts[1], "2");
    let at = s.project().unwrap().endpoint_position(start).unwrap();
    let node = Point { x_nm: at.x_nm + 30 * GRID_NM, y_nm: at.y_nm };
    apply(&mut s, DRAW_WIRE, json!({"from":start.inspect(),"to":node,"bends":[],"auto_heal":false}));
    let id = s.project().unwrap().junctions().find(|(_, point)| *point == node).unwrap().0;
    let junction = WireEndpoint::Junction(id);
    route(&mut s, sheet, terminal(&parts[0], "2"), junction);
    route(&mut s, sheet, terminal(&parts[2], "2"), junction);
    let outside = resistor(&mut s, at.x_nm + 60 * GRID_NM, row);
    route(&mut s, sheet, terminal(&outside, "1"), junction);

    apply(&mut s, SPLIT, json!({"items":{"symbols":parts,"wires":[]},"name":"Tee"}));
    let project = s.project().unwrap();
    let child = project.for_sheet(sheet_named(&s, "Tee")).unwrap();
    let parent = project.for_sheet(sheet).unwrap();
    let [label] = child.hierarchical_labels() else { panic!("one port label") };
    assert_eq!(label.at, node);
    assert_eq!(child.wires().iter().filter(|wire| wire.endpoints().contains(&WireEndpoint::Label(label.id))).count(), 3);
    assert!(parent.junctions().any(|(other, _)| other == id));
    assert!(parent.wires().iter().any(|wire| wire.endpoints().contains(&WireEndpoint::SheetPin(label.id))));
    let net = net_of(&s, terminal(&outside, "1"));
    assert!(net.is_some());
    for part in &parts {
        assert_eq!(net_of(&s, terminal(part, "2")), net);
    }
}

/// Over sheet geometry, the sheet symbol goes where the caller asks; a lone coordinate,
/// a stale revision and a taken name are refused and leave the exact snapshot.
#[test]
fn split_places_the_sheet_symbol_and_refuses_bad_requests() {
    let mut f = fixture();
    let items = selection(&f);
    let s = &mut f.session;
    let project = s.snapshot().unwrap();
    let revision = s.revision();
    let taken = project.sheets().next().unwrap().name.to_owned();
    let wire = project.wires().first().unwrap().id();
    let only_wires = json!({"symbols":[],"wires":[wire]});
    let cases = [
        (json!({"items":only_wires,"name":"Filter","expected_revision":revision}), ErrorCode::InvalidParams, "at least one symbol or net label"),
        (
            json!({"items":only_wires,"name":"Filter","x_nm":0,"y_nm":0,"expected_revision":revision}),
            ErrorCode::InvalidParams,
            "at least one symbol or net label",
        ),
        (json!({"items":items,"name":"Filter","x_nm":0,"expected_revision":revision}), ErrorCode::InvalidParams, "must be given together"),
        (json!({"items":items,"name":"Filter","expected_revision":revision + 1}), ErrorCode::StaleRevision, "revision"),
        (json!({"items":items,"name":taken,"expected_revision":revision}), ErrorCode::InvalidParams, "already exists"),
        (json!({"items":{"symbols":[],"wires":[]},"name":"Filter","expected_revision":revision}), ErrorCode::InvalidParams, "selection must contain"),
    ];
    for (params, code, reason) in cases {
        let error = s.apply(SPLIT, &params).unwrap_err();
        assert_eq!(error.code(), code, "{params}: {error}");
        assert!(error.to_string().contains(reason), "{params}: {error}");
        assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
    }
    let at = Point { x_nm: -60 * GRID_NM, y_nm: 20 * GRID_NM };
    apply(s, SPLIT, json!({"items":items,"name":"Filter","x_nm":at.x_nm,"y_nm":at.y_nm}));
    let main = s.project().unwrap().for_sheet(project.sheet_target()).unwrap();
    assert_eq!(main.sheet_symbols().iter().map(|block| block.at).collect::<Vec<_>>(), [at]);
    let again =
        s.execute(PREVIEW, &json!({"sheet":project.sheet_target().id(),"items":{"symbols":[f.outside[0]],"wires":[]},"name":"filter"})).unwrap_err();
    assert!(again.to_string().contains("already exists"), "{again}");
}

/// The netlist guard tells equal groupings from changed ones: a new connection between
/// two nets, or a net that loses a name, is a different netlist.
#[test]
fn the_netlist_guard_detects_joined_nets_and_lost_names() {
    let mut f = fixture();
    let s = &mut f.session;
    let before = s.project().unwrap().electrical_connectivity();
    assert!(same_netlist(&before, &before));
    connect(s, pin(&f.inside[1], "2"), pin(&f.inside[2], "2"));
    let joined = s.project().unwrap().electrical_connectivity();
    assert!(!same_netlist(&before, &joined));
    let mut f = fixture();
    let s = &mut f.session;
    let label = s.project().unwrap().net_labels().first().unwrap().object_id();
    apply(s, net_labels::DELETE, json!({"id":label}));
    let unnamed = s.project().unwrap().electrical_connectivity();
    assert!(!same_netlist(&before, &unnamed));
    assert!(same_netlist(&unnamed, &before), "adding a name keeps the netlist");
}

/// A schematic-to-board update after the split proposes nothing when every boundary net
/// already had a name. A net known only by its generated number gains the port name, which
/// the update reports as the same terminals under a new name.
#[test]
fn board_update_after_a_split_changes_only_unnamed_boundary_nets() {
    use crate::board::{
        assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT},
        definition::basic_components,
        update::{Baseline, SourceCapture, preview},
    };
    for named in [true, false] {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        let first = resistor(&mut s, 0, 0);
        let second = resistor(&mut s, 40 * GRID_NM, 0);
        connect(&mut s, pin(&first, "2"), pin(&second, "1"));
        let main = s.project().unwrap().sheet_target();
        if named {
            local_label(&mut s, main, terminal(&second, "1"), "SIG");
        }
        let parts: Vec<_> = s.project().unwrap().symbols().iter().map(|symbol| symbol.component_id()).collect();
        let definition = basic_components("resistor-0402").unwrap();
        for (index, part) in parts.iter().enumerate() {
            apply(
                &mut s,
                ASSIGN_FOOTPRINT,
                json!({"component":part,"library":definition.library,"definition":definition.id,
                    "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
            );
            apply(&mut s, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part,"x_nm":index as i64 * 8_000_000,"y_nm":2_000_000}));
        }
        let capture = |project: &Project, id: u128| {
            let source = SourceCapture { capture_id: uuid::Uuid::from_u128(id), content_digest: "a".repeat(64) };
            Baseline::proposed(project, &Baseline::current_choices(project), source).unwrap()
        };
        let accepted = capture(s.project().unwrap(), 1);
        apply(&mut s, SPLIT, json!({"items":{"symbols":[first],"wires":[]},"name":"Split"}));
        let update = preview(&accepted, &capture(s.project().unwrap(), 2)).unwrap();
        if named {
            assert!(update.is_noop(), "{:?} {:?}", update.removed_nets, update.added_nets);
        } else {
            assert!(update.components.is_empty() && update.blockers.is_empty());
            let ([removed], [added]) = (update.removed_nets.as_slice(), update.added_nets.as_slice()) else {
                panic!("one renamed net: {:?} {:?}", update.removed_nets, update.added_nets)
            };
            assert_eq!(removed.terminals, added.terminals);
            assert!(removed.names.is_empty());
            assert_eq!(added.names.len(), 1);
            assert!(added.names[0].starts_with("Net-"), "{added:?}");
        }
    }
}

/// Place a one-pin part whose primary function is an input and whose alternate `TX` is an
/// output, published through the shared symbol-edit transaction, and return its symbol ID.
fn uart_part(s: &mut Session) -> String {
    use crate::authoring::{Action, Item, Kind, Primitive};
    let item = Item::new(Kind::Symbol).unwrap();
    let mut primitive = item.objects[&1].clone();
    let Primitive::Pin { name, electrical_type, alternates, .. } = &mut primitive else { panic!("expected starter pin") };
    *name = Some("RX".into());
    *electrical_type = ElectricalType::Input;
    *alternates = vec![crate::schematic::AlternatePinFunction { name: "TX".into(), electrical_type: ElectricalType::Output }];
    let mut editor = crate::authoring::Session::new(item, false).unwrap();
    editor.execute(0, Action::Replace { object: 1, primitive }).unwrap();
    let manifest = crate::authoring::publishing::new_library("synthetic", "Synthetic", "MIT OR Apache-2.0").unwrap();
    let published = crate::authoring::publishing::prepare_copy(editor.snapshot(), &manifest).unwrap();
    let item = published.item();
    let mut documents = published.documents().clone();
    documents.insert("library.json".into(), manifest);
    s.execute("library.load_source", &json!({"source":"uart","documents":documents})).unwrap();
    let definition = s.libraries.symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = symbol_digest(&definition).unwrap();
    apply(
        s,
        PLACE_SYMBOL,
        json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    s.project().unwrap().symbols().last().unwrap().id()
}

/// The selected alternate pin function of the part's terminal, if any.
fn selected_function(s: &Session) -> Option<String> {
    let project = s.project().unwrap();
    let part = project.components().find(|part| part.definition.pins().iter().any(|pin| !pin.alternates.is_empty())).unwrap();
    project.pin_function(part, "1").map(str::to_owned)
}

/// A selected alternate pin function sets the port direction as the netlist sees it. A part
/// whose input pin meets a passive resistor proposes a passive port; with its output
/// alternate selected, the preview and the saved child label are outputs, and the selection
/// survives the split and its undo.
#[test]
fn port_direction_uses_the_selected_alternate_pin_function() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let uart = uart_part(&mut s);
    let outside = resistor(&mut s, 40 * GRID_NM, 0);
    connect(&mut s, pin(&uart, "1"), pin(&outside, "1"));
    let items = json!({"symbols":[uart],"wires":[]});
    let port_direction = |s: &mut Session| {
        let preview = s.execute(PREVIEW, &json!({"items":items,"name":"Uart"})).unwrap()["split_preview"].clone();
        let [port] = preview["ports"].as_array().unwrap().as_slice() else { panic!("one port: {preview}") };
        port["direction"].clone()
    };
    assert_eq!(port_direction(&mut s), json!("passive"));

    let part = s.project().unwrap().symbols().iter().find(|symbol| symbol.id() == uart).unwrap().component_id();
    apply(&mut s, crate::pin_functions::SET, json!({"target":{"part":part,"terminal":"1"},"function":"TX"}));
    assert_eq!(selected_function(&s).as_deref(), Some("TX"));
    assert_eq!(port_direction(&mut s), json!("output"));

    apply(&mut s, SPLIT, json!({"items":items,"name":"Uart"}));
    let child = sheet_named(&s, "Uart");
    let directions: Vec<_> = s.project().unwrap().for_sheet(child).unwrap().hierarchical_labels().iter().map(|label| label.direction).collect();
    assert_eq!(directions, [Direction::Output]);
    assert_eq!(selected_function(&s).as_deref(), Some("TX"));

    s.execute("edit.undo", &Value::Null).unwrap();
    assert!(s.project().unwrap().sheets().all(|sheet| sheet.name != "Uart"));
    assert_eq!(selected_function(&s).as_deref(), Some("TX"));
    assert_eq!(port_direction(&mut s), json!("output"));
}
