//! Schematic capacity: a project filled to the symbol and wire limits across four sheets, and a
//! sheet filled to the net-label limit, all built through the ordinary schematic commands.
use super::{GRID_NM, MAX_SYMBOLS, Point, Wire, WireEndpoint, wiring::MAX_WIRES};
use crate::{
    DRAW_WIRE, PLACE_SYMBOL, Session, SheetTarget, clipboard,
    net_labels::{self, MAX_LABELS},
    persistence::{MAX_DOCUMENT_BYTES, MAX_DOCUMENTS_BYTES, decode_documents, decode_packed, encode_documents, encode_packed},
    selection::Selection,
    sheets::CREATE_SHEET,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Resistors in one row; each row also carries twice as many wires.
const ROW: usize = 32;
/// Rows on each sheet; four sheets of these reach the symbol limit.
const ROWS: usize = 64;
const SHEETS: usize = 4;
/// Horizontal pitch of the row and vertical pitch between rows.
const PITCH_X: i64 = 20 * GRID_NM;
const PITCH_Y: i64 = 10 * GRID_NM;

/// Left-to-right position of a resistor in its row, centered on the origin.
fn row_x(index: usize) -> i64 {
    (index as i64 - ROW as i64 / 2) * PITCH_X
}

/// Top-to-bottom position of a row on its sheet, centered on the origin.
fn row_y(row: usize) -> i64 {
    (row as i64 - ROWS as i64 / 2) * PITCH_Y
}

/// One row of resistors chained pin 2 to the next pin 1, with a short vertical stub below every
/// pin 1 and below the last pin 2: `ROW` symbols and `2 * ROW` wires.
fn first_row(session: &mut Session) {
    for index in 0..ROW {
        let params = json!({"definition":"resistor","x_nm":row_x(index),"y_nm":row_y(0),"expected_revision":session.revision()});
        session.apply(PLACE_SYMBOL, &params).unwrap();
    }
    let project = session.project().unwrap().clone();
    let symbols = project.symbols();
    for pair in symbols.windows(2) {
        let (from, to) = (pair[0].terminals()[1].inspect(), pair[1].terminals()[0].inspect());
        session.apply(DRAW_WIRE, &json!({"from":from,"to":to,"bends":[],"expected_revision":session.revision()})).unwrap();
    }
    let ends = symbols.iter().map(|symbol| symbol.terminals()[0]).chain(symbols.last().map(|symbol| symbol.terminals()[1]));
    for terminal in ends {
        let at = project.terminal_position(terminal).unwrap();
        let below = json!({"x_nm":at.x_nm,"y_nm":at.y_nm + 4 * GRID_NM});
        session.apply(DRAW_WIRE, &json!({"from":terminal.inspect(),"to":below,"bends":[],"expected_revision":session.revision()})).unwrap();
    }
}

/// Copy everything on the first sheet, which holds exactly one row at this point.
fn copy_row(session: &mut Session) -> String {
    let project = session.project().unwrap();
    let items = Selection {
        symbols: project.symbols().iter().map(|symbol| symbol.id()).collect(),
        wires: project.wires().iter().map(|wire| wire.id()).collect(),
        ..Selection::default()
    };
    let copied = session.execute(clipboard::COPY, &json!({"items":items,"expected_revision":session.revision()})).unwrap();
    copied["clipboard"].as_str().unwrap().to_owned()
}

/// Paste the copied row onto `sheet` with its first resistor at the start of `row`.
fn paste(session: &mut Session, text: &str, sheet: SheetTarget, row: usize) -> Result<u64, crate::CommandError> {
    let params = json!({"text":text,"sheet":sheet.sheet.to_string(),"x_nm":row_x(0),"y_nm":row_y(row),"expected_revision":session.revision()});
    session.apply(clipboard::PASTE, &params)
}

/// Wires on all sheets together, the count the project-wide wire limit applies to.
fn wire_total(session: &Session) -> usize {
    let project = session.project().unwrap();
    project.sheets().map(|sheet| project.for_sheet(sheet.target).unwrap().wires().len()).sum()
}

/// A project with exactly the symbol and wire limits, spread over four sheets, saves as native
/// documents and as a packed file and reopens unchanged. One more symbol, wire or pasted row is
/// refused and leaves the project as it was.
#[test]
fn schematic_at_the_symbol_and_wire_limits_saves_and_reloads() {
    assert_eq!((SHEETS * ROWS * ROW, SHEETS * ROWS * ROW * 2), (MAX_SYMBOLS, MAX_WIRES));
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Schematic capacity"})).unwrap();
    first_row(&mut session);
    let text = copy_row(&mut session);
    for number in 2..=SHEETS {
        session.apply(CREATE_SHEET, &json!({"name":format!("Sheet {number}"),"expected_revision":session.revision()})).unwrap();
    }
    let sheets: Vec<_> = session.project().unwrap().sheets().map(|sheet| sheet.target).collect();
    for (index, &sheet) in sheets.iter().enumerate() {
        let first = if index == 0 { 1 } else { 0 };
        for row in first..ROWS {
            paste(&mut session, &text, sheet, row).unwrap();
        }
    }

    let project = session.snapshot().unwrap();
    assert_eq!(project.all_symbols().count(), MAX_SYMBOLS);
    assert_eq!(project.components().count(), MAX_SYMBOLS);
    assert_eq!(wire_total(&session), MAX_WIRES);
    for &sheet in &sheets {
        let projection = project.for_sheet(sheet).unwrap();
        assert_eq!((projection.symbols().len(), projection.wires().len()), (ROWS * ROW, ROWS * ROW * 2));
    }
    // Each row has one net per chain link plus the two outer pins: ROW + 1 nets, all separate.
    let nets = project.electrical_connectivity().nets.iter().filter(|net| !net.is_empty()).count();
    assert_eq!(nets, SHEETS * ROWS * (ROW + 1));

    let documents = encode_documents(project.clone()).unwrap();
    let largest = documents.values().map(Vec::len).max().unwrap();
    let total: usize = documents.values().map(Vec::len).sum();
    println!("Schematic capacity: largest document {largest} bytes; all documents {total} bytes");
    assert!(largest <= MAX_DOCUMENT_BYTES && total <= MAX_DOCUMENTS_BYTES, "largest {largest}, total {total}");
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    assert_eq!(reopened.all_symbols().count(), MAX_SYMBOLS);
    let packed = decode_packed(&encode_packed(project.clone()).unwrap()).unwrap();
    assert_eq!(encode_documents(packed.project).unwrap(), documents);
    let mut reloaded = Session::default();
    reloaded.open_snapshot(reopened);
    assert_eq!(encode_documents(reloaded.snapshot().unwrap()).unwrap(), documents);

    // Add one otherwise valid free wire to the command-built project. Every sheet remains
    // below the wire limit, so opening must count all sheets rather than just the largest.
    let mut oversized = (*project).clone();
    let first_id = oversized.allocation_high_water() + 1;
    let sheet = oversized.sheet_mut();
    Arc::make_mut(&mut sheet.junctions).insert(first_id, Point { x_nm: 0, y_nm: row_y(ROWS) });
    Arc::make_mut(&mut sheet.junctions).insert(first_id + 1, Point { x_nm: GRID_NM, y_nm: row_y(ROWS) });
    Arc::make_mut(&mut sheet.wires).push(Wire {
        ordinal: first_id + 2,
        endpoints: [WireEndpoint::Junction(first_id), WireEndpoint::Junction(first_id + 1)],
        bends: Some(Vec::new()),
    });
    oversized.numeric_id_high_water = Some(first_id + 2);
    let oversized_documents = encode_documents(Arc::new(oversized)).unwrap();
    let error = decode_documents(&oversized_documents).unwrap_err();
    assert!(error.contains("symbol or wire limit"), "{error}");

    let before = session.snapshot().unwrap();
    let revision_before = session.revision();
    let sheet = sheets[0].sheet.to_string();
    let symbol =
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","sheet":sheet,"x_nm":0,"y_nm":row_y(ROWS),"expected_revision":revision_before}));
    assert!(matches!(symbol, Err(crate::CommandError::CapExceeded(ref reason)) if reason.contains(&format!("{MAX_SYMBOLS} symbols"))));
    let wire = session.apply(
        DRAW_WIRE,
        &json!({"sheet":sheet,"from":{"x_nm":0,"y_nm":row_y(ROWS)},"to":{"x_nm":GRID_NM,"y_nm":row_y(ROWS)},"bends":[],"expected_revision":revision_before}),
    );
    assert!(matches!(wire, Err(crate::CommandError::CapExceeded(ref reason)) if reason.contains("wire limit")));
    assert!(
        matches!(paste(&mut session, &text, sheets[0], ROWS), Err(crate::CommandError::CapExceeded(ref reason)) if reason == "pasted objects exceed project limits")
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision_before);
}

/// A sheet holds exactly the net-label limit, built from pasted columns, and saves and reloads.
/// The next label on that sheet is refused by the command and by the check documents are read
/// with, while another sheet still takes a label and a pasted column.
#[test]
fn sheet_at_the_net_label_limit_saves_and_reloads() {
    const COLUMN: usize = 64;
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Label capacity"})).unwrap();
    for index in 0..COLUMN {
        let params =
            json!({"name":format!("N{index}"),"style":"local","x_nm":0,"y_nm":index as i64 * 2 * GRID_NM,"expected_revision":session.revision()});
        session.apply(net_labels::ADD, &params).unwrap();
    }
    let project = session.project().unwrap();
    let items = Selection { net_labels: project.net_labels().iter().map(|label| label.object_id()).collect(), ..Selection::default() };
    let copied = session.execute(clipboard::COPY, &json!({"items":items,"expected_revision":session.revision()})).unwrap();
    let text = copied["clipboard"].as_str().unwrap().to_owned();
    let paste_column = |session: &mut Session, column: usize, suffix: &str| {
        let params = json!({"text":text,"x_nm":column as i64 * 5 * GRID_NM,"y_nm":0,"net_name_suffix":suffix,"expected_revision":session.revision()});
        session.apply(clipboard::PASTE, &params)
    };
    for column in 1..MAX_LABELS / COLUMN {
        paste_column(&mut session, column, &format!("_{column}")).unwrap();
    }
    assert_eq!(session.project().unwrap().net_labels().len(), MAX_LABELS);

    let project = session.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    assert_eq!(reopened.net_labels().len(), MAX_LABELS);
    let mut over = (*project).clone();
    let mut label = over.net_labels()[0].clone();
    label.id = over.allocation_high_water() + 1;
    label.at = Point { x_nm: -20 * GRID_NM, y_nm: 0 };
    Arc::make_mut(&mut over.sheet_mut().net_labels).push(label);
    assert_eq!(over.validate_net_labels().unwrap_err(), "sheet exceeds the named-label limit");

    let before = session.snapshot().unwrap();
    let extra = json!({"name":"EXTRA","style":"local","x_nm":-20 * GRID_NM,"y_nm":0,"expected_revision":session.revision()});
    let refused = session.apply(net_labels::ADD, &extra).unwrap_err();
    assert!(matches!(refused, crate::CommandError::CapExceeded(ref reason) if reason.contains(&format!("{MAX_LABELS} named labels"))), "{refused}");
    assert!(
        matches!(paste_column(&mut session, MAX_LABELS / COLUMN, "_extra"), Err(crate::CommandError::CapExceeded(ref reason)) if reason == "pasted objects exceed project limits")
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));

    session.apply(CREATE_SHEET, &json!({"name":"Second","expected_revision":session.revision()})).unwrap();
    let second = session.project().unwrap().sheets().nth(1).unwrap().target;
    let mut extra = extra;
    extra["sheet"] = Value::String(second.sheet.to_string());
    extra["expected_revision"] = json!(session.revision());
    session.apply(net_labels::ADD, &extra).unwrap();
    // A pasted column counts against the sheet it lands on, not the full first sheet.
    let params =
        json!({"text":text,"sheet":second.sheet.to_string(),"x_nm":0,"y_nm":0,"net_name_suffix":"_second","expected_revision":session.revision()});
    session.apply(clipboard::PASTE, &params).unwrap();
    let project = session.snapshot().unwrap();
    assert_eq!(project.net_labels().len(), MAX_LABELS);
    assert_eq!(project.for_sheet(second).unwrap().net_labels().len(), COLUMN + 1);
    let documents = encode_documents(project).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// `count` well-formed identities with `prefix`, numbered from 1.
fn ids(prefix: &str, count: usize) -> Value {
    json!((1..=count).map(|id| format!("{prefix}{id}")).collect::<Vec<_>>())
}

/// The selection, part and footprint identity lists accept a list at the symbol, wire or label
/// limit and refuse one more entry, and their published schemas state the same bounds.
#[test]
fn identity_lists_accept_the_limits_and_refuse_one_more() {
    let selection =
        |symbols, wires, labels| json!({"symbols":ids("symbol-", symbols),"wires":ids("wire-", wires),"net_labels":ids("net-label-", labels)});
    assert!(Selection::parse(&selection(MAX_SYMBOLS, MAX_WIRES, MAX_LABELS)).is_some());
    assert!(Selection::parse(&selection(MAX_SYMBOLS + 1, 0, 0)).is_none());
    assert!(Selection::parse(&selection(0, MAX_WIRES + 1, 0)).is_none());
    assert!(Selection::parse(&selection(0, 0, MAX_LABELS + 1)).is_none());
    let schema = crate::ParamKind::Selection.schema();
    let bounds = ["symbols", "wires", "net_labels"].map(|key| schema["properties"][key]["maxItems"].as_u64().unwrap() as usize);
    assert_eq!(bounds, [MAX_SYMBOLS, MAX_WIRES, MAX_LABELS]);
    for (kind, prefix, reason) in [
        (crate::ParamKind::PartIds, "component-", "must contain unique component IDs within the symbol limit"),
        (crate::ParamKind::FootprintIds, "footprint-", "must contain unique placed footprint IDs within the symbol limit"),
    ] {
        assert_eq!(kind.validate(&ids(prefix, MAX_SYMBOLS)), Ok(()));
        assert_eq!(kind.validate(&ids(prefix, MAX_SYMBOLS + 1)), Err(reason));
        assert_eq!(kind.schema()["maxItems"].as_u64(), Some(MAX_SYMBOLS as u64));
    }
}
