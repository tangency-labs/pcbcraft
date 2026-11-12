use super::*;
use crate::{
    SheetTarget, hierarchy as model,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Use only shared native commands to create the nested circuit and unrelated sibling.
fn apply(s: &mut Session, id: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(id, &params).unwrap();
}
/// Connect explicit hierarchy endpoints using the same orthogonal authored path as the canvas.
fn connect(s: &mut Session, sheet: SheetTarget, from: WireEndpoint, to: WireEndpoint) {
    let p = s.project().unwrap().for_sheet(sheet).unwrap();
    let a = p.endpoint_position(from).unwrap();
    let b = p.endpoint_position(to).unwrap();
    let bends = if a.x_nm != b.x_nm && a.y_nm != b.y_nm { vec![Point { x_nm: a.x_nm, y_nm: b.y_nm }] } else { vec![] };
    apply(s, crate::DRAW_WIRE, json!({"sheet":sheet.id(),"from":from.inspect(),"to":to.inspect(),"bends":bends,"auto_heal":false}));
}
/// Root → Power → Sense has two explicit inter-sheet nets and pinned package intent; Other is
/// an unrelated root with a part that must never enter the selection's portable subtree.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Hierarchy clipboard"})).unwrap();
    let root = s.project().unwrap().sheet_target();
    apply(&mut s, model::CREATE_CHILD, json!({"sheet":root.id(),"name":"Power","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().find(|p| p.name == "Power").unwrap().target;
    apply(&mut s, model::CREATE_CHILD, json!({"sheet":child.id(),"name":"Sense","x_nm":20*crate::schematic::GRID_NM,"y_nm":0}));
    let grand = s.project().unwrap().sheets().find(|p| p.name == "Sense").unwrap().target;
    apply(&mut s, crate::sheets::CREATE_SHEET, json!({"name":"Other"}));
    let other = s.project().unwrap().sheets().find(|p| p.name == "Other").unwrap().target;
    for sheet in [root, child, grand, other] {
        apply(&mut s, crate::PLACE_SYMBOL, json!({"sheet":sheet.id(),"definition":"resistor","x_nm":10*crate::schematic::GRID_NM,"y_nm":0}));
    }
    for (sheet, name) in [(child, "VIN"), (grand, "INNER")] {
        apply(&mut s, model::ADD_LABEL, json!({"sheet":sheet.id(),"name":name,"direction":"input","x_nm":0,"y_nm":0}));
    }
    let pin = s.project().unwrap().for_sheet(child).unwrap().labels[0].id;
    let grandpin = s.project().unwrap().for_sheet(grand).unwrap().labels[0].id;
    let terminals = |sheet| s.project().unwrap().for_sheet(sheet).unwrap().symbols()[0].terminals();
    let (r, c, g) = (terminals(root), terminals(child), terminals(grand));
    connect(&mut s, root, WireEndpoint::Terminal(r[0]), WireEndpoint::SheetPin(pin));
    connect(&mut s, child, WireEndpoint::Terminal(c[0]), WireEndpoint::Label(pin));
    connect(&mut s, child, WireEndpoint::Terminal(c[1]), WireEndpoint::SheetPin(grandpin));
    connect(&mut s, grand, WireEndpoint::Terminal(g[0]), WireEndpoint::Label(grandpin));
    let component = s.project().unwrap().for_sheet(child).unwrap().symbols()[0].component;
    apply(
        &mut s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{component}"),"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"}}),
    );
    s
}
/// Select the root block and optionally the root's part/route, independently of child contents.
fn selection(s: &Session, flat: bool) -> Value {
    let p = s.project().unwrap();
    json!({"items":if flat {json!({"symbols":p.symbols().iter().map(|s|s.id()).collect::<Vec<_>>(),"wires":p.wires().iter().map(|w|w.id()).collect::<Vec<_>>()})}else{json!(Selection::default())},"objects":[format!("sheet-symbol-{}",p.subsheets[0].id)],"sheet":p.sheet_target().id(),"expected_revision":s.revision()})
}
/// One explicit cursor destination, with caller-selected relocation/reference policy.
fn paste(s: &Session, text: &str, sheet: SheetTarget) -> Value {
    json!({"text":text,"sheet":sheet.id(),"x_nm":50*crate::schematic::GRID_NM,"y_nm":50*crate::schematic::GRID_NM,"expected_revision":s.revision()})
}

/// A closed nested circuit duplicates independently with fresh sheet/component/interface IDs,
/// exact internal connectivity, pinned mappings, one history step and native persistence.
#[test]
fn complete_subtree_copy_preserves_interfaces_and_excludes_siblings() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let high = before.allocation_high_water();
    let text = s.execute(COPY, &selection(&s, true)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let f = Fragment::from_text(&text).unwrap();
    assert!(f.has_hierarchy());
    assert_eq!(f.hierarchy_counts(), (1, 0, 2));
    assert_eq!(f.counts(), (3, 4, 0));
    assert!(!f.project.sheets.iter().any(|p| p.sheet_name == "Other"));
    assert!(f.project.board_target().is_none());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let root = before.sheet_target();
    s.apply(PASTE, &paste(&s, &text, root)).unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.sheets.len(), 6);
    assert_eq!(p.components.len(), 7);
    assert_eq!(p.subsheets.len(), 2);
    let new_block = &p.subsheets[1];
    let instances = p.sheet_instances();
    assert_eq!(instances.sheet_occurrences.len(), p.sheets.len());
    assert_eq!(instances.sheet_occurrences.iter().map(|occurrence| occurrence.id).collect::<BTreeSet<_>>().len(), p.sheets.len());
    assert!(instances.sheet_occurrences.iter().all(|occurrence| occurrence.circuit_use == before.sheet_instances().circuit_uses[0].id));
    assert!(
        instances
            .sheet_occurrences
            .iter()
            .filter(|occurrence| !before.sheets.iter().any(|sheet| sheet.sheet_id == occurrence.definition))
            .all(|occurrence| !before.sheet_instances().sheet_occurrences.iter().any(|old| old.id == occurrence.id))
    );
    assert_ne!(new_block.child, before.subsheets[0].child);
    assert!(new_block.id > high);
    let child = p.for_sheet(SheetTarget { project: p.id(), sheet: new_block.child }).unwrap();
    assert!(child.labels[0].id > high);
    assert_eq!(child.labels[0].slot, 0);
    assert!(child.symbols()[0].component > high);
    assert_eq!(p.electrical_connectivity().nets.iter().filter(|n| n.len() == 2).count(), 4);
    assert!(
        p.electrical_connectivity()
            .nets
            .iter()
            .filter(|n| n.len() == 2)
            .all(|n| n.iter().all(|t| t.symbol > high) || n.iter().all(|t| t.symbol <= high))
    );
    let package = &p.schematic_packages.assignments[&child.symbols()[0].component];
    assert_eq!(package.mapping["1"], "2");
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(bytes, encode_documents(decode_documents(&bytes).unwrap()).unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// A hierarchy Cut retains child content and the exact unselected parent route at its old
/// coordinates; relocation to another root reuses current sheet/component identities.
#[test]
fn cut_tree_relocation_preserves_original_children_and_dangling_parent_routes() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let child = before.sheets().find(|p| p.name == "Power").unwrap().target;
    let other = before.sheets().find(|p| p.name == "Other").unwrap().target;
    let path = before.wire_path(&before.wires()[0]);
    let text = s.execute(CUT, &selection(&s, false)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let cut = s.snapshot().unwrap();
    assert!(cut.subsheets.is_empty());
    assert_eq!(cut.parent_sheet(child), None);
    assert_eq!(cut.wire_path(&cut.wires()[0]), path);
    assert_eq!(cut.components.len(), 4);
    let old_label = cut.for_sheet(child).unwrap().labels[0].clone();
    apply(
        &mut s,
        model::EDIT_LABEL,
        json!({"sheet":child.id(),"id":format!("label-{}",old_label.id),"name":"EDITED_AFTER_CUT","direction":"input","x_nm":old_label.at.x_nm,"y_nm":old_label.at.y_nm}),
    );
    let component = s.project().unwrap().for_sheet(child).unwrap().symbols()[0].component;
    apply(
        &mut s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{component}"),"library":"starter",
        "definition":"resistor-0603","definition_revision":4,"mapping":{"1":"1","2":"2"}}),
    );
    let cut = s.snapshot().unwrap();
    let mut args = paste(&s, &text, other);
    args["reuse_sheets"] = json!(true);
    s.apply(PASTE, &args).unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.sheets.len(), 4);
    assert_eq!(p.components.len(), 4);
    assert_eq!(p.parent_sheet(child), Some(other));
    assert_eq!(p.sheet_occurrence(child).unwrap().id, before.sheet_occurrence(child).unwrap().id);
    assert_eq!(p.sheet_occurrence(child).unwrap().circuit_use, before.sheet_occurrence(child).unwrap().circuit_use);
    assert_eq!(p.for_sheet(child).unwrap().labels[0].name, "EDITED_AFTER_CUT");
    assert_eq!(p.schematic_packages.assignments[&component].definition, "resistor-0603");
    assert_eq!(p.schematic_packages.assignments[&component].mapping["1"], "1");
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    assert_eq!(p.for_sheet(child).unwrap().symbols()[0].id(), before.for_sheet(child).unwrap().symbols()[0].id());
    assert_eq!(p.wire_path(&p.wires()[0]), path);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&cut, &s.snapshot().unwrap());
}

/// Copies from another saved lineage always clone a new independent tree; reuse cannot steal a
/// parented child, create a cycle, rename current nets or target a foreign project's identity.
#[test]
fn foreign_and_invalid_relocation_leave_the_destination_unchanged() {
    let mut source = fixture();
    let text = source.execute(CUT, &selection(&source, false)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let p = source.snapshot().unwrap();
    let child = p.sheets().find(|p| p.name == "Power").unwrap().target;
    let before = source.snapshot().unwrap();
    let mut args = paste(&source, &text, child);
    args["reuse_sheets"] = json!(true);
    assert!(source.apply(PASTE, &args).is_err());
    assert!(Arc::ptr_eq(&before, &source.snapshot().unwrap()));
    args = paste(&source, &text, p.sheet_target());
    args["reuse_sheets"] = json!(true);
    args["net_name_suffix"] = json!("_B");
    assert!(source.apply(PASTE, &args).is_err());
    source.apply("edit.undo", &Value::Null).unwrap();
    let before = source.snapshot().unwrap();
    args = paste(&source, &text, before.sheet_target());
    args["reuse_sheets"] = json!(true);
    assert!(source.apply(PASTE, &args).is_err());
    assert!(Arc::ptr_eq(&before, &source.snapshot().unwrap()));
    let mut target = Session::default();
    target.apply("project.new", &json!({})).unwrap();
    let root = target.project().unwrap().sheet_target();
    let before = target.snapshot().unwrap();
    args = paste(&target, &text, root);
    args["reuse_sheets"] = json!(true);
    assert!(target.apply(PASTE, &args).is_err());
    assert!(Arc::ptr_eq(&before, &target.snapshot().unwrap()));
    target.apply(PASTE, &paste(&target, &text, root)).unwrap();
    assert_eq!(target.project().unwrap().sheets.len(), 3);
    assert_eq!(target.project().unwrap().components.len(), 2);
}

/// Interface Cut detaches unselected wires on both parent and child sides, and a pasted label
/// receives a new identity and a free side-local slot without moving existing parent pins.
#[test]
fn interface_labels_copy_cut_and_allocate_slots_without_parent_wire_loss() {
    let mut s = fixture();
    let root = s.project().unwrap().sheet_target();
    let child = s.project().unwrap().sheets().find(|p| p.name == "Power").unwrap().target;
    let p = s.project().unwrap().for_sheet(child).unwrap();
    let label = p.labels[0].id;
    let child_path = p.wire_path(&p.wires()[0]);
    let parent_path = s.project().unwrap().wire_path(&s.project().unwrap().wires()[0]);
    let text = s
        .execute(CUT, &json!({"sheet":child.id(),"items":Selection::default(),"objects":[format!("label-{label}")],"expected_revision":s.revision()}))
        .unwrap()["clipboard"]
        .as_str()
        .unwrap()
        .to_owned();
    let p = s.project().unwrap();
    assert_eq!(p.wire_path(&p.wires()[0]), parent_path);
    let child_p = p.for_sheet(child).unwrap();
    assert_eq!(child_p.wire_path(&child_p.wires()[0]), child_path);
    assert!(child_p.labels.is_empty());
    apply(&mut s, model::ADD_LABEL, json!({"sheet":root.id(),"name":"EXISTING","direction":"input","x_nm":0,"y_nm":0}));
    let old = s.project().unwrap().labels[0].clone();
    s.apply(PASTE, &paste(&s, &text, root)).unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.labels.len(), 2);
    assert_eq!(p.labels[0], old);
    assert_ne!(p.labels[1].id, label);
    assert_eq!(p.labels[1].slot, 1);
}

/// Resource limits, rotation, foreign selection IDs and extra root documents fail without any
/// partial import or accepted board update; a tree cannot masquerade as a legacy flat envelope.
#[test]
fn malformed_and_unsupported_hierarchy_clipboards_are_atomic() {
    let mut s = fixture();
    let text = s.execute(COPY, &selection(&s, false)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let before = s.snapshot().unwrap();
    let root = before.sheet_target();
    for extra in [json!({"rotation":90}), json!({"references":"preserve"}), json!({"expected_revision":0})] {
        let mut args = paste(&s, &text, root);
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(s.apply(PASTE, &args).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    envelope["format"] = json!(FORMAT);
    assert!(Fragment::from_text(&envelope.to_string()).is_err());
    envelope["format"] = json!(HIERARCHY_FORMAT);
    envelope["documents"] =
        json!(encode_documents(before.clone()).unwrap().into_iter().map(|(p, b)| (p, String::from_utf8(b).unwrap())).collect::<BTreeMap<_, _>>());
    assert!(Fragment::from_text(&envelope.to_string()).is_err());
    let mut args = selection(&s, false);
    args["objects"] = json!(["sheet-symbol-999999"]);
    assert!(s.execute(COPY, &args).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    while s.project().unwrap().sheets.len() < crate::sheets::MAX_SHEETS {
        let count = s.project().unwrap().sheets.len();
        apply(&mut s, crate::sheets::CREATE_SHEET, json!({"name":format!("Extra {count}")}));
    }
    let before = s.snapshot().unwrap();
    assert!(s.apply(PASTE, &paste(&s, &text, root)).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Cached previews retain transient sheet/object identities while moving only the root symbols
/// and blocks. Descendant geometry stays in its own coordinate system and no history is touched.
#[test]
fn subtree_preview_ids_are_stable_across_cursor_motion() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let text = s.execute(COPY, &selection(&s, true)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let f = Fragment::from_text(&text).unwrap();
    let preview = f.preview(&before, s.next_object_id().unwrap(), &PasteOptions::default()).unwrap();
    let a = preview.at(Point { x_nm: 20_000_000, y_nm: 20_000_000 }, 0).unwrap();
    let b = preview.at(Point { x_nm: 40_000_000, y_nm: 40_000_000 }, 0).unwrap();
    assert_eq!(a.sheets.iter().map(|s| s.sheet_id).collect::<Vec<_>>(), b.sheets.iter().map(|s| s.sheet_id).collect::<Vec<_>>());
    let block = preview.sheet_symbol().unwrap();
    let (a_block, b_block) = (a.subsheets.iter().find(|s| s.id == block).unwrap(), b.subsheets.iter().find(|s| s.id == block).unwrap());
    assert_ne!(a_block.at, b_block.at);
    assert_eq!(a_block.child, b_block.child);
    let child = SheetTarget { project: a.id(), sheet: a_block.child };
    assert_eq!(a.for_sheet(child).unwrap().symbols()[0].position, b.for_sheet(child).unwrap().symbols()[0].position);
    assert_eq!(a.electrical_connectivity().nets, b.electrical_connectivity().nets);
    assert!(preview.at(Point { x_nm: i64::MAX, y_nm: 0 }, 0).is_err());
    assert!(preview.at(Point { x_nm: 0, y_nm: 0 }, 1).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// The versioned root UUID survives arbitrary native document display order; missing, foreign
/// and legacy-disguised root metadata is rejected rather than choosing an array's first entry.
#[test]
fn explicit_root_identity_is_independent_of_native_sheet_order() {
    let mut s = fixture();
    let text = s.execute(COPY, &selection(&s, true)).unwrap()["clipboard"].as_str().unwrap().to_owned();
    let f = Fragment::from_text(&text).unwrap();
    let root = f.project.sheet_target().id();
    let mut reordered = (*f.project).clone();
    Arc::make_mut(&mut reordered.sheets).rotate_left(1);
    reordered.sheet_index = reordered.sheets.iter().position(|p| p.sheet_id == root).unwrap();
    let docs =
        encode_documents(Arc::new(reordered)).unwrap().into_iter().map(|(p, b)| (p, String::from_utf8(b).unwrap())).collect::<BTreeMap<_, _>>();
    let mut data: Value = serde_json::from_str(&text).unwrap();
    data["documents"] = json!(docs);
    let parsed = Fragment::from_text(&data.to_string()).unwrap();
    assert_eq!(parsed.project.sheet_target().id(), root);
    let mut target = Session::default();
    target.apply("project.new", &json!({})).unwrap();
    let destination = target.project().unwrap().sheet_target();
    target.apply(PASTE, &paste(&target, &data.to_string(), destination)).unwrap();
    assert_eq!(target.project().unwrap().symbols().len(), 1);
    assert_eq!(target.project().unwrap().sheets.len(), 3);
    data.as_object_mut().unwrap().remove("root_sheet");
    assert!(Fragment::from_text(&data.to_string()).is_err());
    data["root_sheet"] = json!(uuid::Uuid::nil());
    assert!(Fragment::from_text(&data.to_string()).is_err());
}
