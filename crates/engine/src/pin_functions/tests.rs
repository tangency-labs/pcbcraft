use super::*;
use crate::{
    authoring::{Action, Item, Kind, Primitive},
    library,
    persistence::{ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed},
    schematic::{AlternatePinFunction, ElectricalType, Shape},
};
use std::sync::Arc;

/// Build a synthetic MCU through the shared symbol-edit and publication transactions.
fn editor() -> crate::authoring::Session {
    let item = Item::new(Kind::Symbol).unwrap();
    let mut pin = item.objects[&1].clone();
    let Primitive::Pin { name, electrical_type, alternates, .. } = &mut pin else { panic!("expected starter pin") };
    *name = Some("GPIO".into());
    *electrical_type = ElectricalType::Bidirectional;
    *alternates = vec![AlternatePinFunction { name: "ADC_IN".into(), electrical_type: ElectricalType::Input }];
    let mut editor = crate::authoring::Session::new(item, false).unwrap();
    editor.execute(0, Action::Replace { object: 1, primitive: pin }).unwrap();
    editor
}

/// Install and pin the authored symbol through the same project commands used by MCP.
fn fixture(board: bool) -> Session {
    let item = editor().snapshot();
    let manifest = crate::authoring::publishing::new_library("synthetic", "Synthetic", "MIT OR Apache-2.0").unwrap();
    let published = crate::authoring::publishing::prepare_copy(item, &manifest).unwrap();
    let item = published.item();
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    let mut documents = published.documents().clone();
    documents.insert("library.json".into(), manifest);
    session.execute("library.load_source", &json!({"source":"mcu","documents":documents})).unwrap();
    let definition = session.libraries.symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = library::sources::symbol_digest(&definition).unwrap();
    session
        .apply(
            crate::PLACE_SYMBOL,
            &json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,
        "definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    session
}

fn part_target(session: &Session) -> Value {
    json!({"part":session.project().unwrap().symbols()[0].component_id(),"terminal":"1"})
}

fn params(session: &Session, function: Value) -> Value {
    json!({"target":part_target(session),"function":function,"expected_revision":session.revision()})
}

/// Extract the rendered schematic labels, ignoring non-text shapes.
fn ink(project: &Project) -> Vec<String> {
    let placed = &project.symbols()[0];
    placed
        .graphics(project.component(placed))
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Graphic(crate::library_graphics::Graphic { geometry: crate::library_graphics::Geometry::Text { text, .. }, .. }) => Some(text),
            _ => None,
        })
        .collect()
}

/// One choice changes drawing, inspection, the existing electrical check and netlist metadata.
/// Its dry run and repeated application are no-ops, and one Undo restores every old value.
#[test]
fn chooses_typed_function_without_changing_connectivity() {
    let mut session = fixture(false);
    session.apply(crate::no_connect::SET, &json!({"target":part_target(&session),"marked":true,"expected_revision":session.revision()})).unwrap();
    let before = session.snapshot().unwrap();
    let old_nets = before.netlist().unwrap().netlist.nets;
    assert!(ink(&before).contains(&"GPIO".to_owned()));
    let revision = session.revision();
    let mut choice = params(&session, json!("ADC_IN"));
    choice["dry_run"] = json!(true);
    let preview = session.execute(SET, &choice).unwrap();
    assert_eq!(preview["pin_function_change"]["before"]["electrical_type"], "bidirectional");
    assert_eq!(preview["pin_function_change"]["after"]["name"], "ADC_IN");
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));

    choice["dry_run"] = json!(false);
    assert_eq!(session.execute(SET, &choice).unwrap()["pin_function_change"]["changed"], true);
    let after = session.snapshot().unwrap();
    assert!(ink(&after).contains(&"ADC_IN".to_owned()));
    assert!(!ink(&after).contains(&"GPIO".to_owned()));
    assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
    let inspect = session.inspect();
    let pin = &inspect["project"]["schematic"]["symbols"][0]["pins"][0];
    assert_eq!(pin["name"], "ADC_IN");
    assert_eq!(pin["electrical_type"], "input");
    assert!(after.check_no_connects(session.revision()).findings.iter().any(|finding| finding.rule == "no_connect_incompatible_terminal"));
    let netlist = after.netlist().unwrap().netlist;
    assert_eq!(netlist.nets, old_nets);
    assert_eq!(netlist.components[0].terminals[0].name.as_deref(), Some("ADC_IN"));
    assert_eq!(netlist.components[0].terminals[0].electrical_type, Some(ElectricalType::Input));
    let same = params(&session, json!("ADC_IN"));
    assert_eq!(session.execute(SET, &same).unwrap()["pin_function_change"]["changed"], false);
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &after));
    session.apply("edit.undo", &Value::Null).unwrap();
    let restored = session.snapshot().unwrap();
    assert!(ink(&restored).contains(&"GPIO".to_owned()));
    assert_eq!(restored.netlist().unwrap().netlist.components[0].terminals[0].electrical_type, Some(ElectricalType::Bidirectional));
    assert!(!restored.check_no_connects(session.revision()).findings.iter().any(|finding| finding.rule == "no_connect_incompatible_terminal"));
    session.apply("edit.redo", &Value::Null).unwrap();
    assert!(ink(session.project().unwrap()).contains(&"ADC_IN".to_owned()));
}

/// Unknown, malformed or stale requests cannot publish any part of the transaction.
#[test]
fn refuses_unknown_terminal_function_and_stale_request() {
    let mut session = fixture(false);
    let before = session.snapshot().unwrap();
    let valid = params(&session, json!("ADC_IN"));
    for (key, value, expected) in [
        ("expected_revision", json!(0), None),
        ("function", json!("GPIO"), Some("Unknown alternate pin function")),
        ("function", json!(" ADC_IN"), Some("must be a trimmed pin function name or null")),
        ("target", json!({"junction":"junction-1"}), Some("must be a logical {part, terminal} address")),
        ("target", json!({"part":part_target(&session)["part"],"terminal":"99"}), Some("Unknown terminal")),
        ("target", json!({"part":"component-9999","terminal":"1"}), Some("Unknown part instance")),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        let error = session.execute(SET, &invalid).expect_err("invalid pin function request must be rejected");
        match (expected, error) {
            (None, CommandError::StaleRevision { current }) => assert_eq!(current, session.revision()),
            (Some(expected), CommandError::InvalidParams(reason)) => assert!(reason.contains(expected), "{invalid}: {reason}"),
            (_, error) => panic!("{invalid}: unexpected error {error}"),
        }
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        assert_eq!(session.revision(), valid["expected_revision"].as_u64().unwrap());
    }
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("function");
    assert!(matches!(session.apply(SET, &missing), Err(CommandError::InvalidParams(reason)) if reason.contains("needs `function`")));
    session.apply(SET, &valid).unwrap();
    let clear = params(&session, Value::Null);
    session.apply(SET, &clear).unwrap();
    assert!(ink(session.project().unwrap()).contains(&"GPIO".to_owned()));
}

/// Save and reopen the authored choice with and without a board. Only project.json adopts the
/// new choice format; the pinned symbol and other documents do not change on selection.
#[test]
fn pin_function_storage_is_lazy_and_refuses_invalid_records() {
    for board in [false, true] {
        let mut session = fixture(board);
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(before, encode_documents(decode_documents(&before).unwrap()).unwrap());
        let choice = params(&session, json!("ADC_IN"));
        session.apply(SET, &choice).unwrap();
        let selected = session.snapshot().unwrap();
        let docs = encode_documents(selected.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        assert_eq!(stored["format"], "pcbcraft-project/48");
        assert_eq!(stored["components"][0]["pin_functions"], json!([{"terminal":"1","function":"ADC_IN"}]));
        for (path, bytes) in &before {
            if path != ENTRY_POINT {
                assert_eq!(docs.get(path), Some(bytes), "changed {path}");
            }
        }
        let reopened = decode_documents(&docs).unwrap();
        assert!(ink(&reopened).contains(&"ADC_IN".to_owned()));
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        assert_eq!(encode_documents(decode_packed(&encode_packed(selected).unwrap()).unwrap().project).unwrap(), docs);
        let old: Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
        for (change, expected) in [
            ("old format", "require project format 48"),
            ("missing", "require project format 48"),
            ("empty", "Invalid alternate pin function record count"),
            ("null", "invalid type: null, expected a sequence"),
            ("unknown", "Unknown or duplicate alternate pin function selection"),
            ("duplicate", "Unknown or duplicate alternate pin function selection"),
        ] {
            let mut invalid = stored.clone();
            let component = &mut invalid["components"][0];
            match change {
                "missing" => {
                    component.as_object_mut().unwrap().remove("pin_functions");
                }
                "empty" => component["pin_functions"] = json!([]),
                "null" => component["pin_functions"] = Value::Null,
                "unknown" => component["pin_functions"][0]["function"] = json!("FIXME"),
                "duplicate" => component["pin_functions"] = json!([component["pin_functions"][0], component["pin_functions"][0]]),
                _ => invalid["format"] = old["format"].clone(),
            }
            let mut changed = docs.clone();
            changed.insert(ENTRY_POINT.into(), serde_json::to_vec(&invalid).unwrap());
            let error = decode_documents(&changed).err().expect("invalid pin function record must be rejected");
            assert!(error.contains(expected), "{change}: {error}");
        }
        let clear = params(&session, Value::Null);
        session.apply(SET, &clear).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    }
}

/// The choice envelope preserves the preceding entry/display contracts for assembly profiles
/// and both membership encodings, in both storage homes.
#[test]
fn pin_function_storage_composes_with_profiles_and_memberships() {
    use crate::part_groups::{CREATE, SET_GROUPS};
    for board in [false, true] {
        let mut session = fixture(board);
        session.apply(CREATE, &json!({"name":"Synthetic group","expected_revision":session.revision()})).unwrap();
        let project = session.project().unwrap();
        let group = project.part_groups().groups[0].id;
        let member = project.part_instance_addresses().next().unwrap();
        session.apply(SET_GROUPS, &json!({"part_instance":member,"groups":[group],"expected_revision":session.revision()})).unwrap();
        let grouped = session.snapshot().unwrap();
        for (compact, profile) in [(false, true), (true, false), (true, true)] {
            session.open_snapshot(Arc::new(Project { prefer_compact_part_groups: compact, ..Project::clone(&grouped) }));
            if profile {
                session.apply(crate::SET_ASSEMBLY_PROFILE, &json!({"content":{"name":"Synthetic profile","revision":1,"provenance":{"kind":"authored"}},"expected_revision":session.revision()})).unwrap();
            }
            let original = session.snapshot().unwrap();
            let before = encode_documents(original.clone()).unwrap();
            let entry: Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
            assert_eq!(entry["format"], if profile { "pcbcraft-project/36" } else { "pcbcraft-project/39" });
            let choice = params(&session, json!("ADC_IN"));
            session.apply(SET, &choice).unwrap();
            let docs = encode_documents(session.snapshot().unwrap()).unwrap();
            let selected: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
            assert_eq!(selected["format"], "pcbcraft-project/48");
            if let Some(path) = original.instances_document_path() {
                assert_eq!(docs[&path], before[&path]);
            } else {
                assert_eq!(selected["schematic_instances"], entry["schematic_instances"]);
            }
            let reopened = decode_documents(&docs).unwrap();
            assert_eq!(reopened.part_group_memberships(), original.part_group_memberships());
            assert_eq!(reopened.assembly_profiles(), original.assembly_profiles());
            assert_eq!(encode_documents(reopened).unwrap(), docs);
            session.apply("edit.undo", &Value::Null).unwrap();
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// A saved differential pair keeps its polarity and original instances header when a pin
/// function is selected. Clearing the choice restores the exact preceding /45 bytes.
#[test]
fn pin_function_storage_preserves_differential_pairs() {
    use crate::nets::{Pair, PairState};
    for board in [false, true] {
        let mut session = fixture(board);
        for (index, name) in ["Signal_P", "Signal_N"].iter().enumerate() {
            session
                .apply(
                    crate::net_labels::ADD,
                    &json!({"name":name,"style":"local","x_nm":index as i64*25_400_000,"y_nm":12_700_000,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
        let mut project = session.project().unwrap().clone();
        let ids: Vec<_> = project.net_registry.records.iter().map(|record| record.id).collect();
        assert_eq!(ids.len(), 2);
        let pair = Pair { id: uuid::Uuid::from_u128(9000), positive: ids[0], negative: ids[1], name: "Signal".into(), state: PairState::Active {} };
        Arc::make_mut(&mut project.net_registry).pairs = Arc::new(vec![Arc::new(pair.clone())]);
        session.change(project).unwrap();
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&before[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/45");
        session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
        let docs = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&docs[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/48");
        if let Some(path) = session.project().unwrap().instances_document_path() {
            assert_eq!(docs[&path], before[&path]);
            assert_eq!(serde_json::from_slice::<Value>(&docs[&path]).unwrap()["format"], "pcbcraft-instances/8");
        }
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(reopened.net_registry.pairs.as_ref(), &[Arc::new(pair)]);
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        session.apply(SET, &params(&session, Value::Null)).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Draft and canonical symbol versions appear only with alternates; editor Undo restores the
/// working definition, and invalid authoring never advances its content history.
#[test]
fn alternate_definition_validates_and_versions_lazily() {
    let mut editor = editor();
    let draft = editor.snapshot();
    let encoded = draft.encode().unwrap();
    let decoded = Item::decode(&encoded).unwrap();
    assert_eq!(decoded.encode().unwrap(), encoded);
    assert_eq!(decoded.native().unwrap()["format"], "pcbcraft-symbol/5");
    let mut old: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(old["format"], "pcbcraft-library-item/5");
    old["format"] = json!("pcbcraft-library-item/1");
    let error = Item::decode(&serde_json::to_vec(&old).unwrap()).err().expect("old library item format cannot contain alternates");
    assert_eq!(error, "Invalid or oversized native library item.");
    let good: Vec<_> =
        (0..MAX_ALTERNATES).map(|i| AlternatePinFunction { name: format!("ALT_{i}"), electrical_type: ElectricalType::Input }).collect();
    for (bad, expected) in [
        (vec![good[0].clone(); 2], "bounded list of uniquely named alternate pin functions"),
        (vec![AlternatePinFunction { name: "GPIO".into(), electrical_type: ElectricalType::Input }], "repeats its primary name"),
        (
            good.iter().cloned().chain([AlternatePinFunction { name: "TOO_MANY".into(), electrical_type: ElectricalType::Input }]).collect(),
            "bounded list of uniquely named alternate pin functions",
        ),
    ] {
        let mut primitive = editor.snapshot().objects[&1].clone();
        let Primitive::Pin { alternates, .. } = &mut primitive else { panic!("expected pin") };
        *alternates = bad;
        let error =
            editor.execute(editor.revision(), Action::Replace { object: 1, primitive }).expect_err("invalid alternate declarations must be rejected");
        assert!(error.contains(expected), "{error}");
        assert!(Arc::ptr_eq(&editor.snapshot(), &draft));
    }
    let mut primitive = draft.objects[&1].clone();
    let Primitive::Pin { alternates, .. } = &mut primitive else { panic!("expected pin") };
    *alternates = good;
    editor.execute(editor.revision(), Action::Replace { object: 1, primitive }).unwrap();
    editor.execute(editor.revision(), Action::Undo).unwrap();
    editor.execute(editor.revision(), Action::Undo).unwrap();
    assert_eq!(editor.snapshot().native().unwrap()["format"], library::SYMBOL_FORMAT);
}
