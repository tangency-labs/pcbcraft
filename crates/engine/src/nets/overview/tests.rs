use super::*;
use crate::{
    DRAW_WIRE, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, Terminal,
    error_codes::ErrorCode,
    hierarchy, library, net_labels, persistence,
    schematic::{GRID_NM as G, Point},
    sheets,
};
use std::sync::Arc;

/// Author through the same revision-checked command path as the GUI and MCP.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// A new in-memory project with one sheet.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s
}

/// Add top-level sheets until the project has `count`, returning them in page order.
fn add_sheets(s: &mut Session, count: usize) -> Vec<SheetTarget> {
    let existing = s.project().unwrap().sheets().len();
    for page in existing..count {
        apply(s, sheets::CREATE_SHEET, json!({"name":format!("Page {}", page + 1)}));
    }
    s.project().unwrap().page_order()
}

/// Place a starter resistor and return its first terminal.
fn resistor(s: &mut Session, sheet: SheetTarget, x: i64) -> Terminal {
    apply(s, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":x,"y_nm":0}));
    s.project().unwrap().for_sheet(sheet).unwrap().symbols().last().unwrap().terminals()[0]
}

/// Attach a text label to a terminal at its exact anchor.
fn label(s: &mut Session, sheet: SheetTarget, pin: Terminal, name: &str, style: &str) {
    let at = s.project().unwrap().for_sheet(sheet).unwrap().terminal_position(pin).unwrap();
    apply(s, net_labels::ADD, json!({"sheet":sheet.id(),"name":name,"style":style,"x_nm":at.x_nm,"y_nm":at.y_nm}));
}

/// The command's result, checked to be the same as the typed project report.
fn overview(s: &mut Session, params: Value) -> Value {
    let result = s.execute(SHEET_OVERVIEW, &params).unwrap();
    assert_eq!(result["revision"], json!(s.revision()));
    let only = params.get("sheet").map(|id| s.project().unwrap().resolve_sheet(id.as_str()).unwrap());
    assert_eq!(result["sheet_overview"], serde_json::to_value(s.project().unwrap().sheet_overview(only).unwrap()).unwrap());
    result["sheet_overview"].clone()
}

/// Page numbers, in order, that the named net reaches.
fn pages(report: &Value, name: &str) -> Vec<u64> {
    let net = report["nets"].as_array().unwrap().iter().find(|net| net["name"] == name).unwrap_or_else(|| panic!("{name} missing: {report}"));
    net["sheets"].as_array().unwrap().iter().map(|sheet| sheet["page"].as_u64().unwrap()).collect()
}

/// Net names in report order.
fn names(report: &Value) -> Vec<&str> {
    report["nets"].as_array().unwrap().iter().map(|net| net["name"].as_str().unwrap()).collect()
}

/// Four sheets with CAN_H global labels on pages 1, 3 and 4 and a same-named local label on
/// page 2: the report lists CAN_H on pages 1, 3 and 4 only, each reached by a global label.
#[test]
fn global_label_net_lists_the_sheets_it_reaches() {
    let mut s = session();
    let pages_in_order = add_sheets(&mut s, 4);
    for (index, sheet) in pages_in_order.iter().enumerate() {
        let pin = resistor(&mut s, *sheet, 0);
        label(&mut s, *sheet, pin, "CAN_H", if index == 1 { "local" } else { "global" });
    }
    let report = overview(&mut s, json!({}));
    assert_eq!(names(&report), ["CAN_H"]);
    assert_eq!(pages(&report, "CAN_H"), [1, 3, 4]);
    let net = &report["nets"][0];
    assert_eq!(net["names"], json!(["CAN_H"]));
    assert!(net["id"].is_null());
    let project = s.project().unwrap();
    for (reach, page) in net["sheets"].as_array().unwrap().iter().zip([0, 2, 3]) {
        let target = pages_in_order[page];
        assert_eq!(reach["sheet"], json!(target.id()));
        assert_eq!(reach["occurrence"], json!(project.sheet_occurrence(target).unwrap().id));
        assert_eq!(reach["name"], json!(project.sheets().find(|sheet| sheet.target == target).unwrap().name));
        assert_eq!(reach["via"], json!(["global_label"]));
        assert_eq!(reach["terminals"], json!(1));
    }
}

/// Equal local names on different sheets are separate nets, so nothing crosses sheets.
#[test]
fn local_labels_with_one_name_do_not_cross_sheets() {
    let mut s = session();
    for sheet in add_sheets(&mut s, 2) {
        let pin = resistor(&mut s, sheet, 0);
        label(&mut s, sheet, pin, "SDA", "local");
    }
    assert_eq!(overview(&mut s, json!({})), json!({"nets":[]}));
}

/// A child's hierarchical label and its sheet pin on the parent carry one net across both
/// sheets; the parent row names the sheet pin and the child row the hierarchical label.
#[test]
fn hierarchy_reports_sheet_pin_and_hierarchical_label() {
    let mut s = session();
    let parent = s.project().unwrap().sheet_target();
    apply(&mut s, hierarchy::CREATE_CHILD, json!({"name":"Power","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, hierarchy::ADD_LABEL, json!({"sheet":child.id(),"name":"SIGNAL","direction":"input","x_nm":0,"y_nm":0}));
    let id = s.project().unwrap().for_sheet(child).unwrap().hierarchical_labels()[0].id;
    for sheet in [parent, child] {
        apply(&mut s, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":10_000_000,"y_nm":0}));
        let projection = s.project().unwrap().for_sheet(sheet).unwrap();
        let terminal = projection.symbols()[0].terminals()[0];
        let endpoint = if sheet == parent { WireEndpoint::SheetPin(id) } else { WireEndpoint::Label(id) };
        let from = projection.terminal_position(terminal).unwrap();
        let to = projection.endpoint_position(endpoint).unwrap();
        let bends = if from.x_nm != to.x_nm && from.y_nm != to.y_nm { vec![Point { x_nm: from.x_nm, y_nm: to.y_nm }] } else { vec![] };
        apply(&mut s, DRAW_WIRE, json!({"sheet":sheet.id(),"from":terminal.inspect(),"to":endpoint.inspect(),"bends":bends,"auto_heal":false}));
    }
    let report = overview(&mut s, json!({}));
    assert_eq!(names(&report), ["SIGNAL"]);
    let reach = &report["nets"][0]["sheets"];
    assert_eq!(reach[0]["sheet"], json!(parent.id()));
    assert_eq!(reach[0]["page"], json!(1));
    assert_eq!(reach[0]["via"], json!(["sheet_pin"]));
    assert_eq!(reach[1]["sheet"], json!(child.id()));
    assert_eq!(reach[1]["page"], json!(2));
    assert_eq!(reach[1]["via"], json!(["hierarchical_label"]));
    assert_eq!(reach[0]["terminals"], json!(1));
    assert_eq!(reach[1]["terminals"], json!(1));
}

/// A power symbol's value names a rail on every sheet; the symbol itself is not counted as a
/// part terminal.
#[test]
fn power_symbols_join_sheets_by_value() {
    let mut s = session();
    let targets = add_sheets(&mut s, 2);
    for sheet in &targets {
        let pin = resistor(&mut s, *sheet, 0);
        let at = s.project().unwrap().for_sheet(*sheet).unwrap().terminal_position(pin).unwrap();
        let definition = s.libraries().latest_symbol("power", "vcc").unwrap();
        let digest = library::sources::symbol_digest(&definition).unwrap();
        apply(
            &mut s,
            PLACE_SYMBOL,
            json!({"sheet":sheet.id(),"library":"power","definition":"vcc","definition_revision":definition.revision(),"definition_digest":digest,"x_nm":at.x_nm,"y_nm":at.y_nm}),
        );
    }
    let report = overview(&mut s, json!({}));
    assert_eq!(names(&report), ["VCC"]);
    for reach in report["nets"][0]["sheets"].as_array().unwrap() {
        assert_eq!(reach["via"], json!(["power_symbol"]));
        assert_eq!(reach["terminals"], json!(1));
    }
}

/// Units of one part on two sheets draw the shared supply terminal on both. Once one drawing
/// is connected, its net reaches both sheets through the shared part terminal; bare shared
/// drawings stay unconnected and are not listed.
#[test]
fn units_of_one_part_on_two_sheets_share_terminals() {
    let mut s = session();
    s.execute(
        "library.load_source",
        &json!({"source":"units", "documents":{
            "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
            "symbols/dual.json":include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json"),
        }}),
    )
    .unwrap();
    let definition = s.libraries.symbol("units", "dual-amplifier", 1).unwrap();
    let digest = library::sources::symbol_digest(&definition).unwrap();
    let targets = add_sheets(&mut s, 2);
    apply(
        &mut s,
        PLACE_SYMBOL,
        json!({"sheet":targets[0].id(),"library":"units","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let first = s.project().unwrap().for_sheet(targets[0]).unwrap().symbols()[0].clone();
    apply(&mut s, PLACE_UNIT, json!({"sheet":targets[1].id(),"component":first.component_id(),"unit":"b","x_nm":0,"y_nm":0}));
    assert_eq!(overview(&mut s, json!({})), json!({"nets":[]}));
    let supply = first.terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    label(&mut s, targets[0], supply, "VP", "local");
    let report = overview(&mut s, json!({}));
    assert_eq!(names(&report), ["VP"]);
    assert_eq!(pages(&report, "VP"), [1, 2]);
    for reach in report["nets"][0]["sheets"].as_array().unwrap() {
        assert_eq!(reach["via"], json!(["shared_terminal"]));
        assert_eq!(reach["terminals"], json!(1));
    }
}

/// `sheet` keeps only the nets that reach that sheet, and an unknown or malformed sheet, an
/// extra parameter or a closed project is refused without changing anything.
#[test]
fn sheet_filter_and_refusals() {
    let mut s = session();
    let targets = add_sheets(&mut s, 3);
    for (index, sheet) in targets.iter().enumerate() {
        let pin = resistor(&mut s, *sheet, 0);
        label(&mut s, *sheet, pin, if index == 2 { "GND" } else { "CAN_H" }, "global");
        let other = resistor(&mut s, *sheet, 20 * G);
        if index != 1 {
            label(&mut s, *sheet, other, "GND", "global");
        }
    }
    assert_eq!(names(&overview(&mut s, json!({}))), ["CAN_H", "GND"]);
    assert_eq!(names(&overview(&mut s, json!({"sheet":targets[1].id()}))), ["CAN_H"]);
    let only_last = overview(&mut s, json!({"sheet":targets[2].id()}));
    assert_eq!(names(&only_last), ["GND"]);
    assert_eq!(pages(&only_last, "GND"), [1, 3]);
    assert_eq!(only_last["nets"][0]["sheets"][1]["terminals"], json!(2));
    let revision = s.revision();
    for (params, code) in [
        (json!({"sheet":Uuid::from_u128(0x6f1c_2b9e_0000_4000_8000_0000_0000_0001)}), ErrorCode::NotFound),
        (json!({"sheet":"page-1"}), ErrorCode::InvalidParams),
        (json!({"sheet":7}), ErrorCode::InvalidParams),
        (json!({"page":1}), ErrorCode::InvalidParams),
    ] {
        assert_eq!(s.execute(SHEET_OVERVIEW, &params).unwrap_err().code(), code, "{params}");
        assert_eq!(s.apply(SHEET_OVERVIEW, &params).unwrap_err().code(), code, "{params}");
    }
    assert_eq!(s.revision(), revision);
    let mut closed = Session::default();
    assert_eq!(closed.execute(SHEET_OVERVIEW, &json!({})).unwrap_err().code(), ErrorCode::Unavailable);
    assert!(!closed.available(SHEET_OVERVIEW));
    assert!(s.available(SHEET_OVERVIEW));
}

/// The query changes no revision, history or snapshot through either entry point, follows
/// edits and Undo, survives save and reopen, and reports a promoted net's persistent ID.
#[test]
fn query_is_read_only_and_follows_history_and_reopen() {
    let mut s = session();
    let targets = add_sheets(&mut s, 2);
    let pins: Vec<_> = targets.iter().map(|sheet| resistor(&mut s, *sheet, 0)).collect();
    label(&mut s, targets[0], pins[0], "CAN_H", "global");
    assert_eq!(overview(&mut s, json!({})), json!({"nets":[]}));
    label(&mut s, targets[1], pins[1], "CAN_H", "global");
    let snapshot = s.snapshot().unwrap();
    let (revision, undo) = (s.revision(), s.undo.len());
    let report = overview(&mut s, json!({}));
    assert_eq!(s.apply(SHEET_OVERVIEW, &json!({"sheet":targets[1].id()})).unwrap(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!((s.revision(), s.undo.len()), (revision, undo));
    assert_eq!(pages(&report, "CAN_H"), [1, 2]);

    assert!(snapshot.same_sheet_overview(&snapshot.for_sheet(targets[1]).unwrap()));
    let saved = persistence::encode_documents(snapshot.clone()).unwrap();
    let reopened = persistence::decode_documents(&saved).unwrap();
    assert_eq!(reopened.sheet_overview(None).unwrap(), snapshot.sheet_overview(None).unwrap());

    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(!s.snapshot().unwrap().same_sheet_overview(&snapshot));
    assert_eq!(overview(&mut s, json!({})), json!({"nets":[]}));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(overview(&mut s, json!({})), report);

    apply(&mut s, crate::net_classes::CREATE, json!({"name":"Bus","rules":{"clearance_nm":500_000}}));
    let class = s.project().unwrap().net_classes().definitions[0].id;
    let target = json!({"kind":"derived","anchor":report["nets"][0]["anchor"]});
    apply(&mut s, crate::nets::SET_CLASSES, json!({"target":target,"classes":[class]}));
    let promoted = overview(&mut s, json!({}));
    let id = s.project().unwrap().net_registry.records[0].id;
    assert_eq!(promoted["nets"][0]["id"], json!(id));
    assert_eq!(promoted["nets"][0]["sheets"], report["nets"][0]["sheets"]);
}

/// Moving a part without changing what it connects keeps an equal basis, so a viewer can keep
/// its overview; joining that part to a cross-sheet net changes the basis.
#[test]
fn moves_keep_the_basis_and_connection_changes_replace_it() {
    let mut s = session();
    let targets = add_sheets(&mut s, 2);
    for sheet in &targets {
        let pin = resistor(&mut s, *sheet, 0);
        label(&mut s, *sheet, pin, "CAN_H", "global");
    }
    let spare = resistor(&mut s, targets[0], 40 * G);
    let before = s.snapshot().unwrap();
    let basis = before.sheet_overview_basis().unwrap();
    apply(&mut s, crate::MOVE_SYMBOL, json!({"sheet":targets[0].id(),"symbol":spare.inspect()["symbol"],"x_nm":40 * G,"y_nm":20 * G}));
    let moved = s.snapshot().unwrap();
    assert!(!moved.same_sheet_overview(&before));
    assert_eq!(moved.sheet_overview_basis().unwrap(), basis);
    assert_eq!(basis.overview(&moved, None).unwrap(), before.sheet_overview(None).unwrap());

    label(&mut s, targets[0], spare, "CAN_H", "global");
    assert_ne!(s.snapshot().unwrap().sheet_overview_basis().unwrap(), basis);
    let report = overview(&mut s, json!({}));
    assert_eq!(pages(&report, "CAN_H"), [1, 2]);
    assert_eq!(report["nets"][0]["sheets"][0]["terminals"], json!(2));
}
