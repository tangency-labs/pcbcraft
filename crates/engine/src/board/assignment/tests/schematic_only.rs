use super::*;

/// Capture a logical component without allocating any physical board or placement.
fn schematic() -> (Session, Value) {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let mut params = choice(&s);
    params["component"] = s.inspect()["project"]["components"][0]["id"].clone();
    (s, params)
}

/// Package intent survives both save paths with no board document. Explicit board creation
/// adopts the exact pinned choice atomically, and undo restores its original schematic scope.
#[test]
fn choices_roundtrip_without_a_board_and_follow_explicit_creation() {
    let (mut s, params) = schematic();
    let before = s.snapshot().unwrap();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    assert!(s.project().unwrap().board_target().is_none());
    assert!(s.inspect()["project"].get("board").is_none());
    assert_eq!(s.inspect()["project"]["footprint_assignments"][0]["board"], Value::Null);
    assert_eq!(s.inspect()["project"]["footprint_assignments"][0]["mapping"], params["mapping"]);
    let assigned = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(!assigned.contains_key("board.json"));
    assert!(assigned.keys().any(|path| path.starts_with("library/footprints/")));
    let entry: Value = serde_json::from_slice(&assigned["project.json"]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/14");
    assert_eq!(entry["boards"], json!([]));
    assert_eq!(entry["footprint_choices"][0]["mapping"], params["mapping"]);
    assert!(crate::persistence::encode(s.snapshot().unwrap()).is_err(), "Legacy export must not lose the choices");
    let mut incremental = crate::persistence::DocumentsEncoder::new(s.snapshot().unwrap());
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100);
        if let Some(documents) = incremental.step(1).unwrap() {
            assert_eq!(documents, assigned);
            break;
        }
    }
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let mut repeated = params.clone();
    repeated["expected_revision"] = json!(revision);
    s.apply(ASSIGN_FOOTPRINT, &repeated).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), assigned);
    // Component metadata shares the new entry-point vocabulary without changing package intent.
    let symbol = s.project().unwrap().symbols()[0].id();
    let reference = s.project().unwrap().components().next().unwrap().reference().to_owned();
    let mut metadata = crate::schematic::fields::Metadata::default();
    metadata.attributes.do_not_populate = true;
    s.apply(
        crate::SET_SYMBOL_FIELDS,
        &json!({"symbol":symbol,"reference":reference,"value":"1k","metadata":metadata,"expected_revision":s.revision()}),
    )
    .unwrap();
    let with_metadata = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&with_metadata).unwrap()).unwrap(), with_metadata);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), assigned);
    let reopened = decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project;
    assert_eq!(encode_documents(reopened.clone()).unwrap(), assigned);
    s.open_snapshot(reopened);
    let pinned = s.project().unwrap().footprint_catalog().into_iter().find(|d| d.id == "resistor-0402").unwrap();
    s.apply(crate::board::identity::CREATE_BOARD, &json!({"expected_revision":s.revision()})).unwrap();
    let target = s.project().unwrap().board_target().unwrap();
    let created = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(created.contains_key(&crate::persistence::test_board_path(&created)));
    assert!(s.project().unwrap().schematic_packages.assignments.is_empty());
    assert_eq!(s.project().unwrap().board.assignments.len(), 1);
    assert_eq!(s.project().unwrap().board.definitions.values().next().unwrap().as_ref(), pinned.as_ref());
    assert_eq!(encode_documents(decode_documents(&created).unwrap()).unwrap(), created);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(s.project().unwrap().board_target().is_none());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), assigned);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().board_target(), Some(target));
    s.apply(crate::PREVIEW_UPDATE, &json!({"board":target.board,"expected_revision":s.revision()})).unwrap();
    let preview = s.cached_board_update().unwrap();
    assert!(preview.changes.blockers.is_empty());
    let token = preview.plan_token;
    s.apply(crate::APPLY_UPDATE, &json!({"board":target.board,"plan_token":token,"expected_revision":s.revision()})).unwrap();
    s.apply(
        PLACE_ASSIGNED_FOOTPRINTS,
        &json!({"board":target.board,"components":[params["component"]],"x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    assert_eq!(s.inspect()["project"]["board"]["footprints"][0]["mapping"], params["mapping"]);
}

/// Invalid maps, board IDs, stale revisions and physical placement never create implicit
/// board ownership, reserve IDs or partially replace an existing schematic choice.
#[test]
fn invalid_schematic_choices_and_placements_are_atomic() {
    let (mut s, params) = schematic();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    let before = s.snapshot().unwrap();
    for (field, value) in [
        ("mapping", json!({"1":"1","2":"1"})),
        ("mapping", json!({"1":"3","2":"2"})),
        ("mapping", json!({"1":"1"})),
        ("component", json!("component-999999")),
        ("definition_revision", json!(999999)),
        ("expected_revision", json!(s.revision() - 1)),
        ("board", json!(uuid::Uuid::from_u128(99))),
    ] {
        let mut invalid = params.clone();
        invalid["expected_revision"] = json!(s.revision());
        invalid[field] = value;
        assert!(s.apply(ASSIGN_FOOTPRINT, &invalid).is_err(), "{invalid}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    assert!(!s.available(PLACE_ASSIGNED_FOOTPRINT));
    assert!(!s.available(PLACE_ASSIGNED_FOOTPRINTS));
    assert!(s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":params["component"],"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Deleting components cleans project-owned choices and keeps a shared pinned definition
/// until its final user is deleted. Undo restores the exact offline choices and assets.
#[test]
fn retiring_schematic_components_releases_only_unused_packages() {
    let (mut s, params) = schematic();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":10_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let component = s.project().unwrap().components().last().unwrap().id();
    let mut second = params.clone();
    second["component"] = json!(component);
    second["expected_revision"] = json!(s.revision());
    s.apply(ASSIGN_FOOTPRINT, &second).unwrap();
    let symbols: Vec<_> = s.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbols[0],"expected_revision":s.revision()})).unwrap();
    let one = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(s.project().unwrap().footprint_assignments().len(), 1);
    assert_eq!(one.keys().filter(|path| path.starts_with("library/footprints/")).count(), 1);
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbols[1],"expected_revision":s.revision()})).unwrap();
    let empty = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(s.project().unwrap().footprint_assignments().is_empty());
    assert!(!empty.keys().any(|path| path.starts_with("library/footprints/")));
    assert_eq!(serde_json::from_slice::<Value>(&empty["project.json"]).unwrap()["format"], "pcbcraft-project/14");
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&decode_documents(&one).unwrap(), &s.snapshot().unwrap());
}

/// The new vocabulary is version-gated and never repairs invalid identities, missing assets,
/// duplicate components or ambiguous board ownership by consulting installed starter parts.
#[test]
fn schematic_choice_codec_rejects_invalid_or_lossy_documents() {
    let (mut s, params) = schematic();
    s.apply(ASSIGN_FOOTPRINT, &params).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&documents["project.json"]).unwrap();
    for (field, value) in [
        ("format", json!("pcbcraft-project/9")),
        ("format", json!("pcbcraft-project/12")),
        ("footprint_choices", Value::Null),
        ("footprint_choices", json!([])),
        ("footprint_choices", json!([entry["footprint_choices"][0], entry["footprint_choices"][0]])),
        ("boards", json!([{"id":uuid::Uuid::from_u128(99),"name":"Board","file":"board.json"}])),
    ] {
        let mut invalid = documents.clone();
        let mut changed = entry.clone();
        changed[field] = value;
        invalid.insert("project.json".into(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&invalid).is_err(), "{changed}");
    }
    for field in ["numeric_id_high_water", "boards", "layout_root"] {
        let mut invalid = documents.clone();
        let mut changed = entry.clone();
        changed.as_object_mut().unwrap().remove(field);
        invalid.insert("project.json".into(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&invalid).is_err());
    }
    for (field, value) in [("component", json!(999999)), ("mapping", json!({"1":"1","2":"1"})), ("definition_revision", json!(42))] {
        let mut invalid = documents.clone();
        let mut changed = entry.clone();
        changed["footprint_choices"][0][field] = value;
        invalid.insert("project.json".into(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&invalid).is_err());
    }
    let mut missing = documents.clone();
    missing.retain(|path, _| !path.starts_with("library/footprints/"));
    assert!(decode_documents(&missing).is_err());
    let mut stray = documents;
    stray.insert("board.json".into(), br#"{"format":"pcbcraft-board/1","footprints":[]}"#.to_vec());
    assert!(decode_documents(&stray).is_err());
}
