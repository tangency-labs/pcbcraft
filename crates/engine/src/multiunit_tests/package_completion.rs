use super::*;

/// Build a synthetic quad with four identical amplifier drawings and two shared supply pins.
fn quad() -> Session {
    quad_with_pin_order(false)
}

/// Optionally keep the second unit's drawing and pin roles while reversing its terminal order.
fn quad_with_pin_order(reverse_second: bool) -> Session {
    quad_with_alternates(reverse_second, &[])
}

/// Define the same alternate input function on the specified terminals of the quad.
fn quad_with_alternates(reverse_second: bool, alternate_inputs: &[&str]) -> Session {
    let mut symbol: Value = serde_json::from_str(SYMBOL).unwrap();
    symbol["name"] = json!("Illustrative quad amplifier");
    symbol["default_value"] = json!("Quad amplifier");
    let template = symbol["units"][0].clone();
    let mut units = Vec::new();
    let mut terminals = vec![symbol["terminals"][2].clone(), symbol["terminals"][3].clone()];
    for (index, id) in ["a", "b", "c", "d"].into_iter().enumerate() {
        let mut unit = template.clone();
        unit["id"] = json!(id);
        for (pin, name, kind) in [(0, "IN", "input"), (1, "OUT", "output")] {
            let order = if reverse_second && index == 1 { 1 - pin } else { pin };
            let number = (5 + index * 2 + order).to_string();
            unit["pins"][pin]["terminal"] = json!(number);
            terminals.push(json!({"id":number,"name":name,"electrical_type":kind}));
        }
        units.push(unit);
    }
    symbol["terminals"] = json!(terminals);
    symbol["units"] = json!(units);
    if !alternate_inputs.is_empty() {
        symbol["format"] = json!(library::SYMBOL_FORMAT_V5);
        for pin in symbol["terminals"].as_array_mut().unwrap() {
            if pin["id"].as_str().is_some_and(|id| alternate_inputs.contains(&id)) {
                pin["alternates"] = json!([{"name":"ADC_IN","electrical_type":"input"}]);
            }
        }
    }
    session_with_symbol(&symbol.to_string())
}

/// Place a separate package with the same exact original symbol, not another unit of an existing owner.
fn place_loose(session: &mut Session, index: i64, sheet: Option<SheetTarget>) {
    let digest = library::sources::symbol_digest(&session.libraries.symbol("units", "dual-amplifier", 1).unwrap()).unwrap();
    let mut params = json!({"library":"units","definition":"dual-amplifier","definition_revision":1,
        "definition_digest":digest,"x_nm":25_400_000 * index,"y_nm":0});
    if let Some(sheet) = sheet {
        params["sheet"] = json!(sheet.id());
    }
    apply(session, PLACE_SYMBOL, params);
}

/// Invoke the public command at the current revision with an optional explicit scope.
fn complete(session: &mut Session, scope: Option<&str>, dry_run: bool) -> Result<Value, CommandError> {
    let mut params = json!({"dry_run":dry_run,"expected_revision":session.revision()});
    if let Some(scope) = scope {
        params["scope"] = json!(scope);
    }
    session.execute(COMPLETE_PACKAGES, &params)
}

/// A selected alternate protects a loose owner from retirement or from acting as the destination.
#[test]
fn package_completion_skips_owners_with_selected_alternate_functions() {
    for owner_index in [0, 1] {
        let mut session = quad_with_alternates(false, &["5", "7"]);
        place_loose(&mut session, 1, None);
        let owner = session.project().unwrap().symbols()[owner_index].clone();
        apply(&mut session, crate::pin_functions::SET, json!({"target":{"part":owner.component_id(),"terminal":"5"},"function":"ADC_IN"}));
        let before = session.snapshot().unwrap();
        let saved = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        let history = (session.undo.len(), session.redo.len());
        for dry_run in [true, false] {
            let report = complete(&mut session, Some("project"), dry_run).unwrap();
            assert_eq!(report["changes"], json!([]));
            assert_eq!(report["skipped"], json!([{"part":owner.component_id(),"reason":"terminal_intent"}]));
            assert_eq!(session.revision(), revision);
            assert_eq!((session.undo.len(), session.redo.len()), history);
            assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        }
        let after = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(after, saved);
        let reopened = decode_documents(&after).unwrap();
        assert_eq!(reopened.pin_function(reopened.component(&owner).unwrap(), "5"), Some("ADC_IN"));
    }
}

/// Equivalent unselected choices allow a transfer; a unit missing the alternate cannot match.
#[test]
fn package_completion_matches_the_declared_alternate_functions() {
    for (inputs, can_merge) in [(&["5"][..], false), (&["5", "7"][..], true)] {
        let mut session = quad_with_alternates(false, inputs);
        place_loose(&mut session, 1, None);
        let before = session.snapshot().unwrap();
        let report = complete(&mut session, Some("project"), true).unwrap();
        assert_eq!(report["changes"].as_array().unwrap().len(), usize::from(can_merge));
        assert_eq!(report["skipped"], json!([]));
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
    }
}

/// Pin a command-loaded synthetic ten-pad footprint to all three complete logical terminal sets.
fn place_quad_footprints(session: &mut Session, symbols: &[SymbolPlacement]) {
    let mut footprint: Value = serde_json::from_str(include_str!("../../../../examples/native-parts/footprints/illustrative-six-pad.json")).unwrap();
    footprint["library"] = json!("units");
    footprint["id"] = json!("illustrative-ten-pad");
    footprint["name"] = json!("Illustrative ten-pad package");
    let template = footprint["pads"][0].clone();
    footprint["pads"] = json!(
        (3..=12)
            .map(|number| {
                let mut pad = template.clone();
                pad["number"] = json!(number.to_string());
                pad["center"]["x_nm"] = json!(2_000_000 * number);
                pad
            })
            .collect::<Vec<_>>()
    );
    session
        .execute(
            "library.load_source",
            &json!({"source":"quad-packages","documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
                "footprints/ten.json":footprint.to_string(),
            }}),
        )
        .unwrap();
    let mapping: std::collections::BTreeMap<_, _> = (3..=12).map(|number| (number.to_string(), number.to_string())).collect();
    for (index, symbol) in symbols.iter().enumerate() {
        apply(
            session,
            ASSIGN_FOOTPRINT,
            json!({"component":symbol.component_id(),"library":"units","definition":"illustrative-ten-pad",
            "definition_revision":1,"mapping":mapping}),
        );
        apply(session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":symbol.component_id(),"x_nm":0,"y_nm":10_000_000 * index as i64}));
    }
}

/// Opt-in moves two distinct loose packages without changing their placement identities,
/// wire geometry, net numbers or accepted physical placements. Board review sees the move.
#[test]
fn package_completion_packs_three_loose_quad_units_and_keeps_nets() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    place_loose(&mut session, 2, None);
    let symbols = session.project().unwrap().symbols().to_vec();
    place_quad_footprints(&mut session, &symbols);
    for symbol in &symbols {
        apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":symbol.id(),"terminal":"5"},"to":{"symbol":symbol.id(),"terminal":"6"}}));
    }
    let without_baseline = session.snapshot().unwrap();
    let skipped = complete(&mut session, Some("project"), false).unwrap();
    assert_eq!(skipped["skipped"].as_array().unwrap().len(), 3);
    assert!(skipped["skipped"].as_array().unwrap().iter().all(|item| item["reason"] == "board_review_required"));
    assert!(Arc::ptr_eq(&without_baseline, &session.snapshot().unwrap()));
    apply(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = session.revision();
    let undo = session.undo.len();
    let old_registry = before.net_registry.clone();
    let graph = before.electrical_connectivity();
    let old_names: Vec<_> = before.wires().iter().map(|w| graph.net_name(graph.net_index_for(w.terminals()[0]).unwrap())).collect();

    for scope in [None, Some("none")] {
        assert_eq!(complete(&mut session, scope, false).unwrap()["changes"], json!([]));
    }
    let preview = complete(&mut session, Some("project"), true).unwrap();
    assert_eq!(preview["changes"].as_array().unwrap().len(), 2);
    assert_eq!(preview["changes"][0]["remapped_terminals"], json!({"5":"7","6":"8"}));
    assert_eq!(preview["changes"][1]["remapped_terminals"], json!({"5":"9","6":"10"}));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.undo.len(), undo);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let result = complete(&mut session, Some("project"), false).unwrap();
    assert_eq!(result["changes"], preview["changes"]);
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(session.undo.len(), undo + 1);
    let after = session.snapshot().unwrap();
    assert_eq!(after.components().map(PartInstance::id).collect::<Vec<_>>(), vec![symbols[0].component_id()]);
    assert_eq!(after.symbols().iter().map(SymbolPlacement::unit_id).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    assert_eq!(after.symbols().iter().map(SymbolPlacement::id).collect::<Vec<_>>(), symbols.iter().map(SymbolPlacement::id).collect::<Vec<_>>());
    assert_eq!(after.board.footprints(), before.board.footprints());
    assert_eq!(after.board.baseline, before.board.baseline);
    let graph = after.electrical_connectivity();
    for (index, (from, to)) in before.wires().iter().zip(after.wires()).enumerate() {
        assert_eq!(to.id(), from.id());
        let actual = after.wire_path(to);
        let expected = before.wire_path(from);
        assert!(actual == expected || actual.iter().rev().eq(&expected));
        let terminals = to.terminals();
        let net = graph.net_index_for(terminals[0]).unwrap();
        assert_eq!(graph.net_index_for(terminals[1]), Some(net));
        assert_eq!(graph.net_name(net), old_names[index]);
    }
    assert_eq!(after.net_registry.records.len(), old_registry.records.len());
    for (new, old) in after.net_registry.records.iter().zip(old_registry.records.iter()) {
        assert_eq!(
            (new.id, new.generated_number, &new.state, &new.lineage, &new.class_bindings),
            (old.id, old.generated_number, &old.state, &old.lineage, &old.class_bindings)
        );
    }
    apply(&mut session, PREVIEW_UPDATE, json!({}));
    let update = session.cached_board_update().unwrap();
    assert_eq!(
        update.changes.components,
        symbols[1..].iter().map(|symbol| board::update::ComponentChange::Removed { component: symbol.component }).collect::<Vec<_>>()
    );
    assert!(update.changes.added_nets.iter().any(|net| net.terminals.iter().any(|pin| pin.component == symbols[0].component && pin.terminal == "7")));
    let saved = encode_documents(after).unwrap();
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
}

/// Completion keeps the last visible donor fields without accepting a board update.
#[test]
fn package_completion_retains_donor_footprint_display_on_reopen() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    let symbols = session.project().unwrap().symbols().to_vec();
    place_quad_footprints(&mut session, &symbols);
    apply(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let donor = &symbols[1];
    let component = donor.component;
    let sheet = session.project().unwrap().sheet_target();
    // Keep the packages equivalent: their live values match, even though both changed after acceptance.
    for (symbol, reference) in symbols.iter().zip(["U10", "U20"]) {
        apply(&mut session, SET_SYMBOL_FIELDS, json!({"sheet":sheet.id(),"symbol":symbol.id(),"reference":reference,"value":"Quad low noise"}));
    }
    let before = session.snapshot().unwrap();
    let accepted = before.board.baseline.as_ref().unwrap();
    assert_ne!(accepted.components[&component].reference, "U20");
    assert_ne!(accepted.components[&component].value, "Quad low noise");
    let result = complete(&mut session, Some("project"), false).unwrap();
    assert_eq!(result["changes"][0]["symbol"], donor.id());
    let after = session.snapshot().unwrap();
    let retained = after.board.baseline.as_ref().unwrap();
    assert_eq!(after.board.footprints(), before.board.footprints());
    assert_eq!(retained.components[&component].package, accepted.components[&component].package);
    assert_eq!(retained.components[&component].terminals, accepted.components[&component].terminals);
    assert_eq!(retained.nets, accepted.nets);
    assert_eq!(retained.source, accepted.source);

    let saved = encode_documents(after).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    let board = reopened.boards().next().unwrap();
    let footprint = board.footprints().find(|fp| fp.component == donor.component_id()).unwrap();
    assert!(footprint.pending_removal);
    assert_eq!((footprint.reference.as_ref(), footprint.value.as_ref()), ("U20", "Quad low noise"));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Reversed pin numbers sort only automatic links; authored route direction and saved bytes survive.
#[test]
fn package_completion_preserves_wire_direction_rules_on_reopen() {
    for authored in [false, true] {
        let mut session = quad_with_pin_order(true);
        place_loose(&mut session, 1, None);
        let donor = session.project().unwrap().symbols()[1].clone();
        let mut params = json!({"from":{"symbol":donor.id(),"terminal":"5"},"to":{"symbol":donor.id(),"terminal":"6"}});
        let command = if authored {
            // Pins follow the symbol's terminal list, where the supply pins come first.
            let pins = donor.pins();
            let at = |number| pins[donor.unit_definition().pin_index(number).unwrap()];
            let (from, to) = (at("5"), at("6"));
            params["bends"] = json!([{"x_nm":from.x_nm,"y_nm":from.y_nm + 2 * schematic::GRID_NM},
                {"x_nm":to.x_nm,"y_nm":to.y_nm + 2 * schematic::GRID_NM}]);
            params["auto_heal"] = json!(false);
            DRAW_WIRE
        } else {
            CONNECT_PINS
        };
        apply(&mut session, command, params);
        let before = session.snapshot().unwrap();
        let old_wire = &before.wires()[0];
        let old_route = before.wire_path(old_wire);
        let old_graph = before.electrical_connectivity();
        let old_name = old_graph.net_name(old_graph.net_index_for(old_wire.terminals()[0]).unwrap());
        let result = complete(&mut session, Some("project"), false).unwrap();
        assert_eq!(result["changes"][0]["remapped_terminals"], json!({"5":"8","6":"7"}));
        let after = session.snapshot().unwrap();
        let wire = &after.wires()[0];
        let terminals = wire.terminals();
        let numbers: Vec<_> = terminals.iter().map(Terminal::number).collect();
        let mut route = after.wire_path(wire);
        if authored {
            assert_eq!(numbers, ["8", "7"]);
        } else {
            assert_eq!(numbers, ["7", "8"]);
            route.reverse();
        }
        assert_eq!(wire.id(), old_wire.id());
        assert_eq!(wire.bends(), old_wire.bends());
        assert_eq!(route, old_route);
        let new_graph = after.electrical_connectivity();
        let index = new_graph.net_index_for(terminals[0]).unwrap();
        assert_eq!(new_graph.net_index_for(terminals[1]), Some(index));
        assert_eq!(new_graph.net_name(index), old_name);
        let docs = encode_documents(after).unwrap();
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(encode_documents(reopened).unwrap(), docs);
    }
}

/// Transfer a wired unit after opening a current project, preserving saved net and sheet identities.
#[test]
fn package_completion_preserves_saved_net_and_sheet_identities() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    let symbols = session.project().unwrap().symbols().to_vec();
    for symbol in &symbols {
        apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":symbol.id(),"terminal":"5"},"to":{"symbol":symbol.id(),"terminal":"6"}}));
    }
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    session.open_snapshot(decode_documents(&original).unwrap());
    let before = session.snapshot().unwrap();
    assert!(before.layout.is_some() && before.has_sheet_occurrences());
    let identities: Vec<_> = before.net_registry.records.iter().map(|record| (record.id, record.generated_number.unwrap())).collect();
    assert_eq!(identities.len(), 2);
    let sheet = before.sheet_target();
    let occurrence = before.sheet_occurrence(sheet).unwrap();
    let graph = before.electrical_connectivity();
    let old_names: Vec<_> = before.wires().iter().map(|wire| graph.net_name(graph.net_index_for(wire.terminals()[0]).unwrap())).collect();
    assert_ne!(old_names[0], old_names[1]);
    let revision = session.revision();
    let preview = complete(&mut session, Some("project"), true).unwrap();
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    assert_eq!(preview["changes"][0]["remapped_terminals"], json!({"5":"7","6":"8"}));
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(before).unwrap(), original);
    let applied = complete(&mut session, Some("project"), false).unwrap();
    assert_eq!(applied["changes"], preview["changes"]);
    let after = session.snapshot().unwrap();
    let saved = encode_documents(after.clone()).unwrap();
    assert!(saved.values().map(Vec::len).sum::<usize>() < 64 * 1024, "small fixture exceeded its 64 KiB save budget");
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.net_registry, after.net_registry);
    for (project, names) in [(&after, &old_names), (&reopened, &old_names)] {
        assert_eq!(project.sheet_occurrence(sheet).unwrap(), occurrence);
        assert_eq!(project.net_registry.records.iter().map(|record| (record.id, record.generated_number.unwrap())).collect::<Vec<_>>(), identities);
        let graph = project.electrical_connectivity();
        for (wire, old) in project.wires().iter().zip(names) {
            let terminals = wire.terminals();
            let net = graph.net_index_for(terminals[0]).unwrap();
            assert_eq!(&graph.net_name(net), old);
            assert_eq!(graph.net_index_for(terminals[1]), Some(net));
        }
    }
    assert_eq!(encode_documents(reopened).unwrap(), saved);
}

/// Sheet-scoped preview and Apply refuse a wired transfer if a meaningful net on
/// another sheet has no saved number, even though the transfer's own net is numbered.
#[test]
fn package_completion_refuses_unnumbered_net_on_another_sheet() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    let donor = session.project().unwrap().symbols()[1].id();
    apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":donor,"terminal":"5"},"to":{"symbol":donor,"terminal":"6"}}));
    let root = session.project().unwrap().sheet_target();
    let numbered = session.project().unwrap().net_registry.clone();
    assert_eq!(numbered.records.len(), 1);
    assert!(numbered.records[0].generated_number.is_some());

    apply(&mut session, sheets::CREATE_SHEET, json!({"name":"Remote"}));
    let remote = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, crate::net_labels::ADD, json!({"sheet":remote.id(),"name":"REMOTE","style":"local","x_nm":0,"y_nm":0}));
    assert_eq!(session.project().unwrap().net_registry.records.len(), 2);
    let params = |revision, dry_run| json!({"scope":"sheet","sheet":root.id(),"expected_revision":revision,"dry_run":dry_run});
    let preview = session.execute(COMPLETE_PACKAGES, &params(session.revision(), true)).unwrap();
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    assert_eq!(preview["changes"][0]["remapped_terminals"], json!({"5":"7","6":"8"}));

    // Keep the remote label but omit its record from an otherwise valid current project.
    let mut sparse = session.project().unwrap().clone();
    Arc::make_mut(&mut Arc::make_mut(&mut sparse.net_registry).records).retain(|record| record.id == numbered.records[0].id);
    assert_eq!(sparse.net_registry, numbered);
    let documents = encode_documents(Arc::new(sparse)).unwrap();
    session.open_snapshot(decode_documents(&documents).unwrap());
    // Seed both history stacks without changing net topology or filling the sparse registry.
    session.apply("project.rename", &json!({"name":"Before refusal"})).unwrap();
    session.apply("project.rename", &json!({"name":"Redo witness"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let before = session.snapshot().unwrap();
    assert_eq!(before.net_registry, numbered);
    let (undo, redo) = (session.undo.len(), session.redo.len());
    assert_eq!((undo, redo), (1, 1));
    let revision = session.revision();
    for dry_run in [true, false] {
        let error = session.execute(COMPLETE_PACKAGES, &params(revision, dry_run)).unwrap_err();
        assert!(
            matches!(error, CommandError::InvalidParams(ref text) if text == "Package completion requires persisted net identities and numbers."),
            "{error}"
        );
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), (undo, redo));
    }
}

/// A locked owner is neither a source nor a destination; no-connect intent and different values
/// also remain separate. Sheet scope cannot move an owner on a different sheet.
#[test]
fn package_completion_honors_locks_matching_and_sheet_scope() {
    let mut session = quad();
    for index in 1..=3 {
        place_loose(&mut session, index, None);
    }
    let original = session.project().unwrap().symbols().to_vec();
    let value = "Quad amplifier";
    let locked = original[0].clone();
    apply(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":locked.id(),"reference":"U1","value":value,"designator_locked":true}));
    apply(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":original[2].id(),"reference":"U3","value":"Different amplifier"}));
    apply(&mut session, no_connect::SET, json!({"target":{"part":original[3].component_id(),"terminal":"5"},"marked":true}));
    apply(&mut session, sheets::CREATE_SHEET, json!({"name":"Remote"}));
    let root = session.project().unwrap().sheet_target();
    let remote = session.project().unwrap().sheets().nth(1).unwrap().target;
    place_loose(&mut session, 0, Some(remote));
    let revision = session.revision();
    let local = session.execute(COMPLETE_PACKAGES, &json!({"scope":"sheet","sheet":root.id(),"expected_revision":revision})).unwrap();
    assert_eq!(local["changes"], json!([]));
    assert_eq!(
        local["skipped"],
        json!([
            {"part":locked.component_id(),"reason":"designator_locked"},
            {"part":original[3].component_id(),"reason":"terminal_intent"}
        ])
    );
    assert_eq!(session.revision(), revision);
    let result = complete(&mut session, Some("project"), false).unwrap();
    assert_eq!(result["changes"].as_array().unwrap().len(), 1);
    assert_eq!(result["changes"][0]["to_part"], original[1].component_id());
    assert_eq!(session.project().unwrap().symbols()[0], locked);
    let part = session.project().unwrap().component(&locked).unwrap();
    assert_eq!(part.reference(), "U1");
    assert!(part.designator_locked());
}

/// Established packages take precedence even when loose candidates have lower instance IDs.
#[test]
fn package_completion_fills_an_existing_package_first() {
    let mut session = quad();
    for index in 1..=2 {
        place_loose(&mut session, index, None);
    }
    let existing = session.project().unwrap().symbols()[2].component_id();
    apply(&mut session, PLACE_UNIT, json!({"component":existing,"unit":"b","x_nm":76_200_000,"y_nm":0}));
    let result = complete(&mut session, Some("sheet"), false).unwrap();
    let moves = result["changes"].as_array().unwrap();
    assert_eq!(moves.len(), 2);
    assert!(moves.iter().all(|change| change["to_part"] == existing));
    assert_eq!(session.project().unwrap().available_units(&existing).len(), 0);
}

/// A host may refuse a prepared transfer without consuming revision or history; previews
/// never ask for publication authority, and a granted transfer adds exactly one undo step.
#[test]
fn package_completion_authorizes_only_a_prepared_apply() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let undo = session.undo.len();
    let redo = session.redo.len();
    let mut params = json!({"scope":"project","expected_revision":revision,"dry_run":true});
    let refusal = CommandError::Unavailable("host withdrew publication authority".into());
    let preview = session.complete_packages_with_commit(&params, || Err(refusal.clone())).unwrap();
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    params["dry_run"] = json!(false);
    assert_eq!(session.complete_packages_with_commit(&params, || Err(refusal.clone())), Err(refusal));
    assert_eq!((session.revision(), session.undo.len(), session.redo.len()), (revision, undo, redo));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));

    let result = session.complete_packages_with_commit(&params, || Ok::<_, CommandError>(())).unwrap();
    assert_eq!(result["changes"], preview["changes"]);
    assert_eq!(result["revision"], revision + 1);
    assert_eq!(session.undo.len(), undo + 1);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Unsafe aliasing, stale revision, and invalid scope leave the same snapshot and history.
#[test]
fn package_completion_refuses_new_shared_connections_atomically() {
    let mut session = quad();
    place_loose(&mut session, 1, None);
    let donor = session.project().unwrap().symbols()[1].id();
    apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":donor,"terminal":"5"},"to":{"symbol":donor,"terminal":"3"}}));
    let revision = session.revision();
    let before = session.snapshot().unwrap();
    let undo = session.undo.len();
    for (params, expected_message) in [
        (json!({"scope":"project","expected_revision":revision-1}), None),
        (json!({"scope":"all","expected_revision":revision}), Some("`scope` must be none, sheet or project")),
        (json!({"scope":"none","sheet":before.sheet_target().id(),"expected_revision":revision}), Some("`sheet` is only allowed with sheet scope.")),
        (json!({"scope":"project","expected_revision":revision}), Some("would change connectivity")),
        (json!({"scope":"project","expected_revision":revision,"dry_run":true}), Some("would change connectivity")),
    ] {
        let error = session.execute(COMPLETE_PACKAGES, &params).unwrap_err();
        match expected_message {
            None => assert!(matches!(error, CommandError::StaleRevision { current } if current == revision), "{error}"),
            Some(expected) => assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains(expected)), "{error}"),
        }
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
        assert_eq!(session.undo.len(), undo);
    }
}

/// A read-only session refuses even a valid dry run, because the same command applies transfers.
#[test]
fn package_completion_is_refused_in_a_read_only_session() {
    let mut editor = quad();
    for index in 1..=2 {
        place_loose(&mut editor, index, None);
    }
    let project = editor.snapshot().unwrap();
    assert_eq!(complete(&mut editor, Some("project"), true).unwrap()["changes"].as_array().unwrap().len(), 2);
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    reader.open_snapshot(project.clone());
    let before = reader.inspect();
    for dry_run in [true, false] {
        let params = json!({"scope":"project","dry_run":dry_run,"expected_revision":reader.revision()});
        let executed = reader.execute(COMPLETE_PACKAGES, &params).unwrap_err();
        let applied = reader.apply(COMPLETE_PACKAGES, &params).unwrap_err();
        for error in [executed, applied] {
            assert_eq!(error.code(), error_codes::ErrorCode::OutOfScope, "{error}");
            assert_eq!(error.details(), json!({"command":COMPLETE_PACKAGES,"scope":"read_only"}));
        }
    }
    assert_eq!(reader.inspect(), before);
    assert!(Arc::ptr_eq(&project, &reader.snapshot().unwrap()));
}

/// Detached preparation predicts the post-commit revision before publication, stops cooperatively
/// on cancellation, and refuses to commit once the session advances.
#[test]
fn prepared_package_completion_rejects_stale_and_cancelled_runs_without_mutating_history() {
    use std::sync::atomic::AtomicBool;

    let mut session = quad();
    place_loose(&mut session, 1, None);
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let undo = session.undo.len();
    let params = json!({"scope":"project","expected_revision":revision});

    let cancelled = session.prepare_package_completion(&params).unwrap();
    assert!(cancelled.run_cancellable(&AtomicBool::new(true)).unwrap().is_none());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len()), (revision, undo));

    let prepared = session.prepare_package_completion(&params).unwrap().finish().unwrap();
    assert!(prepared.is_effectful());
    assert_eq!(prepared.report()["revision"], revision + 1);
    assert_eq!(prepared.report()["changes"].as_array().unwrap().len(), 1);
    assert_eq!((session.revision(), session.undo.len()), (revision, undo));

    let stale_job = session.prepare_package_completion(&params).unwrap().finish().unwrap();
    session.apply("project.rename", &json!({"name":"Concurrent edit"})).unwrap();
    let after_rename = session.snapshot().unwrap();
    let renamed_rev = session.revision();
    let renamed_undo = session.undo.len();
    let err = session.commit_package_completion(stale_job).unwrap_err();
    assert!(matches!(err, CommandError::StaleRevision { current } if current == renamed_rev), "{err}");
    assert!(Arc::ptr_eq(&after_rename, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len()), (renamed_rev, renamed_undo));

    let current_params = json!({"scope":"project","expected_revision":renamed_rev});
    let current = session.prepare_package_completion(&current_params).unwrap().finish().unwrap();
    let expected_report = current.report().clone();
    let committed = session.commit_package_completion(current).unwrap();
    assert_eq!(committed, expected_report);
    assert_eq!(session.revision(), renamed_rev + 1);
    assert_eq!(session.undo.len(), renamed_undo + 1);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after_rename, &session.snapshot().unwrap()));
}
