use super::*;
use crate::board::update::{ComponentChange, TerminalKey};

/// Change one manufacturing attribute through its owning schematic unit while
/// preserving all other authored fields and attributes.
fn flag(session: &mut Session, id: u64, name: &str, enabled: bool) {
    let project = session.project().unwrap();
    let component = &project.components[&id];
    let symbol = project.all_symbols().find(|symbol| symbol.component == id).unwrap().id();
    let mut metadata = serde_json::to_value(&component.metadata).unwrap();
    metadata["attributes"][name] = json!(enabled);
    let params = json!({"symbol":symbol,"reference":component.reference,"value":component.value,"metadata":metadata});
    edit(session, crate::SET_SYMBOL_FIELDS, params);
}

/// Select an exact original catalog package using the normal assignment command.
fn assign(session: &mut Session, id: u64, name: &str) {
    let definition = crate::board::definition::starter(name).unwrap();
    edit(
        session,
        ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{id}"),
        "library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
}

/// Population and output filters are independent of physical membership. Only
/// board exclusion stages removal, and uninitialized legacy boards still refuse it.
#[test]
fn only_board_exclusion_changes_physical_membership() {
    for name in ["do_not_populate", "exclude_from_bom", "exclude_from_simulation", "exclude_from_position_files"] {
        let mut session = initialized();
        let id = session.project().unwrap().symbols[0].component;
        flag(&mut session, id, name, true);
        assert!(session.project().unwrap().component_is_board_eligible(id), "{name}");
        plan(&mut session);
        assert!(session.cached_board_update().unwrap().changes.is_noop(), "{name}");
    }
    let mut legacy = super::super::super::tests::fixture();
    let component = &legacy.project().unwrap().components.values().next().unwrap();
    let mut metadata = component.metadata.clone();
    metadata.attributes.exclude_from_board = true;
    let before = legacy.snapshot().unwrap();
    let params = json!({"symbol":legacy.project().unwrap().symbols[0].id(),"reference":component.reference,
        "value":component.value,"metadata":metadata,"expected_revision":legacy.revision()});
    assert!(legacy.apply(crate::SET_SYMBOL_FIELDS, &params).is_err());
    assert!(Arc::ptr_eq(&before, &legacy.snapshot().unwrap()));
    assert!(!legacy.project().unwrap().component_is_board_eligible(u64::MAX));
}

/// Excluded choices remain useful authored intent without becoming physical
/// pending packages. Reviewed removal retains their assets and later unexclude
/// accepts that choice as an unplaced addition, with exact save/Undo behavior.
#[test]
fn excluded_desired_choices_survive_review_roundtrip_reset_and_unexclude() {
    let mut session = initialized();
    let id = session.project().unwrap().symbols[0].component;
    let original = session.project().unwrap().board.footprints()[0].clone();
    flag(&mut session, id, "exclude_from_board", true);
    assert!(
        session
            .project()
            .unwrap()
            .boards()
            .next()
            .unwrap()
            .footprints()
            .find(|view| view.component == format!("component-{id}"))
            .unwrap()
            .pending_removal
    );
    assert_eq!(session.project().unwrap().board.footprints()[0].inspect(session.project().unwrap())["pending_removal"], true);
    assert_eq!(session.inspect()["project"]["board"]["pending_removals"], json!([format!("component-{id}")]));
    session.apply("edit.undo", &json!({})).unwrap();
    assert!(session.project().unwrap().component_is_board_eligible(id));
    session.apply("edit.redo", &json!({})).unwrap();
    assign(&mut session, id, "capacitor-0603");
    assert_eq!(session.project().unwrap().board.assignments[&id].inspect(session.project().unwrap())["pending_update"], false);
    let before_reset = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&before_reset[&crate::persistence::test_board_path(&before_reset)]).unwrap().contains("pcbcraft-board/10"));
    assert_eq!(encode_documents(decode_documents(&before_reset).unwrap()).unwrap(), before_reset);
    let mut malformed = before_reset.clone();
    let mut board: Value = serde_json::from_slice(&malformed[&crate::persistence::test_board_path(&malformed)]).unwrap();
    let assignment = board["assignments"].as_array_mut().unwrap().iter_mut().find(|assignment| assignment["component"] == id).unwrap();
    assignment["mapping"]["1"] = json!("unknown-pad");
    malformed.insert(crate::persistence::test_board_path(&malformed), serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&malformed).is_err());
    assign(&mut session, id, "resistor-0402");
    assign(&mut session, id, "capacitor-0603");
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_reset);
    assert!(
        session
            .apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":format!("component-{id}"),"x_nm":0,"y_nm":0,"expected_revision":session.revision()}))
            .is_err()
    );
    let report = plan(&mut session);
    assert!(session.cached_board_update().unwrap().changes.components.contains(&ComponentChange::Removed { component: id }));
    let before_apply = encode_documents(session.snapshot().unwrap()).unwrap();
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
    let project = session.project().unwrap();
    assert!(!project.board.by_component.contains_key(&id));
    assert_eq!(project.board.assignments[&id].definition, "capacitor-0603");
    assert!(project.board.definitions.contains_key(&project.board.assignments[&id].key()));
    assert!(!project.board.has_pending_package_intent(project));
    let after_apply = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&after_apply).unwrap()).unwrap(), after_apply);
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_apply);
    let restored = &session.project().unwrap().board.footprints()[0];
    assert_eq!((restored.id, restored.x_nm, restored.y_nm, restored.rotation), (original.id, original.x_nm, original.y_nm, original.rotation));
    session.apply("edit.redo", &json!({})).unwrap();
    session.open_snapshot(decode_documents(&after_apply).unwrap());
    plan(&mut session);
    assert!(session.cached_board_update().unwrap().changes.is_noop());
    flag(&mut session, id, "exclude_from_board", false);
    assert!(session.project().unwrap().board.has_pending_package_intent(session.project().unwrap()));
    let pending = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&pending[&crate::persistence::test_board_path(&pending)]).unwrap().contains("pcbcraft-board/11"));
    let report = plan(&mut session);
    assert!(session.cached_board_update().unwrap().changes.components.contains(&ComponentChange::Added { component: id }));
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().components[&id].package.as_ref().unwrap().definition, "capacitor-0603");
    assert!(!session.project().unwrap().board.by_component.contains_key(&id));
    edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":format!("component-{id}"),"x_nm":0,"y_nm":0}));
    assert_eq!(session.project().unwrap().board.footprints().last().unwrap().definition, "capacitor-0603");
}

/// Electrical projection filters members after resolving the full schematic graph,
/// so connections meeting at an excluded terminal still join eligible endpoints.
#[test]
fn excluded_terminals_do_not_break_shared_electrical_connections() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    for x_nm in [0, 10_000_000, 20_000_000] {
        edit(&mut session, PLACE_RESISTOR, json!({"x_nm":x_nm,"y_nm":0}));
    }
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| (symbol.id(), symbol.component)).collect();
    flag(&mut session, symbols[1].1, "exclude_from_board", true);
    assign(&mut session, symbols[0].1, "resistor-0402");
    assign(&mut session, symbols[2].1, "resistor-0402");
    for (from, to) in [(0, 1), (1, 2)] {
        edit(&mut session, CONNECT_PINS, json!({"from":{"symbol":symbols[from].0,"terminal":"1"},"to":{"symbol":symbols[to].0,"terminal":"1"}}));
    }
    // Adoption also skips unplaced excluded parts that have no package choice.
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let baseline = session.project().unwrap().board.baseline.as_ref().unwrap();
    assert_eq!(baseline.components.len(), 2);
    assert!(!baseline.components.contains_key(&symbols[1].1));
    assert_eq!(baseline.nets.len(), 1);
    assert_eq!(
        baseline.nets[0].terminals,
        vec![TerminalKey { component: symbols[0].1, terminal: "1".into() }, TerminalKey { component: symbols[2].1, terminal: "1".into() }]
    );
    let bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    plan(&mut session);
    assert!(session.cached_board_update().unwrap().changes.is_noop());
}

/// Compatibility adoption preserves actual excluded placements as its physical
/// predecessor, even if the in-memory legacy input also has another desired choice.
#[test]
fn adoption_preserves_excluded_placements_until_reviewed_removal() {
    let mut session = super::super::super::tests::fixture();
    let mut project = session.project().unwrap().clone();
    let id = project.symbols[0].component;
    Arc::make_mut(&mut project.components).get_mut(&id).unwrap().metadata.attributes.exclude_from_board = true;
    let board = Arc::make_mut(&mut project.board);
    let mut desired = board.assignments.values().find(|assignment| assignment.component != id).unwrap().clone();
    desired.component = id;
    board.assignments.insert(id, desired);
    session.open_snapshot(Arc::new(project));
    let original = session.project().unwrap().board.footprints()[0].clone();
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().components[&id].package.as_ref().unwrap().definition, original.definition);
    assert_eq!(session.project().unwrap().board.assignments[&id].definition, "resistor-0603");
    let bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    let report = plan(&mut session);
    assert!(session.cached_board_update().unwrap().changes.components.contains(&ComponentChange::Removed { component: id }));
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
    assert!(!session.project().unwrap().board.by_component.contains_key(&id));
    assert_eq!(session.project().unwrap().board.assignments[&id].definition, "resistor-0603");
}
