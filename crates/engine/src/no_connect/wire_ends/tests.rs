use super::*;
use crate::{DRAW_WIRE, MOVE_JUNCTION, PLACE_SYMBOL, persistence};

/// Original resistor plus one authored open wire; both the pin and wire endpoint are real IDs.
fn stub() -> (Session, WireTarget) {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    s.apply(DRAW_WIRE, &json!({"from":pin.inspect(),"to":{"x_nm":-25_400_000,"y_nm":0},"bends":[],"expected_revision":s.revision()})).unwrap();
    let target = s.project().unwrap().wire_end_targets().next().unwrap().0;
    (s, target)
}

/// Use the public revision-checked command rather than mutating fixture data.
fn mark(s: &mut Session, target: &WireTarget, marked: bool) {
    s.apply(SET, &json!({"target":target,"marked":marked,"expected_revision":s.revision()})).unwrap();
}

/// A flag silences only its end's dangling warning, retaining unrelated unconnected pins and
/// the actual netlist. Invalid/stale commands are atomic; no-op, undo and redo retain identity.
#[test]
fn wire_no_connect_intent_controls_only_dangling_findings() {
    let (mut s, target) = stub();
    let before = s.snapshot().unwrap();
    let before_graph = before.electrical_connectivity().inspect();
    let report = before.check_no_connects(s.revision());
    assert_eq!(report.wire_findings.len(), 1);
    assert_eq!(report.wire_findings[0].rule, "dangling_wire");
    mark(&mut s, &target, true);
    let marked = s.snapshot().unwrap();
    let report = marked.check_no_connects(s.revision());
    assert_eq!(report.wire_markers.len(), 1);
    assert_eq!(report.wire_markers[0].target, target);
    assert!(report.wire_markers[0].issues.is_empty());
    assert!(report.wire_findings.is_empty());
    assert_eq!(report.findings.len(), 1, "the other unconnected resistor pin still needs attention");
    assert_eq!(marked.electrical_connectivity().inspect(), before_graph);
    let revision = s.revision();
    mark(&mut s, &target, true);
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&marked, &s.snapshot().unwrap()));
    for params in [
        json!({"target":target,"marked":false,"expected_revision":revision-1}),
        json!({"target":{"junction":"junction-999999"},"marked":true,"expected_revision":revision}),
        json!({"target":{"junction":target.junction,"extra":true},"marked":true,"expected_revision":revision}),
    ] {
        assert!(s.apply(SET, &params).is_err());
        assert!(Arc::ptr_eq(&marked, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &json!({})).unwrap();
    crate::assert_same_content(&marked, &s.snapshot().unwrap());
}

/// Sheet/8 carries stable wire-end IDs together with project/19, with exact native round trips,
/// rejection of old headers/null/duplicates/foreign ends, and deterministic marker removal.
#[test]
fn wire_no_connect_storage_roundtrips_and_rejects_invalid_records() {
    let (mut s, target) = stub();
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    mark(&mut s, &target, true);
    let p = s.snapshot().unwrap();
    assert!(persistence::encode(p.clone()).is_err());
    let docs = persistence::encode_documents(p.clone()).unwrap();
    let sheet_path = docs.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();
    let sheet: Value = serde_json::from_slice(&docs[&sheet_path]).unwrap();
    assert_eq!(sheet["format"], "pcbcraft-sheet/8");
    let entry: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/19");
    let folder = persistence::decode_documents(&docs).unwrap();
    let archive = persistence::decode_packed(&persistence::encode_packed(p).unwrap()).unwrap().project;
    for reopened in [folder, archive] {
        assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), docs);
        s.open_snapshot(reopened);
        assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_markers[0].target, target);
        mark(&mut s, &target, false);
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap()[&sheet_path], before[&sheet_path]);
    }
    for records in [Value::Null, json!([]), json!([target.ordinal().unwrap(), target.ordinal().unwrap()]), json!([999999])] {
        let mut bad = docs.clone();
        let mut value = sheet.clone();
        value["no_connect_ends"] = records;
        bad.insert(sheet_path.clone(), serde_json::to_vec(&value).unwrap());
        assert!(persistence::decode_documents(&bad).is_err());
    }
    let mut old = docs.clone();
    let mut value = sheet;
    value["format"] = json!("pcbcraft-sheet/7");
    old.insert(sheet_path.clone(), serde_json::to_vec(&value).unwrap());
    assert!(persistence::decode_documents(&old).is_err());
}

/// The marker follows moved endpoints and copied IDs. Extending a marked end preserves its
/// conflict instead of silently healing it away; deleting its owning wire removes the marker.
#[test]
fn wire_no_connect_moves_copies_and_preserves_connection_conflicts() {
    let (mut s, target) = stub();
    mark(&mut s, &target, true);
    s.apply(MOVE_JUNCTION, &json!({"junction":target.junction,"to":{"x_nm":-38_100_000,"y_nm":0},"expected_revision":s.revision()})).unwrap();
    let p = s.snapshot().unwrap();
    assert_eq!(p.check_no_connects(s.revision()).wire_markers[0].appearance.at.x_nm, -38_100_000);
    let wire = p.wires()[0].id();
    let text = crate::clipboard::Fragment::capture(&p, &crate::selection::Selection::one_wire(wire.clone())).unwrap().to_text().unwrap();
    s.apply(crate::clipboard::PASTE, &json!({"text":text,"x_nm":0,"y_nm":25_400_000,"rotation":90,"expected_revision":s.revision()})).unwrap();
    let report = s.project().unwrap().check_no_connects(s.revision());
    assert_eq!(report.wire_markers.len(), 2);
    assert_ne!(report.wire_markers[0].target, report.wire_markers[1].target);
    assert_eq!(report.wire_markers[0].target, target);
    s.apply("edit.undo", &json!({})).unwrap();
    s.apply(DRAW_WIRE, &json!({"from":{"junction":target.junction},"to":{"x_nm":-50_800_000,"y_nm":0},"bends":[],"expected_revision":s.revision()}))
        .unwrap();
    let report = s.project().unwrap().check_no_connects(s.revision());
    assert_eq!(report.wire_markers.len(), 1);
    assert_eq!(report.wire_markers[0].issues, ["no_connect_not_dangling"]);
    assert!(report.wire_findings.iter().any(|finding| finding.rule == "no_connect_not_dangling"));
    let extended = s.snapshot().unwrap();
    let pin = extended.symbols()[0].terminals()[0];
    let pin_at = extended.terminal_position(pin).unwrap();
    let far = extended.wire_end_targets().next().unwrap().0;
    // A preferred redraw must not erase the older route through a marked transit point.
    s.apply(
        DRAW_WIRE,
        &json!({"from":pin.inspect(),"to":{"junction":far.junction},
        "bends":[{"x_nm":pin_at.x_nm,"y_nm":12_700_000},{"x_nm":-50_800_000,"y_nm":12_700_000}],
        "expected_revision":s.revision()}),
    )
    .unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_markers[0].target, target);
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_markers[0].issues, ["no_connect_not_dangling"]);
    s.apply("edit.undo", &json!({})).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    s.apply(crate::DELETE_WIRE, &json!({"wire":wire,"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_markers.len(), 0);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_markers.len(), 1);
}

/// An end can belong to a named net without making the physical endpoint cease to be free.
/// Only a label exactly on the end supplies its own termination; naming does not hide an open end.
#[test]
fn wire_no_connect_named_net_and_open_end_remain_independent() {
    let (mut s, target) = stub();
    let pin_at = s.project().unwrap().symbols()[0].pins()[0];
    s.apply(crate::net_labels::ADD, &json!({"name":"SPARE","style":"global","x_nm":pin_at.x_nm,"y_nm":pin_at.y_nm,"expected_revision":s.revision()}))
        .unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).wire_findings.len(), 1);
    let before = s.snapshot().unwrap();
    let graph = before.electrical_connectivity().inspect();
    mark(&mut s, &target, true);
    let after = s.snapshot().unwrap();
    assert!(after.check_no_connects(s.revision()).wire_markers[0].issues.is_empty());
    assert_eq!(after.electrical_connectivity().inspect(), graph);
    mark(&mut s, &target, false);
    s.apply(crate::net_labels::ADD, &json!({"name":"SPARE","style":"global","x_nm":-25_400_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    assert!(s.project().unwrap().check_no_connects(s.revision()).wire_findings.is_empty());
}

/// Root routes and every descendant retain their own marker IDs when an entire subtree is
/// duplicated. Ownership is remapped independently and remains exact through save and undo.
#[test]
fn wire_no_connect_hierarchy_clipboard_preserves_root_and_descendant_intent() {
    let (mut s, root_target) = stub();
    mark(&mut s, &root_target, true);
    let root = s.project().unwrap().sheet_target();
    s.apply(crate::hierarchy::CREATE_CHILD, &json!({"sheet":root.id(),"name":"Child","x_nm":50_800_000,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    s.apply(
        DRAW_WIRE,
        &json!({"sheet":child.id(),"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":25_400_000,"y_nm":0},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    let child_target = s.project().unwrap().for_sheet(child).unwrap().wire_end_targets().next().unwrap().0;
    mark(&mut s, &child_target, true);
    let p = s.snapshot().unwrap();
    let block = p.subsheets[0].id;
    let copied = s.execute(crate::clipboard::COPY, &json!({"sheet":root.id(),"items":crate::selection::Selection::one_wire(p.wires()[0].id()),"objects":[format!("sheet-symbol-{block}")],"expected_revision":s.revision()})).unwrap();
    s.apply(
        crate::clipboard::PASTE,
        &json!({"sheet":root.id(),"text":copied["clipboard"],"x_nm":0,"y_nm":50_800_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let after = s.snapshot().unwrap();
    let report = after.check_no_connects(s.revision());
    assert_eq!(report.wire_markers.len(), 4);
    let ids: BTreeSet<_> = report.wire_markers.iter().map(|marker| &marker.target.junction).collect();
    assert_eq!(ids.len(), 4);
    assert!(report.wire_markers.iter().all(|marker| marker.issues.is_empty()));
    assert_eq!(report.wire_markers.iter().filter(|marker| marker.appearance.sheet == root).count(), 2);
    let docs = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
    // A valid junction owned by another sheet must not be accepted as a local attachment.
    let root_path = format!("sheets/{}.json", root.id());
    let mut foreign = docs.clone();
    let mut value: Value = serde_json::from_slice(&foreign[&root_path]).unwrap();
    value["no_connect_ends"] = json!([child_target.ordinal().unwrap()]);
    foreign.insert(root_path, serde_json::to_vec(&value).unwrap());
    assert!(persistence::decode_documents(&foreign).is_err());
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&p, &s.snapshot().unwrap());
    s.apply("edit.redo", &json!({})).unwrap();
    crate::assert_same_content(&after, &s.snapshot().unwrap());
}

/// Pin and wire markers cannot hide existing physical board violations or change its netlist.
#[test]
fn wire_no_connect_and_pin_intent_preserve_physical_drc_findings() {
    let mut s = crate::board::track::tests::routed_fixture();
    s.apply(
        DRAW_WIRE,
        &json!({"from":{"x_nm":-100_000_000,"y_nm":0},"to":{"x_nm":-80_000_000,"y_nm":0},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    let wire_target = before.wire_end_targets().next().unwrap().0;
    let terminal = before.symbols()[0].terminals()[0];
    let pin_target = before.no_connect_target(terminal).unwrap();
    let board = before.board_target().unwrap();
    let drc = before.board_view(board).unwrap().check_design(Default::default());
    assert!(drc.total > 0);
    let graph = before.electrical_connectivity().inspect();
    mark(&mut s, &wire_target, true);
    s.apply(SET, &json!({"target":pin_target,"marked":true,"expected_revision":s.revision()})).unwrap();
    let after = s.snapshot().unwrap();
    assert_eq!(after.electrical_connectivity().inspect(), graph);
    assert_eq!(after.board_view(board).unwrap().check_design(Default::default()), drc);
    assert_eq!(after.check_no_connects(s.revision()).markers[0].issues, ["no_connect_connected"]);
}
