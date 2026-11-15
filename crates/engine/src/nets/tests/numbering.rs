use super::*;

/// Read a displayed number through the same graph projection used by GUI and MCP.
fn name_at(session: &Session, target: &Value) -> String {
    let project = session.project().unwrap();
    let symbol = target["symbol"].as_str().unwrap();
    let terminal = target["terminal"].as_str().unwrap();
    let pin =
        project.all_symbols().find(|placed| placed.id() == symbol).unwrap().terminals().into_iter().find(|pin| pin.number() == terminal).unwrap();
    let graph = project.electrical_connectivity();
    graph.net_name(graph.net_index_for(pin).unwrap())
}

/// The representative component can be renamed and deleted without renaming the surviving
/// net. Undo/Redo and native folder/archive reopen retain both number and persistent UUID.
#[test]
fn neutral_names_survive_reference_edits_primary_deletion_and_native_history() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[1], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    let name = name_at(&session, &pins[1]);
    assert_eq!(name, "Net-0001");
    let id = id_for(&session, &pins[1]);
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"symbol":pins[0]["symbol"],"reference":"Z99","value":"R"}));
    assert_eq!(name_at(&session, &pins[1]), name);
    let before = session.snapshot().unwrap();
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
    assert_eq!(name_at(&session, &pins[1]), name);
    assert_eq!(id_for(&session, &pins[1]), id);
    assert!(!session.project().unwrap().all_symbols().any(|symbol| symbol.id() == pins[0]["symbol"].as_str().unwrap()));
    let deleted = session.snapshot().unwrap();
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(before).unwrap());
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(deleted).unwrap());
}

/// Previously unregistered files keep exact bytes on inspect/save; their first topology edit
/// adopts the numbers shown before the edit instead of recomputing them after anchor removal.
#[test]
fn legacy_read_is_pure_and_first_deletion_adopts_the_visible_number() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[1], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    let mut legacy = session.snapshot().unwrap().as_ref().clone();
    legacy.net_registry = Arc::default();
    legacy.named_graph = Arc::default();
    for sheet in Arc::make_mut(&mut legacy.sheets) {
        sheet.connectivity = Arc::default();
    }
    let docs = encode_documents(Arc::new(legacy)).unwrap();
    session.open_snapshot(decode_documents(&docs).unwrap());
    let before = name_at(&session, &pins[1]);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
    assert!(session.project().unwrap().net_registry.records.is_empty());
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
    assert_eq!(name_at(&session, &pins[1]), before);
    assert_eq!(session.project().unwrap().net_registry.records.len(), 1);
    round_trip(&session);
}

/// Independent nets have independent numbers. A true merge gets a fresh identity/number;
/// historical numbers remain reserved and Undo restores both original nets exactly.
#[test]
fn merges_allocate_a_new_number_without_reusing_either_parent() {
    let (mut session, pins) = circuit(4, true);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[2], &pins[3]);
    let a = name_at(&session, &pins[0]);
    let b = name_at(&session, &pins[2]);
    assert_eq!((&*a, &*b), ("Net-0001", "Net-0002"));
    connect(&mut session, &pins[1], &pins[2]);
    assert_eq!(name_at(&session, &pins[0]), "Net-0003");
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(name_at(&session, &pins[0]), a);
    assert_eq!(name_at(&session, &pins[2]), b);
}

/// Older persistent UUIDs without presentation numbers keep their read-only numbers on the
/// first edit, independently of UUID order and of which component was the original anchor.
#[test]
fn legacy_unnumbered_identities_adopt_without_renumbering_other_nets() {
    let (mut session, pins) = circuit(6, false);
    for (a, b) in [(0, 1), (1, 2), (3, 4), (4, 5)] {
        connect(&mut session, &pins[a], &pins[b]);
    }
    let mut legacy = session.snapshot().unwrap().as_ref().clone();
    for record in &mut Arc::make_mut(&mut legacy.net_registry).records {
        record.generated_number = None;
    }
    legacy.named_graph = Arc::default();
    for sheet in Arc::make_mut(&mut legacy.sheets) {
        sheet.connectivity = Arc::default();
    }
    let docs = encode_documents(Arc::new(legacy)).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&docs[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/17");
    session.open_snapshot(decode_documents(&docs).unwrap());
    let a = name_at(&session, &pins[1]);
    let b = name_at(&session, &pins[4]);
    assert_ne!(a, b);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
    assert_eq!(name_at(&session, &pins[1]), a);
    assert_eq!(name_at(&session, &pins[4]), b);
    round_trip(&session);
}

/// Authored literals can resemble generated names, but their collision qualifiers must not
/// reintroduce a component-dependent path or digest when the representative is deleted.
#[test]
fn generated_literal_collisions_remain_component_independent() {
    let (mut session, pins) = circuit(3, false);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[1], &pins[2]);
    apply(&mut session, crate::net_labels::ADD, json!({"name":"Net-0001","style":"local","x_nm":200_000_000,"y_nm":0}));
    let qualified = name_at(&session, &pins[1]);
    assert_eq!(qualified, "Generated::Net-0001");
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
    assert_eq!(name_at(&session, &pins[1]), qualified);
}

/// New envelopes reject malformed numbers and numbers smuggled into old vocabularies. Failed
/// decoding leaves the input documents intact instead of repairing or silently renumbering.
#[test]
fn numbered_codec_rejects_invalid_missing_duplicate_and_old_version_fields() {
    for board in [false, true] {
        let (mut session, pins) = circuit(4, board);
        connect(&mut session, &pins[0], &pins[1]);
        connect(&mut session, &pins[2], &pins[3]);
        let project = session.snapshot().unwrap();
        let path = project.instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let original = encode_documents(project).unwrap();
        for case in 0..7 {
            let mut docs = original.clone();
            let mut value: Value = serde_json::from_slice(&docs[&path]).unwrap();
            let registry = if board { &mut value["net_registry"] } else { &mut value["schematic_instances"]["net_registry"] };
            match case {
                0 => registry["records"][0]["generated_number"] = json!(0),
                1 => registry["records"][0]["generated_number"] = Value::Null,
                2 => registry["records"][1]["generated_number"] = registry["records"][0]["generated_number"].clone(),
                3 => {
                    registry["records"][0].as_object_mut().unwrap().remove("generated_number");
                }
                4 => registry["records"][0]["generated_number"] = json!(u64::MAX),
                5 => registry["records"][0]["generated_number"] = json!(1.5),
                _ => {
                    value["format"] = json!(if board { "pcbcraft-instances/2" } else { "pcbcraft-project/17" });
                }
            }
            docs.insert(path.clone(), serde_json::to_vec(&value).unwrap());
            let attempted = docs.clone();
            assert!(decode_documents(&docs).is_err(), "board={board}, case={case}");
            assert_eq!(docs, attempted);
        }
        assert_eq!(encode_documents(decode_documents(&original).unwrap()).unwrap(), original);
    }
}

/// Exhausted numbering refuses the complete shared command without changing document bytes,
/// revision or undo history; a number is never wrapped, reused or assigned after publication.
#[test]
fn number_exhaustion_is_atomic() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    let mut project = session.snapshot().unwrap().as_ref().clone();
    Arc::make_mut(&mut project.net_registry).records[0].generated_number = Some(u32::MAX);
    project.named_graph = Arc::default();
    for sheet in Arc::make_mut(&mut project.sheets) {
        sheet.connectivity = Arc::default();
    }
    session.open_snapshot(Arc::new(project));
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    assert!(session.apply(crate::CONNECT_PINS, &json!({"from":pins[2],"to":pins[3],"expected_revision":revision})).is_err());
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}
