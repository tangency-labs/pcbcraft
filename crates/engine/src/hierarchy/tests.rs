use super::*;

/// Submit authored changes through the same revision-checked path as GUI and MCP.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Parent pins and child labels join across documents; text alone never joins siblings.
#[test]
fn hierarchy_connects_explicit_interfaces_and_round_trips() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let parent = s.project().unwrap().sheet_target();
    apply(&mut s, CREATE_CHILD, json!({"name":"Power","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"SIGNAL","direction":"input","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().for_sheet(child).unwrap().labels[0].id;
    for sheet in [parent, child] {
        apply(&mut s, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":10_000_000,"y_nm":0}));
        let projection = s.project().unwrap().for_sheet(sheet).unwrap();
        let terminal = projection.symbols()[0].terminals()[0];
        let endpoint = if sheet == parent { WireEndpoint::SheetPin(label) } else { WireEndpoint::Label(label) };
        let from = projection.terminal_position(terminal).unwrap();
        let to = projection.endpoint_position(endpoint).unwrap();
        let bends = if from.x_nm != to.x_nm && from.y_nm != to.y_nm { vec![Point { x_nm: from.x_nm, y_nm: to.y_nm }] } else { vec![] };
        apply(&mut s, DRAW_WIRE, json!({"sheet":sheet.id(),"from":terminal.inspect(),"to":endpoint.inspect(),"bends":bends,"auto_heal":false}));
    }
    let p = s.snapshot().unwrap();
    assert_eq!(p.electrical_connectivity().nets.iter().filter(|net| net.len() == 2).count(), 1);
    assert_eq!(p.connectivity().nets[0].len(), 1);
    assert_eq!(p.parent_sheet(child), Some(parent));
    let docs = persistence::encode_documents(p.clone()).unwrap();
    let mut incremental = persistence::DocumentsEncoder::new(p.clone());
    let streamed = loop {
        if let Some(done) = incremental.step(1).unwrap() {
            break done;
        }
    };
    assert_eq!(streamed, docs);
    assert!(incremental.step(1).is_err());
    let loaded = persistence::decode_documents(&docs).unwrap();
    assert_eq!(docs, persistence::encode_documents(loaded.clone()).unwrap());
    assert_eq!(loaded.electrical_connectivity().nets, p.electrical_connectivity().nets);
    assert!(persistence::encode(p.clone()).is_err());
    let packed = persistence::encode_packed(p.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_packed(&packed).unwrap().project).unwrap(), docs);
    let block = &p.subsheets[0];
    let destination = Point { x_nm: block.at.x_nm + 1_000_000, y_nm: block.at.y_nm + 2_000_000 };
    let preview = p.moved_hierarchy(&format!("sheet-symbol-{}", block.id), destination).unwrap();
    assert_eq!(preview.wire_path(&preview.wires[0]).last().copied(), preview.endpoint_position(WireEndpoint::SheetPin(label)));
    assert_eq!(p.subsheets[0].at, block.at);
    let old_pin = p.sheet_pins(&p.subsheets[0]).next().unwrap().at;
    apply(&mut s, EDIT_LABEL, json!({"sheet":child.id(),"id":format!("label-{label}"),"name":"RENAMED","direction":"input","x_nm":100,"y_nm":0}));
    let p = s.project().unwrap();
    let pin = p.sheet_pins(&p.subsheets[0]).next().unwrap();
    assert_eq!(pin.label.name, "RENAMED");
    assert_eq!(pin.at, old_pin);
    assert_eq!(p.electrical_connectivity().nets.iter().filter(|net| net.len() == 2).count(), 1);
    let renamed = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let changed: Vec<_> = docs.iter().filter(|(path, bytes)| renamed.get(*path) != Some(*bytes)).map(|(path, _)| path.clone()).collect();
    assert_eq!(changed, vec![format!("sheets/{}.json", child.id())]);
    apply(&mut s, DELETE_LABEL, json!({"sheet":child.id(),"id":format!("label-{label}")}));
    assert!(s.project().unwrap().wires().is_empty());
    assert!(s.project().unwrap().for_sheet(child).unwrap().wires().is_empty());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().electrical_connectivity().nets.iter().filter(|net| net.len() == 2).count(), 1);
}

/// Cycles, duplicate parents, foreign endpoints, stale requests and malformed documents fail atomically.
#[test]
fn rejected_hierarchy_edits_keep_the_exact_snapshot() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, CREATE_CHILD, json!({"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"DATA","direction":"input","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().for_sheet(child).unwrap().labels[0].id;
    let before = s.snapshot().unwrap();
    for (cmd, mut params) in [
        (PLACE_SHEET, json!({"sheet":child.id(),"child":main.id(),"x_nm":0,"y_nm":0})),
        (PLACE_SHEET, json!({"sheet":main.id(),"child":child.id(),"x_nm":0,"y_nm":0})),
        (ADD_LABEL, json!({"sheet":child.id(),"name":"DATA","direction":"output","x_nm":0,"y_nm":0})),
        (DRAW_WIRE, json!({"sheet":main.id(),"from":{"label":format!("label-{label}")},"to":{"x_nm":123,"y_nm":456},"bends":[]})),
    ] {
        params["expected_revision"] = json!(s.revision());
        assert!(s.apply(cmd, &params).is_err(), "{cmd}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let mut docs = persistence::encode_documents(before).unwrap();
    let path = format!("sheets/{}.json", main.id());
    // Invalid external input is constructed from an engine-authored document, never a fixture from another implementation.
    let mut bad: Value = serde_json::from_slice(&docs[&path]).unwrap();
    bad["subsheets"][0]["child"] = json!(main.id());
    docs.insert(path, serde_json::to_vec(&bad).unwrap());
    assert!(persistence::decode_documents(&docs).unwrap_err().contains("cycle"));
    let block = s.project().unwrap().subsheets[0].id;
    apply(&mut s, REMOVE_SHEET, json!({"sheet":main.id(),"id":format!("sheet-symbol-{}",block)}));
    assert!(s.project().unwrap().parent_sheet(child).is_none());
    assert_eq!(s.project().unwrap().for_sheet(child).unwrap().labels.len(), 1);
}

/// Removal preserves neighboring pin coordinates; sheet deletion promotes descendants and
/// removes incoming pin wires atomically, with the complete tree restored by Undo.
#[test]
fn stable_slots_and_child_deletion_preserve_other_documents() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, CREATE_CHILD, json!({"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, CREATE_CHILD, json!({"sheet":child.id(),"name":"Grandchild","x_nm":0,"y_nm":0}));
    let grandchild = s.project().unwrap().sheets().nth(2).unwrap().target;
    for name in ["FIRST", "SECOND"] {
        apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":name,"direction":"input","x_nm":0,"y_nm":0}));
    }
    let child_view = s.project().unwrap().for_sheet(child).unwrap();
    let first = child_view.labels[0].id;
    let second = child_view.labels[1].id;
    let pin = s.project().unwrap().sheet_pins(&s.project().unwrap().subsheets[0]).find(|p| p.label.id == second).unwrap().at;
    apply(&mut s, DELETE_LABEL, json!({"sheet":child.id(),"id":format!("label-{first}")}));
    assert_eq!(s.project().unwrap().sheet_pins(&s.project().unwrap().subsheets[0]).next().unwrap().at, pin);
    apply(
        &mut s,
        DRAW_WIRE,
        json!({"sheet":main.id(),"from":{"sheet_pin":format!("label-{second}")},"to":{"x_nm":pin.x_nm-1_000_000,"y_nm":pin.y_nm},"bends":[],"auto_heal":true}),
    );
    let before = s.snapshot().unwrap();
    apply(&mut s, sheets::DELETE_SHEET, json!({"sheet":child.id()}));
    let p = s.project().unwrap();
    assert!(p.for_sheet(child).is_none());
    assert!(p.parent_sheet(grandchild).is_none());
    assert!(p.sheet_symbols().is_empty());
    assert!(p.wires().is_empty());
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    persistence::decode_documents(&docs).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.project().unwrap().parent_sheet(grandchild), Some(child));
}

/// The same interface name in siblings is not a global label. Only the owning parent's pin
/// links to each child's anchor, and malicious fields cannot escape those ownership checks.
#[test]
fn sibling_names_remain_separate_and_malformed_hierarchy_is_refused() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    for name in ["One", "Two"] {
        apply(&mut s, CREATE_CHILD, json!({"sheet":main.id(),"name":name,"x_nm":0,"y_nm":0}));
        let child = s.project().unwrap().sheets().last().unwrap().target;
        apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"DATA","direction":"input","x_nm":0,"y_nm":0}));
        apply(&mut s, PLACE_RESISTOR, json!({"sheet":child.id(),"x_nm":10_000_000,"y_nm":0}));
        let p = s.project().unwrap().for_sheet(child).unwrap();
        let label = p.labels[0].id;
        let pin = p.symbols()[0].terminals()[0];
        apply(
            &mut s,
            DRAW_WIRE,
            json!({"sheet":child.id(),"from":pin.inspect(),"to":{"label":format!("label-{label}")},"bends":[],"auto_heal":false}),
        );
    }
    assert_eq!(s.project().unwrap().electrical_connectivity().nets.iter().filter(|net| net.len() == 1).count(), 2);
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    let path = format!("sheets/{}.json", child.id());
    let original: Value = serde_json::from_slice(&docs[&path]).unwrap();
    for (field, value) in
        [("id", json!(0)), ("slot", json!(64)), ("at", json!({"x_nm":i64::MAX,"y_nm":0})), ("direction", json!("unknown")), ("name", json!(" DATA "))]
    {
        let mut malformed = docs.clone();
        let mut document = original.clone();
        document["labels"][0][field] = value;
        malformed.insert(path.clone(), serde_json::to_vec(&document).unwrap());
        assert!(persistence::decode_documents(&malformed).is_err(), "{field}");
    }
    // Parent coordinate arithmetic must also reject hostile values without panicking.
    let parent_path = format!("sheets/{}.json", main.id());
    let mut malformed = docs.clone();
    let mut document: Value = serde_json::from_slice(&docs[&parent_path]).unwrap();
    document["subsheets"][0]["at"]["x_nm"] = json!(i64::MAX);
    malformed.insert(parent_path, serde_json::to_vec(&document).unwrap());
    assert!(persistence::decode_documents(&malformed).is_err());
}

/// Authored geometry, right-side pin contacts and attached routes survive resizing, undo and
/// persistence. Invalid or stale rectangles leave the exact snapshot untouched.
#[test]
fn resize_sheet_is_atomic_persistent_and_preserves_pin_identity() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let parent = s.project().unwrap().sheet_target();
    apply(&mut s, CREATE_CHILD, json!({"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"OUT","direction":"output","x_nm":0,"y_nm":0}));
    let p = s.project().unwrap();
    let id = p.sheet_symbols()[0].id;
    let pin = p.sheet_pins(&p.sheet_symbols()[0]).next().unwrap();
    let end = WireEndpoint::SheetPin(pin.label.id);
    let at = pin.at;
    apply(
        &mut s,
        DRAW_WIRE,
        json!({"sheet":parent.id(),"from":end.inspect(),"to":{"x_nm":at.x_nm+10_000_000,"y_nm":at.y_nm},"bends":[],"auto_heal":false}),
    );
    let before = s.snapshot().unwrap();
    let prior_docs = persistence::encode_documents(before.clone()).unwrap();
    let object = format!("sheet-symbol-{id}");
    let params = json!({"sheet":parent.id(),"id":object,"x_nm":-1_270_000,"y_nm":-2_540_000,"width_nm":30_480_000,"height_nm":20_320_000});
    apply(&mut s, RESIZE_SHEET, params.clone());
    let resized = s.snapshot().unwrap();
    assert_eq!(resized.endpoint_position(end), Some(Point { x_nm: 29_210_000, y_nm: 2_540_000 }));
    assert_eq!(resized.wires()[0].endpoints()[0], end);
    assert_eq!(resized.wire_path(&resized.wires()[0])[0], resized.endpoint_position(end).unwrap());
    assert_eq!(resized.connectivity().nets, before.connectivity().nets);
    let docs = persistence::encode_documents(resized.clone()).unwrap();
    assert_eq!(docs, persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap());
    let changed: Vec<_> = prior_docs.iter().filter(|(path, bytes)| docs.get(*path) != Some(*bytes)).map(|(path, _)| path.clone()).collect();
    assert_eq!(changed, vec![format!("sheets/{}.json", parent.id())]);
    let revision = s.revision();
    apply(&mut s, RESIZE_SHEET, params.clone());
    assert_eq!(revision, s.revision());
    for (field, value) in [("width_nm", json!(1)), ("height_nm", json!(1)), ("width_nm", json!(i64::MAX)), ("x_nm", json!(MAX_CENTER_NM))] {
        let mut bad = params.clone();
        bad[field] = value;
        bad["expected_revision"] = json!(s.revision());
        assert!(s.apply(RESIZE_SHEET, &bad).is_err());
        assert!(Arc::ptr_eq(&resized, &s.snapshot().unwrap()));
    }
    let mut stale = params.clone();
    stale["expected_revision"] = json!(0);
    assert!(s.apply(RESIZE_SHEET, &stale).is_err());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    // Existing sheet/2 records omit size and retain their exact canonical bytes.
    assert_eq!(prior_docs, persistence::encode_documents(persistence::decode_documents(&prior_docs).unwrap()).unwrap());
    let path = format!("sheets/{}.json", parent.id());
    for size in [Value::Null, json!({"x_nm":-1,"y_nm":1}), json!({"x_nm":i64::MAX,"y_nm":20_320_000})] {
        let mut bad_docs = docs.clone();
        let mut sheet: Value = serde_json::from_slice(&bad_docs[&path]).unwrap();
        sheet["subsheets"][0]["size"] = size;
        bad_docs.insert(path.clone(), serde_json::to_vec(&sheet).unwrap());
        assert!(persistence::decode_documents(&bad_docs).is_err());
    }
}
