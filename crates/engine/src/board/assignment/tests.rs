use super::*;
mod schematic_only;
use crate::{
    PLACE_RESISTOR,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};

/// Start with an unassigned component; callers must explicitly choose every terminal mapping.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
    s
}

/// Use a deliberately swapped map so tests detect accidental matching by pin number.
fn choice(s: &Session) -> Value {
    json!({"component":"component-1", "library":"basic-components", "definition":"resistor-0402",
        "definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":s.revision()})
}

/// Assignment creates no board geometry or object ID; it survives offline folder/archive reopen
/// and placement consumes only its pinned data. Undo distinguishes choosing from placing.
#[test]
fn assignment_and_placement_are_independent_offline_transactions() {
    let mut s = fixture();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let component = s.inspect()["project"]["components"][0]["id"].clone();
    let mut params = choice(&s);
    params["component"] = component.clone();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    assert!(s.project().unwrap().board.footprints().is_empty());
    let assigned = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&assigned[&crate::persistence::test_board_path(&assigned)]).unwrap().contains("pcbcraft-board/7"));
    assert_eq!(encode_documents(decode_documents(&assigned).unwrap()).unwrap(), assigned);
    let mut encoder = crate::persistence::DocumentsEncoder::new(s.snapshot().unwrap());
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100);
        if let Some(documents) = encoder.step(1).unwrap() {
            assert_eq!(documents, assigned);
            break;
        }
    }
    let mut deleted = Session::default();
    deleted.open_snapshot(s.snapshot().unwrap());
    deleted.apply(crate::DELETE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":deleted.revision()})).unwrap();
    assert!(deleted.project().unwrap().board.assignments.is_empty());
    assert!(deleted.project().unwrap().board.definitions.is_empty());
    let reopened = decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project;
    assert_eq!(encode_documents(reopened.clone()).unwrap(), assigned);
    let mut restored = Session::default();
    restored.open_snapshot(reopened);
    let placement = json!({"component":component,"x_nm":2_540_000,"y_nm":-1_270_000,"expected_revision":restored.revision()});
    restored.apply(PLACE_ASSIGNED_FOOTPRINT, &placement).unwrap();
    let stored = restored.inspect();
    assert_eq!(stored["project"]["board"]["footprints"][0]["mapping"], params["mapping"]);
    assert_eq!(stored["project"]["board"]["footprints"][0]["x_nm"], 2_540_000);
    assert_eq!(stored["project"]["schematic"], s.inspect()["project"]["schematic"]);
    restored.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&decode_documents(&assigned).unwrap(), &restored.snapshot().unwrap());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Missing/duplicated terminals or pads, stale edits and unknown component/board/source IDs
/// cannot pin data or consume history. Already placed packages cannot be silently remapped.
#[test]
fn assignment_failures_are_atomic_and_placed_remaps_require_review() {
    let mut s = fixture();
    let mut params = choice(&s);
    params["component"] = s.inspect()["project"]["components"][0]["id"].clone();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    for (field, value) in [
        ("mapping", json!({"1":"1","2":"1"})),
        ("mapping", json!({"1":"1"})),
        ("component", json!("component-999")),
        ("library", json!("missing")),
        ("definition_revision", json!(0)),
        ("board", json!("00000000-0000-0000-0000-000000000001")),
        ("expected_revision", json!(0)),
    ] {
        let mut invalid = params.clone();
        invalid[field] = value;
        assert!(s.apply(ASSIGN_FOOTPRINT, &invalid).is_err());
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    let rev = s.revision();
    params["expected_revision"] = json!(rev);
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    assert_eq!(s.revision(), rev);
    s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":params["component"],"x_nm":0,"y_nm":0,"expected_revision":rev})).unwrap();
    let placed = encode_documents(s.snapshot().unwrap()).unwrap();
    params["expected_revision"] = json!(s.revision());
    params["mapping"] = json!({"1":"1","2":"2"});
    assert!(s.apply(ASSIGN_FOOTPRINT, &params).is_err());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), placed);
    let mut corrupt: Value = serde_json::from_slice(&placed[&crate::persistence::test_board_path(&placed)]).unwrap();
    corrupt["assignments"][0]["mapping"] = params["mapping"].clone();
    let mut invalid = placed.clone();
    invalid.insert(crate::persistence::test_board_path(&invalid), serde_json::to_vec(&corrupt).unwrap());
    assert!(decode_documents(&invalid).is_err());
    corrupt["format"] = json!("pcbcraft-board/6");
    invalid.insert(crate::persistence::test_board_path(&invalid), serde_json::to_vec(&corrupt).unwrap());
    assert!(decode_documents(&invalid).is_err());
}

/// An original domain fixture pins a package absent from this build's source catalog. Encoding
/// and reopening it uses production serializers, and later placement never consults a source.
#[test]
fn unavailable_source_is_not_needed_for_pinned_assignment_or_placement() {
    let mut s = fixture();
    let mut params = choice(&s);
    params["component"] = s.inspect()["project"]["components"][0]["id"].clone();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    let mut project = s.project().unwrap().clone();
    let board = Arc::make_mut(&mut project.board);
    let mut definition = board.definitions.values().next().unwrap().as_ref().clone();
    definition.library = "offline-fixture".into();
    definition.id = "custom-passive".into();
    definition.revision = 19;
    board.definitions.clear();
    board.definitions.insert(definition.key(), Arc::new(definition));
    let assigned = board.assignments.values_mut().next().unwrap();
    assigned.library = "offline-fixture".into();
    assigned.definition = "custom-passive".into();
    assigned.definition_revision = 19;
    assert!(super::definition::basic_components("custom-passive").is_none());
    let bytes = encode_documents(Arc::new(project)).unwrap();
    s.open_snapshot(decode_documents(&bytes).unwrap());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":params["component"],"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.inspect()["project"]["board"]["footprints"][0]["definition_revision"], 19);
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":s.revision()})).unwrap();
    assert!(s.project().unwrap().board.assignments.is_empty());
    assert!(s.project().unwrap().board.definitions.is_empty());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().board.assignments.len(), 1);
}

/// Hierarchy and assignments share the incremental writer without sharing ownership: assigning
/// a child component changes only board data, while deleting its sheet removes its package.
#[test]
fn child_sheet_assignment_round_trips_and_deletes_with_its_component() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    s.apply(crate::hierarchy::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    s.apply(
        crate::hierarchy::ADD_LABEL,
        &json!({"sheet":child.id(),"name":"IN","direction":"input","x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(PLACE_RESISTOR, &json!({"sheet":child.id(),"x_nm":10_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut params = choice(&s);
    params["component"] = s.inspect()["project"]["components"][0]["id"].clone();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    let assigned = encode_documents(s.snapshot().unwrap()).unwrap();
    for (path, bytes) in &before {
        if !path.ends_with("board.json") {
            assert_eq!(assigned.get(path), Some(bytes), "{path}");
        }
    }
    let mut writer = crate::persistence::DocumentsEncoder::new(s.snapshot().unwrap());
    let mut streamed = None;
    for _ in 0..100 {
        if let Some(done) = writer.step(1).unwrap() {
            streamed = Some(done);
            break;
        }
    }
    assert_eq!(streamed, Some(assigned.clone()));
    s.open_snapshot(decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), assigned);
    s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":params["component"],"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let placed = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&placed).unwrap()).unwrap(), placed);
    s.apply(crate::sheets::DELETE_SHEET, &json!({"sheet":child.id(),"expected_revision":s.revision()})).unwrap();
    let project = s.project().unwrap();
    assert!(project.board.assignments.is_empty());
    assert!(project.board.footprints().is_empty());
    assert!(project.board.definitions.is_empty());
    decode_documents(&encode_documents(s.snapshot().unwrap()).unwrap()).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), placed);
}

/// Authored alphanumeric pads survive assignment, placement, cross-probing and offline encoding.
/// Missing pads and duplicate mappings still reject atomically before geometry is placed.
#[test]
fn alphanumeric_pad_ids_roundtrip_through_assignment_and_board_views() {
    let mut s = fixture();
    let mut definition = definition::basic_components("resistor-0402").unwrap().as_ref().clone();
    definition.library = "authoring-test".into();
    definition.id = "alpha-pads".into();
    definition.pads[0].number = "A1".into();
    definition.pads[1].number = "B_2".into();
    definition.validate().unwrap();
    let mut project = s.project().unwrap().clone();
    Arc::make_mut(&mut project.board).definitions.insert(definition.key(), Arc::new(definition.clone()));
    s.open_snapshot(Arc::new(project));
    let component = s.inspect()["project"]["components"][0]["id"].clone();
    let params = json!({"component":component,"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"B_2","2":"A1"},"expected_revision":s.revision()});
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    for mapping in [json!({"1":"A1","2":"A1"}), json!({"1":"A1","2":"MISSING"}), json!({"1":"A1","2":"bad pad"})] {
        let mut bad = params.clone();
        bad["mapping"] = mapping;
        assert!(s.apply(ASSIGN_FOOTPRINT, &bad).is_err());
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":2_540_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let snapshot = s.snapshot().unwrap();
    let target = snapshot.board_target().unwrap();
    let f = snapshot.board_view(target).unwrap().footprints().next().unwrap();
    assert_eq!(f.pads.iter().map(|p| p.number.as_str()).collect::<Vec<_>>(), ["A1", "B_2"]);
    assert!(f.pads.iter().all(|p| p.copper.is_some()));
    for terminal in snapshot.all_symbols().flat_map(|s| s.terminals()) {
        let pad = snapshot.pad_for_terminal(terminal).unwrap();
        assert_eq!(pad.inspect()["pad"], if terminal.number() == "1" { "B_2" } else { "A1" });
        assert_eq!(snapshot.terminal_for_pad(pad.on_board(target.board)), Some(terminal));
        assert!(snapshot.pad_position(pad).is_some());
    }
    let docs = encode_documents(snapshot.clone()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);
    assert_eq!(reopened.board_view(target).unwrap().footprints().next().unwrap().pads, f.pads);
    assert_eq!(encode_documents(decode_packed(&encode_packed(snapshot.clone()).unwrap()).unwrap().project).unwrap(), docs);
    assert!(crate::board::Pad::parse(&f.id, "bad pad").is_none());
}

/// The 128-pad authoring limit is supported by mappings too; numeric-looking identifiers
/// retain their spelling instead of aliasing each other through integer conversion.
#[test]
fn mapping_capacity_and_numeric_looking_pad_names_are_preserved() {
    let mut mapping = serde_json::Map::new();
    for n in 0..128 {
        mapping.insert(format!("P{n}"), json!(format!("A{n}")));
    }
    assert_eq!(crate::board::parse_mapping(&Value::Object(mapping.clone())).unwrap().len(), 128);
    mapping.insert("P128".into(), json!("A128"));
    assert!(crate::board::parse_mapping(&Value::Object(mapping)).is_none());
    let a = crate::board::Pad::parse("footprint-1", "01").unwrap();
    let b = crate::board::Pad::parse("footprint-1", "1").unwrap();
    assert_ne!(a, b);
    assert_eq!(a.inspect()["pad"], "01");
    assert_eq!(b.inspect()["pad"], "1");
}

/// Build three pinned parts with one existing placement and an accepted update baseline.
fn group_fixture() -> (Session, Vec<String>) {
    let mut session = fixture();
    let mut components = Vec::new();
    for index in 0..3 {
        if index > 0 {
            session.apply(PLACE_RESISTOR, &json!({"x_nm":index*10_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        }
        let component = session.project().unwrap().components().last().unwrap().id();
        let mut params = choice(&session);
        params["component"] = json!(component);
        session.apply(ASSIGN_FOOTPRINT, &params).unwrap();
        components.push(component);
    }
    session
        .apply(
            PLACE_ASSIGNED_FOOTPRINT,
            &json!({"component":components[0],"x_nm":-50_000_000,"y_nm":25_000_000,"expected_revision":session.revision()}),
        )
        .unwrap();
    session.apply(crate::INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    (session, components)
}

/// A group preview is disposable, predicts actual geometry and leaves an unrelated plan token
/// intact. Commit places every new part together; one Undo preserves the pre-existing placement.
#[test]
fn group_placement_preview_commit_history_and_offline_geometry_agree() {
    let (mut session, components) = group_fixture();
    session.apply(crate::PREVIEW_UPDATE, &json!({"expected_revision":session.revision()})).unwrap();
    let token = session.cached_board_update().unwrap().plan_token;
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let params = json!({"components":components[1..],"x_nm":10_000_000,"y_nm":-5_000_000,"rotation":90,"expected_revision":revision});
    let preview = session.preview_footprint_placement(&params).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.cached_board_update().unwrap().plan_token, token);
    let existing = session.inspect()["project"]["board"]["footprints"][0].clone();
    session.apply(PLACE_ASSIGNED_FOOTPRINTS, &params).unwrap();
    assert!(session.revision() > revision);
    assert!(session.revision() >= preview.highest_id());
    assert_eq!(session.inspect()["project"]["board"]["footprints"][0], existing);
    let poses =
        |project: &Project| project.boards().next().unwrap().footprints().map(|f| (f.id, f.component, f.position, f.rotation)).collect::<Vec<_>>();
    assert_eq!(poses(&preview), poses(session.project().unwrap()));
    assert_eq!(poses(&preview).len(), 3);
    assert_ne!(poses(&preview)[1].2, poses(&preview)[2].2);
    let accepted = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&accepted).unwrap()).unwrap(), accepted);
    session.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before, &session.snapshot().unwrap());
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), accepted);
}

/// Duplicate, unknown, placed, foreign, out-of-range and stale inputs reject the whole group,
/// including a valid first component. No preview or rejected commit reserves IDs or history.
#[test]
fn group_placement_invalid_requests_never_partially_place_or_consume_ids() {
    let (mut session, components) = group_fixture();
    let before = session.snapshot().unwrap();
    let params = json!({"components":components[1..],"x_nm":0,"y_nm":0,"expected_revision":session.revision()});
    for (field, value) in [
        ("components", json!([])),
        ("components", json!([components[1], components[1]])),
        ("components", json!([components[1], components[0]])),
        ("components", json!([components[1], "component-999999"])),
        ("components", json!(["component-01"])),
        ("components", json!([1])),
        ("board", json!(uuid::Uuid::nil())),
        ("rotation", json!(45)),
        ("x_nm", json!(crate::MAX_CENTER_NM)),
        ("expected_revision", json!(session.revision() - 1)),
    ] {
        let mut invalid = params.clone();
        invalid[field] = value;
        assert!(session.preview_footprint_placement(&invalid).is_err(), "{invalid}");
        assert!(session.apply(PLACE_ASSIGNED_FOOTPRINTS, &invalid).is_err(), "{invalid}");
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    let candidate = session.preview_footprint_placement(&params).unwrap();
    session.apply(PLACE_ASSIGNED_FOOTPRINTS, &params).unwrap();
    assert_eq!(session.project().unwrap().highest_id(), candidate.highest_id());
}

/// Desired assignments cannot bypass reviewed update acceptance just because the canvas has
/// a convenient group-placement command. The retained baseline remains the physical authority.
#[test]
fn group_placement_refuses_pending_package_choices() {
    let (mut session, components) = group_fixture();
    let mut change = choice(&session);
    change["component"] = json!(components[1]);
    change["mapping"] = json!({"1":"1","2":"2"});
    session.apply(ASSIGN_FOOTPRINT, &change).unwrap();
    let before = session.snapshot().unwrap();
    let params = json!({"components":components[1..],"x_nm":0,"y_nm":0,"expected_revision":session.revision()});
    assert!(session.preview_footprint_placement(&params).is_err());
    assert!(session.apply(PLACE_ASSIGNED_FOOTPRINTS, &params).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Defaults are suggestions, not assignments: matching IDs cover named or unnamed terminals,
/// incompatible/unknown inputs stay unresolved, and an exact saved custom map takes precedence.
#[test]
fn footprint_mapping_defaults_preserve_saved_choices_without_authoring_edits() {
    let mut session = fixture();
    let component = session.project().unwrap().components().next().unwrap().id();
    let package = definition::basic_components("resistor-0402").unwrap();
    let before = session.snapshot().unwrap();
    assert_eq!(
        session.project().unwrap().default_footprint_mapping(&component, &package),
        Some(BTreeMap::from([("1".into(), "1".into()), ("2".into(), "2".into())]))
    );
    assert!(session.project().unwrap().default_footprint_mapping("component-999999", &package).is_none());
    assert!(session.project().unwrap().default_footprint_mapping("component-01", &package).is_none());
    assert!(session.project().unwrap().default_footprint_mapping(&component, &definition::basic_components("sot23").unwrap()).is_none());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let mut params = choice(&session);
    params["component"] = json!(component);
    session.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    assert_eq!(
        session.project().unwrap().default_footprint_mapping(&component, &package),
        Some(BTreeMap::from([("1".into(), "2".into()), ("2".into(), "1".into())]))
    );
}

/// Exact catalog data accounts for the two supported starter LED symbol revisions' polarity;
/// unrelated two-pad packages receive ordinary numeric defaults instead of being left empty.
#[test]
fn named_pin_defaults_use_revision_qualified_library_mappings() {
    for revision in [1, 2] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        let symbol = session.libraries().symbol("basic-components", "led", revision).unwrap();
        let digest = crate::library::sources::symbol_digest(&symbol).unwrap();
        session
            .apply(
                crate::PLACE_SYMBOL,
                &json!({"library":"basic-components","definition":"led","definition_revision":revision,"definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
            )
            .unwrap();
        let component = session.project().unwrap().components().next().unwrap().id();
        let project = session.project().unwrap();
        assert_eq!(project.components().next().unwrap().definition.revision(), revision);
        assert_eq!(
            project.default_footprint_mapping(&component, &definition::basic_components("led-0402").unwrap()),
            Some(BTreeMap::from([("1".into(), "2".into()), ("2".into(), "1".into())]))
        );
        assert_eq!(
            project.default_footprint_mapping(&component, &definition::basic_components("capacitor-0402").unwrap()),
            Some(BTreeMap::from([("1".into(), "1".into()), ("2".into(), "2".into())]))
        );
        assert!(project.footprint_assignments().is_empty());
    }
}
