use super::*;
use crate::{
    Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::json;

/// Initialize the accepted state only after the production chooser/placement commands.
fn initialized() -> Session {
    let mut session = super::tests::fixture();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    session
}

/// Enabling the mode is one explicit edit; old projects otherwise retain their
/// saved encoding, and current state is adopted once rather than on every call.
#[test]
fn initialization_is_explicit_versioned_atomic_and_a_repeat_is_noop() {
    let mut session = super::tests::fixture();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(!std::str::from_utf8(&before[&crate::persistence::test_board_path(&before)]).unwrap().contains("baseline"));
    let revision = session.revision();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":revision})).unwrap();
    assert_eq!(session.revision(), revision + 1);
    let docs = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&docs[&crate::persistence::test_board_path(&docs)]).unwrap().contains("pcbcraft-board/10"));
    let source = session.project().unwrap().board.baseline.as_ref().unwrap().source.clone();
    source.validate().unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    assert_eq!(encode_documents(decode_packed(&encode_packed(session.snapshot().unwrap()).unwrap()).unwrap().project).unwrap(), docs);
    let revision = session.revision();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":revision})).unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Each supported capture deletion path retains exactly the accepted physical
/// state and its source metadata, including when every live component disappears.
#[test]
fn symbol_group_and_sheet_deletions_retain_visible_offline_footprints() {
    for mode in 0..3 {
        let mut session = super::tests::fixture();
        let main = session.project().unwrap().sheet_target();
        if mode == 2 {
            session.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Keep","expected_revision":session.revision()})).unwrap();
        }
        session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
        let accepted = encode_documents(session.snapshot().unwrap()).unwrap();
        let symbol = session.project().unwrap().symbols[0].id();
        let component = session.project().unwrap().symbols[0].component;
        let reference = session.project().unwrap().components[&component].reference.clone();
        let mut params = json!({"expected_revision":session.revision()});
        let command = match mode {
            0 => {
                params["symbol"] = json!(symbol);
                crate::DELETE_SYMBOL
            }
            1 => {
                params["items"] = json!({"symbols":[symbol],"wires":[],"net_labels":[]});
                crate::DELETE_ITEMS
            }
            _ => {
                params["sheet"] = json!(main.id());
                crate::sheets::DELETE_SHEET
            }
        };
        if mode != 2 {
            params["sheet"] = json!(main.id());
        }
        session.apply(command, &params).unwrap();
        let project = session.project().unwrap();
        assert!(!project.components.contains_key(&component));
        assert_eq!(project.board.footprints().len(), 2);
        let view = project.boards().next().unwrap();
        let footprint = view.footprints().find(|footprint| footprint.component == format!("component-{component}")).unwrap();
        assert!(footprint.pending_removal);
        assert_eq!(footprint.reference, reference);
        assert_eq!(footprint.pads.len(), 2);
        assert!(!footprint.graphics.is_empty());
        assert!(view.selection_for_footprints(&BTreeSet::from([footprint.id.clone()])).symbols.is_empty());
        assert!(session.inspect()["project"]["board"]["pending_removals"].as_array().unwrap().contains(&json!(format!("component-{component}"))));
        let documents = encode_documents(session.snapshot().unwrap()).unwrap();
        let restored = decode_documents(&documents).unwrap();
        assert_eq!(encode_documents(restored.clone()).unwrap(), documents);
        assert_eq!(restored.boards().next().unwrap().footprints().count(), 2);
        assert_eq!(restored.board.definitions.len(), 2);
        let packed = encode_packed(session.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_packed(&packed).unwrap().project).unwrap(), documents);
        let mut encoder = crate::persistence::DocumentsEncoder::new(session.snapshot().unwrap());
        let incremental = loop {
            if let Some(done) = encoder.step(1).unwrap() {
                break done;
            }
        };
        assert_eq!(incremental, documents);
        assert!(encoder.step(1).is_err());
        session.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), accepted);
        session.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents);
    }
}

/// Accepted package assets remain registered even after both their source and an
/// explicitly deleted physical placement are absent from the editable project.
#[test]
fn accepted_assets_are_not_garbage_collected_with_their_last_live_source() {
    let mut session = initialized();
    let symbol = session.project().unwrap().symbols[0].id();
    let footprint = session.project().unwrap().board.footprints()[0].id;
    session.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    session
        .apply(crate::DELETE_FOOTPRINTS, &json!({"footprints":[format!("footprint-{footprint}")],"expected_revision":session.revision()}))
        .unwrap();
    assert_eq!(session.project().unwrap().board.footprints().len(), 1);
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(documents.keys().any(|path| path.contains("resistor-0402.r")));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// Editing schematic wires changes proposed connectivity without accepting a new
/// board graph, including when initialization is invoked again afterward.
#[test]
fn accepted_pad_nets_do_not_follow_unreviewed_schematic_edits() {
    let mut session = super::tests::fixture();
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| symbol.id()).collect();
    session
        .apply(
            crate::CONNECT_PINS,
            &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let old_source = session.project().unwrap().board.baseline.as_ref().unwrap().source.clone();
    let old_nets: Vec<_> = session
        .project()
        .unwrap()
        .boards()
        .next()
        .unwrap()
        .footprints()
        .flat_map(|footprint| footprint.pads.into_iter().map(|pad| pad.net))
        .collect();
    assert_eq!(old_nets.iter().filter(|name| name.is_some()).count(), 2);
    let wire = session.project().unwrap().wires[0].id();
    session.apply(crate::DELETE_WIRE, &json!({"wire":wire,"expected_revision":session.revision()})).unwrap();
    assert!(session.project().unwrap().electrical_connectivity().nets.is_empty());
    let current_nets: Vec<_> = session
        .project()
        .unwrap()
        .boards()
        .next()
        .unwrap()
        .footprints()
        .flat_map(|footprint| footprint.pads.into_iter().map(|pad| pad.net))
        .collect();
    assert_eq!(current_nets, old_nets);
    let revision = session.revision();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":revision})).unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().source, old_source);
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// A traversal indexes the exact accepted snapshot, including named, unnamed and
/// absent memberships. A new accepted graph or source deletion cannot reuse an
/// older index, while a partly consumed older iterator keeps its original names.
#[test]
fn accepted_pad_net_lookup_is_bound_to_each_lazy_snapshot_traversal() {
    let mut session = initialized();
    let mut project = session.project().unwrap().clone();
    let ids: Vec<_> = project.components.keys().copied().collect();
    let baseline = Arc::make_mut(Arc::make_mut(&mut project.board).baseline.as_mut().unwrap());
    baseline.nets = vec![
        NetState { terminals: vec![TerminalKey { component: ids[0], terminal: "1".into() }], names: vec!["RAIL".into(), "VCC".into()] },
        NetState {
            terminals: vec![TerminalKey { component: ids[0], terminal: "2".into() }, TerminalKey { component: ids[1], terminal: "1".into() }],
            names: vec![],
        },
    ];
    baseline.validate().unwrap();
    let original = Arc::new(project);
    let mut older_traversal = original.boards().next().unwrap().footprints();
    let first = older_traversal.next().unwrap();
    assert_eq!(first.pads.iter().map(|pad| pad.net.as_deref()).collect::<Vec<_>>(), vec![Some("RAIL"), Some("Net 2")]);

    let mut changed = original.as_ref().clone();
    let baseline = Arc::make_mut(Arc::make_mut(&mut changed.board).baseline.as_mut().unwrap());
    baseline.nets[0].names = vec!["CHANGED".into()];
    baseline.nets[1].names = vec!["REVIEWED".into()];
    baseline.validate().unwrap();
    let second = older_traversal.next().unwrap();
    assert_eq!(second.pads.iter().map(|pad| pad.net.as_deref()).collect::<Vec<_>>(), vec![Some("Net 2"), None]);
    assert!(older_traversal.next().is_none());

    session.open_snapshot(Arc::new(changed));
    let labels = |project: &Project| {
        project.boards().next().unwrap().footprints().flat_map(|footprint| footprint.pads.into_iter().map(|pad| pad.net)).collect::<Vec<_>>()
    };
    let expected = vec![Some("CHANGED".into()), Some("REVIEWED".into()), Some("REVIEWED".into()), None];
    assert_eq!(labels(session.project().unwrap()), expected);
    let symbol = session.project().unwrap().symbols[0].id();
    session.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    assert!(session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().pending_removal);
    assert_eq!(labels(session.project().unwrap()), expected);
    let restored = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    assert_eq!(labels(&restored), expected);
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(labels(session.project().unwrap()), expected);
}

/// Ordinary placement edits share immutable accepted history rather than copying
/// its potentially large component/net records into every undo snapshot.
#[test]
fn moving_a_footprint_shares_the_accepted_baseline_arc() {
    let mut session = initialized();
    let baseline = session.project().unwrap().board.baseline.as_ref().unwrap().clone();
    let footprint = session.project().unwrap().board.footprints()[0].id;
    session.apply(crate::SET_FOOTPRINT_TRANSFORM, &json!({"footprint":format!("footprint-{footprint}"),"x_nm":4_000_000,"y_nm":5_000_000,"rotation":90,"expected_revision":session.revision()})).unwrap();
    assert!(Arc::ptr_eq(&baseline, session.project().unwrap().board.baseline.as_ref().unwrap()));
    session.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&baseline, session.project().unwrap().board.baseline.as_ref().unwrap()));
    session.apply("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&baseline, session.project().unwrap().board.baseline.as_ref().unwrap()));
}

/// Last-live reference/value overlays survive every deletion path and reopen.
/// They do not advance accepted package, terminal or electrical source state.
#[test]
fn deleting_after_shared_field_edits_keeps_last_visible_display_metadata() {
    for mode in 0..3 {
        let mut session = super::tests::fixture();
        let main = session.project().unwrap().sheet_target();
        if mode == 2 {
            session.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Keep","expected_revision":session.revision()})).unwrap();
        }
        session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
        let old = session.project().unwrap().board.baseline.as_ref().unwrap().clone();
        let symbol = session.project().unwrap().symbols[0].id();
        let component = session.project().unwrap().symbols[0].component;
        session
            .apply(
                crate::SET_SYMBOL_FIELDS,
                &json!({"sheet":main.id(),"symbol":symbol,"reference":"R20","value":"33k","expected_revision":session.revision()}),
            )
            .unwrap();
        let before_delete = encode_documents(session.snapshot().unwrap()).unwrap();
        assert!(Arc::ptr_eq(&old, session.project().unwrap().board.baseline.as_ref().unwrap()));
        let mut params = json!({"expected_revision":session.revision(),"sheet":main.id()});
        let command = match mode {
            0 => {
                params["symbol"] = json!(symbol);
                crate::DELETE_SYMBOL
            }
            1 => {
                params["items"] = json!({"symbols":[symbol],"wires":[],"net_labels":[]});
                crate::DELETE_ITEMS
            }
            _ => crate::sheets::DELETE_SHEET,
        };
        session.apply(command, &params).unwrap();
        let updated = session.project().unwrap().board.baseline.as_ref().unwrap();
        assert!(!Arc::ptr_eq(&old, updated));
        assert_eq!(updated.components[&component].reference, "R20");
        assert_eq!(updated.components[&component].value, "33k");
        assert_eq!(updated.components[&component].package, old.components[&component].package);
        assert_eq!(updated.components[&component].terminals, old.components[&component].terminals);
        assert_eq!(updated.nets, old.nets);
        assert_eq!(updated.source, old.source);
        assert_ne!(old.components[&component].reference, "R20");
        let pending = encode_documents(session.snapshot().unwrap()).unwrap();
        let restored = decode_documents(&pending).unwrap();
        let view = restored.boards().next().unwrap();
        let retained = view.footprints().find(|footprint| footprint.component == format!("component-{component}")).unwrap();
        assert_eq!((retained.reference, &*retained.value), ("R20", "33k"));
        assert!(retained.pending_removal);
        assert_eq!(encode_documents(restored).unwrap(), pending);
        session.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_delete);
        assert!(Arc::ptr_eq(&old, session.project().unwrap().board.baseline.as_ref().unwrap()));
    }
}

/// Missing choices, forged board identities and modified package mappings fail
/// atomically; the opt-in mode does not silently accept newly added parts.
#[test]
fn baseline_validation_and_unsupported_assignment_failures_are_atomic() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).is_err());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);

    let mut session = initialized();
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut board: serde_json::Value = serde_json::from_slice(&documents[&crate::persistence::test_board_path(&documents)]).unwrap();
    let mut incomplete = board.clone();
    for component in incomplete["baseline"]["components"].as_object_mut().unwrap().values_mut() {
        component["package"] = serde_json::Value::Null;
    }
    let mut invalid_documents = documents.clone();
    invalid_documents.insert(crate::persistence::test_board_path(&invalid_documents), serde_json::to_vec(&incomplete).unwrap());
    assert!(decode_documents(&invalid_documents).unwrap_err().contains("unassigned package"));
    for (field, value) in [("format", json!("pcbcraft-board/9")), ("baseline", json!(null))] {
        let mut invalid = board.clone();
        invalid[field] = value;
        let mut docs = documents.clone();
        docs.insert(crate::persistence::test_board_path(&docs), serde_json::to_vec(&invalid).unwrap());
        assert!(decode_documents(&docs).is_err());
    }
    board["baseline"]["board"] = json!(Uuid::from_u128(987));
    let mut invalid = documents.clone();
    invalid.insert(crate::persistence::test_board_path(&invalid), serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&invalid).is_err());
    session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":20_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let id = session.project().unwrap().symbols.last().unwrap().component;
    let definition = super::super::definition::basic_components("resistor-0402").unwrap();
    session.apply(super::super::assignment::ASSIGN_FOOTPRINT, &json!({"component":format!("component-{id}"),"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()})).unwrap();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(
        session
            .apply(
                super::super::assignment::PLACE_ASSIGNED_FOOTPRINT,
                &json!({"component":format!("component-{id}"),"x_nm":0,"y_nm":0,"expected_revision":session.revision()})
            )
            .is_err()
    );
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Nested board history uses the same duplicate-rejecting codec as standalone
/// baseline decoding; duplicated mapping keys cannot be adopted through a folder.
#[test]
fn nested_accepted_package_mappings_reject_duplicate_json_keys() {
    let session = initialized();
    let mut documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let package =
        session.project().unwrap().board.baseline.as_ref().unwrap().components.values().find_map(|component| component.package.as_ref()).unwrap();
    let corrupted = super::tests::duplicate_mapping(&documents[&crate::persistence::test_board_path(&documents)], &package.mapping);
    documents.insert(crate::persistence::test_board_path(&documents), corrupted);
    assert!(decode_documents(&documents).is_err());
}

/// Historical terminals cannot contradict a live immutable symbol with the same
/// exact identity. Distinct revisions and removed sources still retain their own
/// independently validated terminal and physical package contracts.
#[test]
fn accepted_terminals_must_match_a_live_identical_symbol_revision() {
    let mut session = initialized();
    let id = session.project().unwrap().symbols[0].component;
    let symbol = session.project().unwrap().symbols[0].id();
    let corrupt = |mut documents: crate::persistence::Documents, different_revision: bool| {
        let mut board: serde_json::Value = serde_json::from_slice(&documents[&crate::persistence::test_board_path(&documents)]).unwrap();
        let historical = &mut board["baseline"]["components"][id.to_string()];
        historical["terminals"] = json!(["X", "Y"]);
        historical["package"]["mapping"] = json!({"X":"1","Y":"2"});
        if different_revision {
            historical["revision"] = json!(historical["revision"].as_u64().unwrap() + 1);
        }
        for footprint in board["footprints"].as_array_mut().unwrap() {
            if footprint["component"] == id {
                footprint["mapping"] = json!({"X":"1","Y":"2"});
            }
        }
        // The independent assignment list is optional; accepted placements alone
        // must not bypass the same-identity logical terminal invariant.
        board.as_object_mut().unwrap().remove("assignments");
        documents.insert(crate::persistence::test_board_path(&documents), serde_json::to_vec(&board).unwrap());
        documents
    };
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let same_revision = corrupt(documents.clone(), false);
    assert!(decode_documents(&same_revision).unwrap_err().contains("same pinned symbol revision"));
    assert!(decode_documents(&corrupt(documents, true)).is_ok());
    session.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    let removed = corrupt(encode_documents(session.snapshot().unwrap()).unwrap(), false);
    let restored = decode_documents(&removed).unwrap();
    assert_eq!(restored.boards().next().unwrap().footprints().count(), 2);
    assert!(!restored.components.contains_key(&id));
}

/// No monolithic version authorizes baseline records, even when the embedded
/// history is otherwise valid; diagnostic encoding must also refuse losing it.
#[test]
fn monolithic_encodings_cannot_smuggle_or_discard_accepted_baselines() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let original = crate::persistence::encode(crate::persistence::diagnostic_snapshot(session.snapshot().unwrap())).unwrap();
    session.apply(INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let baseline = session.project().unwrap().board.baseline.as_ref().unwrap();
    let error = crate::persistence::encode(crate::persistence::diagnostic_snapshot(session.snapshot().unwrap())).unwrap_err();
    assert!(error.contains("accepted board baselines"), "unexpected rejection: {error}");
    for format in ["pcbcraft-project/4", "pcbcraft-project/8", "pcbcraft-project/10"] {
        let mut invalid: serde_json::Value = serde_json::from_slice(&original).unwrap();
        invalid["format"] = json!(format);
        if format != "pcbcraft-project/10" {
            invalid.as_object_mut().unwrap().remove("boards");
        }
        if format == "pcbcraft-project/4" {
            invalid.as_object_mut().unwrap().remove("numeric_id_high_water");
        }
        invalid["board"] = json!({"footprints":[], "baseline":baseline.as_ref()});
        let error = crate::persistence::decode(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
        assert!(error.contains("accepted board baselines require split board format 10"), "{format}: {error}");
    }
}
