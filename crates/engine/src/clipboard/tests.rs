use super::*;
use crate::{DRAW_WIRE, PLACE_SYMBOL, persistence, schematic::GRID_NM};
use serde_json::json;

/// Original fixtures use shared commands and discover IDs rather than editing native documents.
fn fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Clipboard source"})).unwrap();
    for x in [0, 20 * GRID_NM] {
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let symbols = session.project().unwrap().symbols();
    let from = symbols[0].terminals()[1].inspect();
    let to = symbols[1].terminals()[0].inspect();
    session.apply(DRAW_WIRE, &json!({"from":from,"to":to,"bends":[],"expected_revision":session.revision()})).unwrap();
    session
}

/// Select all ordinary objects in the source sheet.
fn all(s: &Session) -> Selection {
    let p = s.project().unwrap();
    Selection {
        symbols: p.symbols().iter().map(|s| s.id()).collect(),
        wires: p.wires().iter().map(|w| w.id()).collect(),
        net_labels: p.net_labels().iter().map(|l| l.object_id()).collect(),
    }
}

/// Copy is a query, and a different session needs neither the source session nor its catalog.
/// Paste creates disjoint IDs/references, preserves the circuit and is one undo/redo step.
#[test]
fn portable_copy_paste_is_atomic_and_offline() {
    let mut source = fixture();
    let before = source.snapshot().unwrap();
    let revision = source.revision();
    let copied = source.execute(COPY, &json!({"items":all(&source),"expected_revision":revision})).unwrap();
    assert!(Arc::ptr_eq(&source.snapshot().unwrap(), &before));
    assert_eq!(source.revision(), revision);
    let text = copied["clipboard"].as_str().unwrap();
    let fragment = Fragment::from_text(text).unwrap();
    assert_eq!(fragment.counts(), (2, 1, 0));
    let mut target = fixture();
    let before = target.snapshot().unwrap();
    target.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":20*GRID_NM,"expected_revision":target.revision()})).unwrap();
    let after = target.snapshot().unwrap();
    assert_eq!(after.symbols().len(), 4);
    assert_eq!(after.wires().len(), 2);
    let refs = after.components().map(|c| c.reference().to_owned()).collect::<BTreeSet<_>>();
    assert_eq!(refs.len(), 4);
    let old_ids = before.symbols().iter().map(|s| s.id()).collect::<BTreeSet<_>>();
    assert!(after.symbols()[2..].iter().all(|s| !old_ids.contains(&s.id())));
    let wire = &after.wires()[1];
    assert!(wire.terminals().iter().all(|pin| after.symbols()[2..].iter().any(|s| s.ordinal == pin.symbol)));
    let docs = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
    target.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &target.snapshot().unwrap());
    target.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&after, &target.snapshot().unwrap());
}

/// Copying a wire alone retains its route and frees both absent component endpoints.
#[test]
fn partial_copy_detaches_absent_endpoints_and_preserves_branches() {
    let mut source = fixture();
    let end = source.project().unwrap().wires()[0].endpoints()[1].inspect();
    source.apply(DRAW_WIRE, &json!({"from":end,"to":{"x_nm":40*GRID_NM,"y_nm":0},"bends":[],"expected_revision":source.revision()})).unwrap();
    let items = Selection { wires: source.project().unwrap().wires().iter().map(|w| w.id()).collect(), ..Selection::default() };
    let fragment = Fragment::from_text(&Fragment::capture(source.project().unwrap(), &items).unwrap().to_text().unwrap()).unwrap();
    assert_eq!(fragment.project.symbols().len(), 0);
    assert_eq!(fragment.project.wires()[0].endpoints()[1], fragment.project.wires()[1].endpoints()[0]);
    for (a, b) in source.project().unwrap().wires().iter().zip(fragment.project.wires()) {
        assert_eq!(source.project().unwrap().wire_path(a), fragment.project.wire_path(b));
        assert!(b.terminals().is_empty());
    }
}

/// Cutting a component must preserve unselected incident wiring and the removed component's
/// route coordinates. Undo restores both the component identity and terminal attachment.
#[test]
fn cut_keeps_unselected_incident_routes() {
    let mut source = fixture();
    let before = source.snapshot().unwrap();
    let items = Selection::one_symbol(before.symbols()[0].id());
    let result = source.execute(CUT, &json!({"items":items,"expected_revision":source.revision()})).unwrap();
    assert_eq!(Fragment::from_text(result["clipboard"].as_str().unwrap()).unwrap().counts(), (1, 0, 0));
    let after = source.snapshot().unwrap();
    assert_eq!(after.symbols().len(), 1);
    assert_eq!(after.wires().len(), 1);
    assert_eq!(before.wire_path(&before.wires()[0]), after.wire_path(&after.wires()[0]));
    assert!(matches!(after.wires()[0].endpoints()[0], WireEndpoint::Junction(_)));
    source.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &source.snapshot().unwrap());
}

/// Stale requests, reference conflicts, corrupt/unsupported input and out-of-bounds placement
/// preserve the exact snapshot, revision, redo and allocation state.
#[test]
fn rejected_pastes_are_noops() {
    let mut source = fixture();
    let text = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap().to_text().unwrap();
    let before = source.snapshot().unwrap();
    let revision = source.revision();
    for extra in [
        json!({"references":"preserve"}),
        json!({"x_nm":i64::MAX}),
        json!({"expected_revision":revision-1}),
        json!({"text":"plain external text"}),
        json!({"text":"{\"format\":\"pcbcraft-clipboard/99\"}"}),
    ] {
        let mut params = json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":revision});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(source.apply(PASTE, &params).is_err());
        assert!(Arc::ptr_eq(&before, &source.snapshot().unwrap()));
        assert_eq!(source.revision(), revision);
    }
    assert!(Fragment::from_text(&" ".repeat(MAX_BYTES + 1)).is_err());
}

/// Explicit destination sheets and reference preservation work across projects; missing targets
/// in multisheet projects are rejected rather than silently using the first sheet.
#[test]
fn paste_targets_the_requested_sheet_and_rotates_the_whole_fragment() {
    let source = fixture();
    let fragment = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap();
    let text = fragment.to_text().unwrap();
    let mut target = Session::default();
    target.apply("project.new", &Value::Null).unwrap();
    target.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Second","expected_revision":target.revision()})).unwrap();
    let second = target.project().unwrap().sheets().nth(1).unwrap().target;
    let params = json!({"text":text,"x_nm":0,"y_nm":0,"rotation":90,"references":"preserve","expected_revision":target.revision()});
    assert!(target.apply(PASTE, &params).is_err());
    let mut params = params;
    params["sheet"] = json!(second.id());
    target.apply(PASTE, &params).unwrap();
    assert!(target.project().unwrap().symbols().is_empty());
    let sheet = target.project().unwrap().for_sheet(second).unwrap();
    assert_eq!(sheet.symbols().len(), 2);
    assert_eq!(sheet.symbols()[1].position, Point { x_nm: 0, y_nm: -20 * GRID_NM });
    assert_eq!(sheet.symbols()[0].turns(), 1);
}

/// Exact package intent and terminal permutation survive a board-to-boardless copy without
/// introducing physical placements or consulting the receiving library catalog.
#[test]
fn copied_package_assignments_keep_the_pinned_mapping() {
    let mut source = fixture();
    let component = source.project().unwrap().components().next().unwrap().id();
    source.apply(crate::ASSIGN_FOOTPRINT,&json!({"component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":source.revision()})).unwrap();
    let text = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap().to_text().unwrap();
    let mut target = Session::default();
    target.apply("project.new", &Value::Null).unwrap();
    target.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":target.revision()})).unwrap();
    let p = target.project().unwrap();
    assert!(p.board_target().is_none());
    assert_eq!(p.schematic_packages.assignments.len(), 1);
    assert_eq!(
        p.schematic_packages.assignments.values().next().unwrap().mapping,
        BTreeMap::from([("1".into(), "2".into()), ("2".into(), "1".into())])
    );
    assert_eq!(p.schematic_packages.definitions.len(), 1);
}

/// A clipboard cannot replace an existing pinned library identity/revision with different content.
#[test]
fn conflicting_definition_content_cannot_be_pasted() {
    let mut source = fixture();
    let text = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap().to_text().unwrap();
    let mut envelope: Envelope = serde_json::from_str(&text).unwrap();
    let definition = envelope.documents.iter_mut().find(|(path, _)| path.starts_with("library/symbols/")).unwrap().1;
    let mut data: Value = serde_json::from_str(definition).unwrap();
    data["name"] = json!("Different content with the same identity");
    *definition = data.to_string();
    let text = serde_json::to_string(&envelope).unwrap();
    assert!(Fragment::from_text(&text).is_ok());
    let before = source.snapshot().unwrap();
    assert!(source.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":source.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &source.snapshot().unwrap()));
}

/// Named nets retain deliberate global joining by default, with an explicit suffix isolating a
/// pasted circuit. Invalid suffixes fail atomically rather than silently truncating names.
#[test]
fn pasted_label_names_and_rotations_follow_the_options() {
    let mut source = fixture();
    source
        .apply(
            crate::net_labels::ADD,
            &json!({"name":"SUPPLY","style":"global","x_nm":0,"y_nm":0,"rotation":90,"expected_revision":source.revision()}),
        )
        .unwrap();
    let text = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap().to_text().unwrap();
    source
        .apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":30*GRID_NM,"rotation":90,"net_name_suffix":"_B","expected_revision":source.revision()}))
        .unwrap();
    let label = source.project().unwrap().net_labels().last().unwrap();
    assert_eq!(label.name, "SUPPLY_B");
    assert_eq!(label.rotation, 180);
    let before = source.snapshot().unwrap();
    assert!(source.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"net_name_suffix":"\n","expected_revision":source.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &source.snapshot().unwrap()));
}

/// Native power names use component values, so suffix isolation must cover them as well as text labels.
#[test]
fn copied_power_symbols_receive_the_net_name_suffix() {
    let mut source = Session::default();
    source.apply("project.new", &Value::Null).unwrap();
    let definition = source.libraries.latest_symbol("power", "gnd").unwrap();
    source.apply(PLACE_SYMBOL,&json!({"library":"power","definition":"gnd","definition_revision":definition.revision(),"definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),"x_nm":0,"y_nm":0,"value":"GND","expected_revision":source.revision()})).unwrap();
    let text = Fragment::capture(source.project().unwrap(), &all(&source)).unwrap().to_text().unwrap();
    source.apply(PASTE, &json!({"text":text,"x_nm":10*GRID_NM,"y_nm":0,"net_name_suffix":"_B","expected_revision":source.revision()})).unwrap();
    let project = source.project().unwrap();
    let pasted = project.symbols().last().unwrap();
    assert_eq!(project.value(pasted), "GND_B");
    assert_ne!(project.reference(&project.symbols()[0]), project.reference(pasted));
}
