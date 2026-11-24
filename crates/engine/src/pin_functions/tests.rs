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
    project
        .symbol_graphics(placed)
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

/// Extend the fixture with independent, uniquely placed parts without replaying thousands of edits.
fn extend_fixture(project: &mut Project, count: usize) {
    let part = project.components().next().unwrap().clone();
    let symbol = project.symbols()[0].clone();
    for i in 1..count {
        let mut extra = part.clone();
        extra.ordinal = 2 * i as u64 + 1;
        extra.reference_number = i + 1;
        extra.reference = format!("{}{}", extra.definition.reference_prefix, i + 1);
        let mut placed = symbol.clone();
        placed.ordinal = extra.ordinal + 1;
        placed.component = extra.ordinal;
        placed.position.x_nm = (i % 64) as i64 * 10 * crate::schematic::GRID_NM;
        placed.position.y_nm = (i / 64) as i64 * 10 * crate::schematic::GRID_NM;
        assert!(placed.position.x_nm.abs().max(placed.position.y_nm.abs()) <= crate::schematic::MAX_CENTER_NM);
        Arc::make_mut(&mut project.units).insert((extra.ordinal, placed.unit_id().to_owned()), placed.ordinal);
        Arc::make_mut(&mut project.components).insert(extra.ordinal, extra);
        Arc::make_mut(&mut Arc::make_mut(&mut project.sheets)[0].symbols).push(placed);
    }
}

/// At the placement limit, the checked edit still shares untouched project data and skips
/// general edit preparation. Drawing one unit does not consult its unrelated placements.
#[test]
fn pin_function_edit_shares_snapshot_and_drawing_uses_one_owner_in_a_large_project() {
    let mut session = fixture(false);
    let mut project = session.project().unwrap().clone();
    extend_fixture(&mut project, crate::schematic::MAX_SYMBOLS);
    session.change(project).unwrap();
    let before = session.snapshot().unwrap();
    assert_eq!(before.components.len(), crate::schematic::MAX_SYMBOLS);
    let target = before.symbols().last().unwrap();
    let symbol_id = target.id();
    let target = json!({"part":target.component_id(),"terminal":"1"});
    let choice = |session: &Session, function: Value, dry_run| json!({"target":target,"function":function,"dry_run":dry_run,"expected_revision":session.revision()});
    FULL_PREPARATIONS.set(0);
    session.execute(SET, &choice(&session, json!("ADC_IN"), true)).unwrap();
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
    session.apply(SET, &choice(&session, json!("ADC_IN"), false)).unwrap();
    let selected = session.snapshot().unwrap();
    assert_eq!(selected.pin_function_count(), 1);
    assert!(Arc::ptr_eq(&selected.components, &before.components));
    assert!(Arc::ptr_eq(&selected.sheets, &before.sheets));
    assert!(Arc::ptr_eq(&selected.net_registry, &before.net_registry));
    PLACEMENT_COMPARISONS.set(0);
    let symbol = selected.symbol(&symbol_id).unwrap();
    assert!((1..=14).contains(&PLACEMENT_COMPARISONS.get()));
    OWNER_LOOKUPS.set(0);
    let graphics = selected.symbol_graphics(symbol);
    assert_eq!(OWNER_LOOKUPS.replace(0), 1);
    assert_eq!(graphics.len(), symbol.unit_definition().graphics().len());
    assert!(graphics.iter().any(|shape| matches!(shape, Shape::Graphic(crate::library_graphics::Graphic { geometry: crate::library_graphics::Geometry::Text { text, .. }, .. }) if text == "ADC_IN")));
    session.execute(SET, &choice(&session, Value::Null, true)).unwrap();
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &selected));
    session.apply(SET, &choice(&session, Value::Null, false)).unwrap();
    assert_eq!(FULL_PREPARATIONS.get(), 0);
    let cleared = session.snapshot().unwrap();
    assert_eq!(cleared.pin_function_count(), 0);
    assert!(Arc::ptr_eq(&cleared.components, &before.components));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &selected));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
}

/// A valid reopened entry point one byte under the native limit must refuse a growing
/// selection in Preview and Apply without replacing the snapshot or touching history.
#[test]
fn pin_function_preview_and_apply_refuse_native_project_overflow() {
    use crate::{persistence::MAX_DOCUMENT_BYTES, schematic::fields::Field};

    let mut session = fixture(false);
    let mut project = session.project().unwrap().clone();
    extend_fixture(&mut project, 140);
    for part in Arc::make_mut(&mut project.components).values_mut() {
        for index in 1..=64 {
            part.metadata.fields.insert(uuid::Uuid::from_u128(index).to_string(), Field { name: format!("Padding {index}"), value: String::new() });
        }
    }
    session.change(project).unwrap();
    let mut project = session.project().unwrap().clone();
    let baseline = encode_documents(Arc::new(project.clone())).unwrap()[ENTRY_POINT].len();
    let mut remaining = MAX_DOCUMENT_BYTES - 1 - baseline;
    // ASCII padding grows the canonical entry point by precisely one byte per character.
    for part in Arc::make_mut(&mut project.components).values_mut() {
        for field in part.metadata.fields.values_mut() {
            let count = remaining.min(2048);
            field.value = "x".repeat(count);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    let saved = encode_documents(Arc::new(project)).unwrap();
    assert_eq!(saved[ENTRY_POINT].len(), MAX_DOCUMENT_BYTES - 1);
    session.open_snapshot(decode_documents(&saved).unwrap());
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let mut request = params(&session, json!("ADC_IN"));
    request["dry_run"] = json!(true);
    let preview = session.execute(SET, &request).unwrap_err();
    assert!(matches!(&preview, crate::CommandError::InvalidParams(reason) if reason.contains("encoded document exceeds its size limit")));
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
    request["dry_run"] = json!(false);
    let apply = session.apply(SET, &request).unwrap_err();
    assert!(matches!(&apply, crate::CommandError::InvalidParams(reason) if reason.contains("encoded document exceeds its size limit")));
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
}

/// Matching accepted terminals use the selected metadata without changing the accepted
/// package, pad mapping, or connectivity. One Undo restores the prior export.
#[test]
fn board_netlist_uses_selected_function_without_changing_accepted_state() {
    let mut session = fixture(true);
    let package = crate::board::definition::basic_components("resistor-0402").unwrap();
    session.apply(crate::ASSIGN_FOOTPRINT, &json!({"component":part_target(&session)["part"],"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()})).unwrap();
    let preview = session.execute(crate::PREVIEW_UPDATE, &json!({"expected_revision":session.revision()})).unwrap();
    session
        .apply(crate::APPLY_UPDATE, &json!({"plan_token":preview["board_update_preview"]["plan_token"],"expected_revision":session.revision()}))
        .unwrap();
    let board_netlist = |session: &Session| {
        let project = session.project().unwrap();
        project.board_view(project.board_target().unwrap()).unwrap().netlist().unwrap().netlist
    };
    let before = board_netlist(&session);
    let mut expected = before.clone();
    let pin = expected.components[0].terminals.iter_mut().find(|pin| pin.terminal == "1").unwrap();
    assert_eq!(pin.name.as_deref(), Some("GPIO"));
    assert_eq!(pin.electrical_type, Some(ElectricalType::Bidirectional));
    assert_eq!(pin.pads.as_deref(), Some(&["1".to_owned()][..]));
    pin.name = Some("ADC_IN".into());
    pin.electrical_type = Some(ElectricalType::Input);
    session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
    assert_eq!(board_netlist(&session), expected);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(board_netlist(&session), before);
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

/// Save and reopen a pin-function project with or without a board. Selecting or clearing
/// a choice stays on /48 and only changes project.json.
#[test]
fn pin_function_storage_writes_current_format_and_refuses_invalid_records() {
    for board in [false, true] {
        let mut session = fixture(board);
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let primary: Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
        assert_eq!(primary["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        assert!(primary["components"][0].get("pin_functions").is_none());
        assert_eq!(before, encode_documents(decode_documents(&before).unwrap()).unwrap());
        let choice = params(&session, json!("ADC_IN"));
        session.apply(SET, &choice).unwrap();
        let selected = session.snapshot().unwrap();
        let docs = encode_documents(selected.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        assert_eq!(stored["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
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
        assert!(serde_json::to_vec(&stored["components"][0]["pin_functions"]).unwrap().len() <= 64);
        assert!(docs[ENTRY_POINT].len() <= before[ENTRY_POINT].len() + 256);
        for (change, expected) in [
            ("old format", "require project format 48"),
            ("empty", "Invalid alternate pin function record count"),
            ("null", "invalid type: null, expected a sequence"),
            ("unknown", "Unknown or duplicate alternate pin function selection"),
            ("duplicate", "Unknown or duplicate alternate pin function selection"),
        ] {
            let mut invalid = stored.clone();
            let component = &mut invalid["components"][0];
            match change {
                "empty" => component["pin_functions"] = json!([]),
                "null" => component["pin_functions"] = Value::Null,
                "unknown" => component["pin_functions"][0]["function"] = json!("FIXME"),
                "duplicate" => component["pin_functions"] = json!([component["pin_functions"][0], component["pin_functions"][0]]),
                _ => invalid["format"] = json!("pcbcraft-project/16"),
            }
            let mut changed = docs.clone();
            changed.insert(ENTRY_POINT.into(), serde_json::to_vec(&invalid).unwrap());
            let error = decode_documents(&changed).expect_err("invalid pin function record must be rejected");
            assert!(error.contains(expected), "{change}: {error}");
        }
        let clear = params(&session, Value::Null);
        session.apply(SET, &clear).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Version-5 placement upgrades an open flat project atomically and saves the pinned definition.
#[test]
fn pin_function_symbol_placement_adopts_current_project_storage() {
    let mut session = fixture(false);
    let placed = &session.project().unwrap().symbols()[0];
    let definition = placed.definition.clone();
    let symbol = placed.id();
    session
        .execute(crate::clipboard::COPY, &json!({"items":{"symbols":[symbol],"wires":[],"net_labels":[]},"expected_revision":session.revision()}))
        .unwrap();

    // Open the same sort of flat split project the native decoder still accepts.
    let mut blank = Session::default();
    blank.apply("project.new", &json!({"create_board":false})).unwrap();
    let mut flat = Project::clone(&crate::persistence::diagnostic_snapshot(blank.snapshot().unwrap()));
    flat.layout = None;
    let documents = crate::persistence::historical_documents(Arc::new(flat), "pcbcraft-project/9");
    session.open_snapshot(decode_documents(&documents).unwrap());
    let before = session.snapshot().unwrap();
    assert!(before.layout.is_none());
    assert!(!before.has_sheet_occurrences());
    let revision = session.revision();
    let digest = library::sources::symbol_digest(&definition).unwrap();
    let request = json!({"library":definition.library,"definition":definition.id,"definition_revision":definition.revision(),
        "definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":revision});
    let mut invalid = request.clone();
    invalid["unit"] = json!("unknown");
    assert!(session.apply(crate::PLACE_SYMBOL, &invalid).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);

    session.apply(crate::PLACE_SYMBOL, &request).unwrap();
    assert!(session.project().unwrap().supports_pin_function_storage());
    session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.symbols()[0].definition, definition);
    assert!(ink(&reopened).contains(&"ADC_IN".to_owned()));
    session.apply("edit.undo", &json!({})).unwrap();
    session.apply("edit.undo", &json!({})).unwrap();
    let undone = session.snapshot().unwrap();
    assert!(!undone.supports_pin_function_storage());
    crate::assert_same_content(&before, &undone);
}

/// Preview and apply both refuse missing current storage without modifying the project.
#[test]
fn pin_function_selection_requires_current_project_storage() {
    let mut project = fixture(false).project().unwrap().clone();
    project.layout = None;
    let mut session = Session::default();
    session.open_snapshot(Arc::new(project));
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for dry_run in [true, false] {
        let mut request = params(&session, json!("ADC_IN"));
        request["dry_run"] = json!(dry_run);
        assert!(matches!(session.execute(SET, &request), Err(CommandError::InvalidParams(message))
            if message == "Alternate pin functions require current project storage."));
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        assert_eq!(session.revision(), revision);
    }
}

/// The choice envelope preserves both membership encodings in both storage homes, and
/// assembly profiles on projects that own a board (the public profile command requires one).
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
        for (compact, profile) in [(false, false), (true, false), (false, true), (true, true)] {
            if profile && !board {
                continue;
            }
            session.open_snapshot(Arc::new(Project { prefer_compact_part_groups: compact, ..Project::clone(&grouped) }));
            if profile {
                session.apply(crate::SET_ASSEMBLY_PROFILE, &json!({"content":{"name":"Synthetic profile","revision":1,"provenance":{"kind":"authored"}},"expected_revision":session.revision()})).unwrap();
            }
            let original = session.snapshot().unwrap();
            let before = encode_documents(original.clone()).unwrap();
            let entry: Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
            assert_eq!(entry["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
            assert_eq!(entry.get("assembly_profiles").is_some(), profile);
            let path = original.instances_document_path();
            let membership_home: Value = match &path {
                Some(path) => serde_json::from_slice(&before[path]).unwrap(),
                None => entry["schematic_instances"].clone(),
            };
            assert_eq!(membership_home.get("part_group_memberships").is_some(), !compact);
            assert_eq!(membership_home.get("compact_part_group_memberships").is_some(), compact);
            let choice = params(&session, json!("ADC_IN"));
            session.apply(SET, &choice).unwrap();
            let docs = encode_documents(session.snapshot().unwrap()).unwrap();
            let selected: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
            assert_eq!(selected["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
            if let Some(path) = path {
                assert_eq!(docs[&path], before[&path]);
            } else {
                assert_eq!(selected["schematic_instances"], entry["schematic_instances"]);
            }
            assert_eq!(selected["assembly_profiles"], entry["assembly_profiles"]);
            let reopened = decode_documents(&docs).unwrap();
            assert_eq!(reopened.part_group_memberships(), original.part_group_memberships());
            assert_eq!(reopened.assembly_profiles(), original.assembly_profiles());
            assert_eq!(encode_documents(reopened).unwrap(), docs);
            session.apply("edit.undo", &Value::Null).unwrap();
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// A saved differential pair keeps its polarity and instances header when a selection
/// is added to or cleared from the pin-function project.
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
        assert_eq!(serde_json::from_slice::<Value>(&before[ENTRY_POINT]).unwrap()["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
        let docs = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&docs[ENTRY_POINT]).unwrap()["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
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

/// A pin function choice and a designator lock coexist under the current project header,
/// and both survive a save and reopen through the folder and packed codecs.
#[test]
fn pin_function_storage_with_designator_lock_keeps_the_current_header() {
    for board in [false, true] {
        let mut session = fixture(board);
        session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
        let project = session.project().unwrap();
        let placed = &project.symbols()[0];
        let fields = json!({"symbol":placed.id(),"reference":project.reference(placed),"value":project.value(placed),
            "designator_locked":true,"expected_revision":session.revision()});
        session.apply(crate::SET_SYMBOL_FIELDS, &fields).unwrap();
        let saved = session.snapshot().unwrap();
        let docs = encode_documents(saved.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        assert_eq!(stored["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        assert_eq!(stored["components"][0]["pin_functions"], json!([{"terminal":"1","function":"ADC_IN"}]));
        assert_eq!(stored["components"][0]["designator_locked"], true);
        let reopened = decode_documents(&docs).unwrap();
        assert!(ink(&reopened).contains(&"ADC_IN".to_owned()));
        assert!(reopened.components().next().unwrap().designator_locked());
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        assert_eq!(encode_documents(decode_packed(&encode_packed(saved).unwrap()).unwrap().project).unwrap(), docs);
    }
}

/// Authored electrical intent is an optional record beside pin function choices and an optional
/// designator lock: it never changes the current project header or the instance header. Save
/// and reopen preserve every feature exactly, an older pin-function header refuses the intent,
/// and clearing the intent and lock returns to the pin-function bytes exactly.
#[test]
fn pin_function_storage_with_electrical_intent_keeps_the_current_header() {
    for board in [false, true] {
        let mut session = fixture(board);
        session.apply(SET, &params(&session, json!("ADC_IN"))).unwrap();
        let label = json!({"name":"TEST_INTENT","style":"local","x_nm":50_800_000,"y_nm":0,"expected_revision":session.revision()});
        session.apply(crate::net_labels::ADD, &label).unwrap();
        let pins_only = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&pins_only[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/61");
        let project = session.project().unwrap();
        let label = project.net_labels().iter().find(|label| label.name == "TEST_INTENT").unwrap();
        let net = project
            .net_registry
            .records
            .iter()
            .find(|record| record.primary.kind == crate::nets::AnchorKind::Label && record.primary.object == label.id)
            .unwrap()
            .id;
        let address = crate::nets::NetAddress { project: project.id(), scope: project.net_scope(), net };
        let target = json!({"kind":"net","address":address});
        let volts = json!({"working_voltage":{"magnitude":{"value":"24","unit":"V"},"basis":"dc","reference":{"kind":"earth"}}});
        let intent = |session: &Session, intent: &Value| json!({"version":1,"expected_revision":session.revision(),"target":target,"intent":intent});
        session.apply(crate::nets::intent::SET, &intent(&session, &volts)).unwrap();
        let saved = session.snapshot().unwrap();
        let docs = encode_documents(saved.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        assert_eq!(stored["format"], "pcbcraft-project/61");
        assert_eq!(stored["components"][0]["pin_functions"], json!([{"terminal":"1","function":"ADC_IN"}]));
        assert!(stored["components"][0].get("designator_locked").is_none());
        if let Some(path) = saved.instances_document_path() {
            let format = |docs: &crate::persistence::Documents| serde_json::from_slice::<Value>(&docs[&path]).unwrap()["format"].clone();
            assert_eq!(format(&docs), format(&pins_only));
        }
        let mut invalid = stored.clone();
        invalid["format"] = json!("pcbcraft-project/48");
        let mut changed = docs.clone();
        changed.insert(ENTRY_POINT.into(), serde_json::to_vec(&invalid).unwrap());
        let error = decode_documents(&changed).expect_err("an older header must refuse authored intent");
        assert!(error.contains("Authored electrical intent requires pcbcraft-project/61"), "{error}");
        let reopened = decode_documents(&docs).unwrap();
        assert!(ink(&reopened).contains(&"ADC_IN".to_owned()));
        assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);
        assert_eq!(encode_documents(decode_packed(&encode_packed(saved).unwrap()).unwrap().project).unwrap(), docs);
        let mut reader = Session::default();
        reader.open_snapshot(reopened);
        let inspected = reader.execute(crate::nets::intent::INSPECT, &json!({"version":1,"target":target})).unwrap();
        assert_eq!(inspected["intent"], volts);

        let lock = |session: &mut Session, locked: bool| {
            let project = session.project().unwrap();
            let placed = &project.symbols()[0];
            let fields = json!({"symbol":placed.id(),"reference":project.reference(placed),"value":project.value(placed),
                "designator_locked":locked,"expected_revision":session.revision()});
            session.apply(crate::SET_SYMBOL_FIELDS, &fields).unwrap();
        };
        lock(&mut session, true);
        let locked = encode_documents(session.snapshot().unwrap()).unwrap();
        let stored: Value = serde_json::from_slice(&locked[ENTRY_POINT]).unwrap();
        assert_eq!(stored["format"], "pcbcraft-project/61");
        assert_eq!(stored["components"][0]["designator_locked"], true);
        let reopened = decode_documents(&locked).unwrap();
        assert!(reopened.components().next().unwrap().designator_locked());
        assert_eq!(encode_documents(reopened).unwrap(), locked);

        session.apply(crate::nets::intent::SET, &intent(&session, &json!({}))).unwrap();
        let pins_and_lock = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&pins_and_lock[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/61");
        lock(&mut session, false);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), pins_only);
    }
}

/// Edits stay on the new draft and symbol formats after removing alternates. Invalid
/// edits do not advance history; Undo still restores the original snapshot.
#[test]
fn alternate_definition_validates_and_keeps_current_formats() {
    let mut editor = editor();
    let draft = editor.snapshot();
    let encoded = draft.encode().unwrap();
    let decoded = Item::decode(&encoded).unwrap();
    assert_eq!(decoded.encode().unwrap(), encoded);
    assert_eq!(decoded.native().unwrap()["format"], "pcbcraft-symbol/5");
    let mut old: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(old["format"], "pcbcraft-library-item/5");
    old["format"] = json!("pcbcraft-library-item/1");
    let error = Item::decode(&serde_json::to_vec(&old).unwrap()).expect_err("old library item format cannot contain alternates");
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
    editor.execute(editor.revision(), Action::Replace { object: 1, primitive: primitive.clone() }).unwrap();
    let Primitive::Pin { alternates, .. } = &mut primitive else { panic!("expected pin") };
    alternates.clear();
    editor.execute(editor.revision(), Action::Replace { object: 1, primitive }).unwrap();
    let current = Item::decode(&editor.snapshot().encode().unwrap()).unwrap();
    assert_eq!(current.native().unwrap()["format"], library::SYMBOL_FORMAT_V5);
    assert_eq!(serde_json::from_slice::<Value>(&current.encode().unwrap()).unwrap()["format"], "pcbcraft-library-item/5");
    for _ in 0..3 {
        editor.execute(editor.revision(), Action::Undo).unwrap();
    }
    assert_eq!(editor.snapshot().native().unwrap()["format"], library::SYMBOL_FORMAT_V5);
}
