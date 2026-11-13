use super::*;
use crate::persistence::{decode_documents, encode_documents};

const SYMBOL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../libraries/starter/symbols/resistor-v2.json"));
const FOOTPRINT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../libraries/starter/footprints/resistor-0603.json"));

/// Original project-owned definitions with a distinct native library identity and retained license.
fn documents() -> Documents {
    let mut footprint: Value = serde_json::from_str(FOOTPRINT).unwrap();
    footprint["library"] = json!("local-demo");
    BTreeMap::from([
        (
            "library.json".into(),
            json!({"format":"pcbcraft-library/1","id":"local-demo","name":"Local demo","license":"MIT OR Apache-2.0"}).to_string(),
        ),
        ("symbols/resistor.json".into(), SYMBOL.into()),
        ("footprints/resistor.json".into(), footprint.to_string()),
    ])
}

/// Source commands preserve the project pointer/history; only an explicit placement creates an undo step.
#[test]
fn source_commands_place_and_pin_offline_through_the_existing_command_path() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    session.execute(LOAD, &json!({"source":"personal","documents":documents()})).unwrap();
    let search = session.execute(SEARCH, &json!({"query":"RESISTOR"})).unwrap();
    assert_eq!(search.as_array().unwrap().len(), 5); // external symbol/footprint and built-in symbol/0402/0603
    assert!(search.as_array().unwrap().iter().any(|item| item["key"]["kind"] == "footprint" && item["source"] == "personal"));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(revision, session.revision());
    session
        .apply(
            crate::PLACE_SYMBOL,
            &json!({"library":"local-demo","definition":"resistor","definition_revision":2,"definition_digest":session.libraries().search("",Some(Kind::Symbol))[0].1.inspect()["digest"],"x_nm":0,"y_nm":0,"expected_revision":revision}),
        )
        .unwrap();
    let pinned = encode_documents(session.snapshot().unwrap()).unwrap();
    let state = session.snapshot().unwrap();
    session.execute(REMOVE, &json!({"source":"personal"})).unwrap();
    assert!(Arc::ptr_eq(&state, &session.snapshot().unwrap()));
    let reopened = decode_documents(&pinned).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), pinned);
    let symbol = &reopened.symbols()[0].definition;
    assert_eq!(symbol.library, "local-demo");
    assert_eq!(symbol.provenance.as_ref().unwrap().license, "MIT OR Apache-2.0");
    assert!(!String::from_utf8(pinned.values().flatten().copied().collect()).unwrap().contains("personal"));
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &session.snapshot().unwrap());
}

/// Bundled registrations reject mutation without changing project/catalog state; protection
/// follows their delivery origin, while external registrations remain removable.
#[test]
fn bundled_sources_are_read_only_without_changing_project_or_catalog() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let project = session.snapshot().unwrap();
    let revision = session.revision();
    let catalog = session.libraries().inspect();
    let generation = session.libraries().generation();
    let bytes = encode_documents(project.clone()).unwrap();
    for source in ["bundled-starter", "bundled-power"] {
        assert!(session.execute(REMOVE, &json!({"source":source})).unwrap_err().to_string().contains("read-only"));
        assert!(session.execute(LOAD, &json!({"source":source,"documents":documents()})).unwrap_err().to_string().contains("read-only"));
        session.mark_library_unavailable(source);
        assert_eq!(session.libraries().inspect(), catalog);
        assert_eq!(session.libraries().generation(), generation);
        assert!(catalog.as_array().unwrap().iter().any(|entry| entry["source"] == source && entry["builtin"] == true));
    }
    session.execute(LOAD, &json!({"source":"bundled-personal","documents":documents()})).unwrap();
    assert!(session.libraries().inspect().as_array().unwrap().iter().any(|entry| entry["source"] == "bundled-personal" && entry["builtin"] == false));
    session.execute(REMOVE, &json!({"source":"bundled-personal"})).unwrap();
    assert_eq!(session.libraries().inspect(), catalog);
    assert!(Arc::ptr_eq(&project, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// Invalid files are isolated, malformed manifests/transport bounds fail, and upload/folder data agree.
#[test]
fn native_validation_reports_paths_and_bounds_before_adoption() {
    let mut files = documents();
    files.insert("symbols/bad.json".into(), "no JSON".into());
    let loaded = NativeSource::load(&files).unwrap();
    assert_eq!(loaded.items.len(), 2);
    assert!(loaded.problems[0].contains("symbols/bad.json"));
    let bundle = json!({"format":"pcbcraft-library-bundle/1","documents":files}).to_string();
    assert_eq!(NativeSource::from_bundle(bundle.as_bytes()).unwrap().items.len(), 2);
    let duplicate = br#"{"format":"pcbcraft-library-bundle/1","documents":{"library.json":"a","library.json":"b"}}"#;
    assert!(NativeSource::from_bundle(duplicate).unwrap_err().contains("duplicate"));
    let too_many: Documents = (0..=MAX_FILES).map(|i| (format!("symbols/file-{i}.json"), String::new())).collect();
    let bundle = json!({"format":"pcbcraft-library-bundle/1","documents":too_many}).to_string();
    assert!(NativeSource::from_bundle(bundle.as_bytes()).unwrap_err().contains("Too many"));
    let mut files = documents();
    files.insert("symbols/../../escape.json".into(), SYMBOL.into());
    assert!(NativeSource::load(&files).is_err());
    let mut files = documents();
    files.insert("library.json".into(), "bad".into());
    assert!(NativeSource::load(&files).is_err());
    let mut files = documents();
    files.insert("symbols/large.json".into(), " ".repeat(MAX_FILE_BYTES + 1));
    assert!(NativeSource::load(&files).is_err());
}

/// No source ordering or filename may silently resolve conflicting content at the same revision.
#[test]
fn conflicts_are_quarantined_within_between_and_across_source_refreshes() {
    let files = documents();
    let mut changed = files.clone();
    let mut symbol: Value = serde_json::from_str(SYMBOL).unwrap();
    symbol["default_value"] = json!("2k");
    changed.insert("symbols/resistor.json".into(), symbol.to_string());
    let mut catalog = Catalog::default();
    catalog.install("first", Arc::new(NativeSource::load(&files).unwrap())).unwrap();
    catalog.install("second", Arc::new(NativeSource::load(&files).unwrap())).unwrap();
    assert_eq!(catalog.search("", Some(Kind::Symbol)).len(), 1);
    catalog.install("second", Arc::new(NativeSource::load(&changed).unwrap())).unwrap();
    assert!(catalog.search("", Some(Kind::Symbol)).is_empty());
    assert!(catalog.inspect().to_string().contains("conflicting"));
    catalog.remove("second").unwrap();
    assert_eq!(catalog.search("", Some(Kind::Symbol)).len(), 1);
    catalog.install("first", Arc::new(NativeSource::load(&changed).unwrap())).unwrap();
    assert!(catalog.search("", Some(Kind::Symbol)).is_empty());
    let mut within = files;
    within.insert("symbols/duplicate.json".into(), symbol.to_string());
    let mut catalog = Catalog::default();
    catalog.install("within", Arc::new(NativeSource::load(&within).unwrap())).unwrap();
    assert!(catalog.search("", Some(Kind::Symbol)).is_empty());
    assert_eq!(catalog.search("", Some(Kind::Footprint)).len(), 1);
}

/// Failed/stale placements and conflicting pinned definitions leave the exact project/history intact.
#[test]
fn source_refresh_cannot_replace_pinned_geometry_or_bypass_revision_checks() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let files = documents();
    session.execute(LOAD, &json!({"source":"source","documents":files})).unwrap();
    let revision = session.revision();
    let mut params = json!({"library":"local-demo","definition":"resistor","definition_revision":2,"definition_digest":session.libraries().search("",Some(Kind::Symbol))[0].1.inspect()["digest"],"x_nm":0,"y_nm":0,"expected_revision":revision});
    session.apply(crate::PLACE_SYMBOL, &params).unwrap();
    let pinned = session.snapshot().unwrap();
    assert!(session.apply(crate::PLACE_SYMBOL, &params).is_err());
    session.execute(REMOVE, &json!({"source":"source"})).unwrap();
    let mut files = documents();
    let mut symbol: Value = serde_json::from_str(SYMBOL).unwrap();
    symbol["default_value"] = json!("2k");
    files.insert("symbols/resistor.json".into(), symbol.to_string());
    session.execute(LOAD, &json!({"source":"source","documents":files})).unwrap();
    params["expected_revision"] = json!(session.revision());
    assert!(session.apply(crate::PLACE_SYMBOL, &params).is_err());
    params["definition_digest"] = session.libraries().search("", Some(Kind::Symbol))[0].1.inspect()["digest"].clone();
    assert!(session.apply(crate::PLACE_SYMBOL, &params).unwrap_err().to_string().contains("pinned project copy"));
    assert!(Arc::ptr_eq(&pinned, &session.snapshot().unwrap()));
}

/// Omitted external identity guards report the missing parameter before lookup or digest checks;
/// rejected calls preserve the exact document, revision and undo history.
#[test]
fn external_symbol_placement_names_missing_revision_and_digest_parameters() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.execute(LOAD, &json!({"source":"personal","documents":documents()})).unwrap();
    let before = session.snapshot().unwrap();
    let state = session.inspect();
    let params = json!({
        "library":"local-demo", "definition":"resistor", "definition_revision":2,
        "definition_digest":session.libraries().search("",Some(Kind::Symbol))[0].1.inspect()["digest"],
        "x_nm":0, "y_nm":0, "expected_revision":session.revision()
    });
    for name in ["definition_revision", "definition_digest"] {
        let mut missing = params.clone();
        missing.as_object_mut().unwrap().remove(name);
        let error = session.apply(crate::PLACE_SYMBOL, &missing).unwrap_err();
        assert!(
            matches!(&error, crate::CommandError::InvalidParams(message) if message == &format!("`{name}` is required for explicit library selections")),
            "{error}"
        );
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.inspect(), state);
    }
    session.apply(crate::PLACE_SYMBOL, &params).unwrap();
    assert_eq!(session.project().unwrap().symbols().len(), 1);
}
