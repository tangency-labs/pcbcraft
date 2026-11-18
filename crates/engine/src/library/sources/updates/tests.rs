use super::*;
use crate::{
    ASSIGN_FOOTPRINT, CommandError, PLACE_SYMBOL, Session,
    authoring::{
        Item as Draft, Kind as DraftKind,
        publishing::{new_library, prepare},
    },
    library::sources::{AVAILABLE_REVISIONS, Documents, LOAD, REMOVE, symbol_digest},
    persistence::{decode_documents, encode_documents},
};
use std::sync::Arc;

/// Publish independent synthetic symbols through the native authoring encoder.
fn symbols(last: u32) -> (Documents, Key) {
    let manifest = new_library("revision-demo", "Revision demo", "MIT OR Apache-2.0").unwrap();
    let first = prepare(Arc::new(Draft::new(DraftKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let mut draft = first.item().as_ref().clone();
    draft.name = "Later drawing".into();
    let later = prepare(Arc::new(draft), &manifest, 1, Some(1), false).unwrap();
    let mut documents = first.documents().clone();
    // Publication reserves headroom for future edits. Source reading has no such authoring
    // restriction; exercise its full revision range with an independently encoded record.
    let mut native: Value = serde_json::from_str(later.documents().values().next().unwrap()).unwrap();
    native["revision"] = json!(last);
    documents.insert(format!("symbols/later-r{last}.json"), native.to_string());
    documents.insert("library.json".into(), manifest);
    let symbol = first.item().symbol().unwrap();
    (documents, Key { library: symbol.library.clone(), kind: Kind::Symbol, id: symbol.id.clone(), revision: 1 })
}

/// Project pins are created only by the ordinary exact-digest placement command.
fn place(session: &mut Session, key: &Key) {
    let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
    session
        .apply(
            PLACE_SYMBOL,
            &json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
        "definition_digest":symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
}

/// Source loading is separate from authored project state, including in a boardless project.
fn fixture(last: u32) -> (Session, Documents, Key) {
    let (documents, key) = symbols(last);
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    (session, documents, key)
}

/// Exactly the two old instances are listed; current instances retain their pins. Querying
/// preserves bytes, snapshot ownership, revision, source generation and both history directions.
#[test]
fn available_revisions_lists_exactly_two_of_three_without_editing() {
    let (mut session, _, mut key) = fixture(2);
    place(&mut session, &key);
    place(&mut session, &key);
    key.revision = 2;
    place(&mut session, &key);
    session.apply("project.rename", &json!({"name":"Temporary name"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let before = session.snapshot().unwrap();
    let inspection = session.inspect();
    let bytes = encode_documents(before.clone()).unwrap();
    let catalog_generation = session.libraries().generation();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let instances: Vec<_> = before.components().map(|instance| instance.id()).collect();
    assert_eq!(report["outdated_instances"], json!(&instances[..2]));
    assert_eq!(report["instances"].as_array().unwrap().len(), 3);
    for row in &report["instances"].as_array().unwrap()[..2] {
        assert_eq!(row["items"][0]["status"], "Update available");
        assert_eq!(row["items"][0]["pinned"]["revision"], 1);
        assert_eq!(row["items"][0]["latest"]["revision"], 2);
        assert_eq!(row["items"][0]["latest_digest"].as_str().unwrap().len(), 64);
        assert!(row["items"][0]["diagnostic"].is_null());
    }
    assert_eq!(report["instances"][2]["items"][0]["status"], "Matches source");
    assert_eq!(report["revision"], session.revision());
    assert_eq!(report["project"], json!(before.id()));
    assert!(report["board"].is_null());
    assert_eq!(session.inspect(), inspection);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.libraries().generation(), catalog_generation);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    session.apply(AVAILABLE_REVISIONS, &Value::Null).unwrap();
    assert_eq!(session.inspect(), inspection);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(session.inspect()["project"]["name"], "Temporary name");
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// Missing, unavailable and older-only sources are unknown, not evidence of being current;
/// removing a source and reopening offline cannot manufacture an update.
#[test]
fn available_revisions_reports_unknown_sources_and_older_only_content() {
    let (mut session, documents, mut key) = fixture(2);
    key.revision = 2;
    place(&mut session, &key);
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    session.mark_library_unavailable("demo");
    let unknown = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(unknown["instances"][0]["items"][0]["status"], "Source unavailable");
    assert!(unknown["instances"][0]["items"][0]["latest"].is_null());
    assert_eq!(unknown["outdated_instances"], json!([]));
    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    let mut older = documents;
    older.retain(|path, _| !path.contains("-r2."));
    session.execute(LOAD, &json!({"source":"older","documents":older})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["instances"][0]["items"][0]["status"], "Source unavailable");
    assert_eq!(report["instances"][0]["items"][0]["latest"]["revision"], 1);
    assert_eq!(report["outdated_instances"], json!([]));
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    let report = reopened.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["instances"][0]["items"][0]["status"], "Source unavailable");
    assert_eq!(encode_documents(reopened.snapshot().unwrap()).unwrap(), saved);
}

/// A newer revision remains discoverable when the exact pin is absent from loaded sources.
#[test]
fn available_revisions_finds_newer_candidates_without_the_pinned_source() {
    let (mut session, mut documents, key) = fixture(2);
    place(&mut session, &key);
    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    documents.retain(|path, _| !path.contains("-r1."));
    session.execute(LOAD, &json!({"source":"newer-only","documents":documents})).unwrap();
    let missing = session.libraries().item(&key).unwrap_err();
    assert_eq!(missing, format!("Unavailable Symbol {}/{} r1.", key.library, key.id));
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let instance = session.project().unwrap().components().next().unwrap().id();
    assert_eq!(report["outdated_instances"], json!([instance]));
    let row = &report["instances"][0]["items"][0];
    assert_eq!(row["pinned"], json!(key));
    assert_eq!(row["status"], "Update available");
    assert_eq!(row["latest"]["revision"], 2);
    assert_eq!(row["latest_digest"].as_str().unwrap().len(), 64);
    assert!(row["diagnostic"].is_null());
}

/// A quarantined newest revision cannot fall back to an older valid update. Conflicting
/// pinned content likewise never advertises safe availability under an immutable address.
#[test]
fn available_revisions_preserves_conflicts_without_latest_fallback() {
    let (mut session, documents, key) = fixture(3);
    place(&mut session, &key);
    let mut middle = documents.clone();
    let path = middle.keys().find(|path| path.contains("-r3.")).unwrap().clone();
    let mut symbol: Value = serde_json::from_str(&middle.remove(&path).unwrap()).unwrap();
    symbol["revision"] = json!(2);
    middle.insert("symbols/middle-r2.json".into(), symbol.to_string());
    session.execute(LOAD, &json!({"source":"middle","documents":middle})).unwrap();
    let mut changed = documents.clone();
    let path = changed.keys().find(|path| path.contains("-r3.")).unwrap().clone();
    let mut symbol: Value = serde_json::from_str(&changed[&path]).unwrap();
    symbol["name"] = json!("Conflicting drawing");
    changed.insert(path, symbol.to_string());
    session.execute(LOAD, &json!({"source":"conflict","documents":changed})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["outdated_instances"], json!([]));
    let row = &report["instances"][0]["items"][0];
    assert_eq!(row["status"], "Conflict");
    assert_eq!(row["latest"]["revision"], 3);
    assert!(row["latest_digest"].is_null());
    assert!(row["diagnostic"].as_str().unwrap().contains("Conflicting Symbol"));
    session.execute(REMOVE, &json!({"source":"conflict"})).unwrap();
    let mut changed = documents;
    let path = changed.keys().find(|path| path.contains("-r1.")).unwrap().clone();
    let mut symbol: Value = serde_json::from_str(&changed[&path]).unwrap();
    symbol["name"] = json!("Conflicting pin");
    changed.insert(path, symbol.to_string());
    session.execute(LOAD, &json!({"source":"pin-conflict","documents":changed})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let row = &report["instances"][0]["items"][0];
    assert_eq!(row["status"], "Conflict");
    assert_eq!(row["pinned"]["revision"], 1);
    assert_eq!(row["latest"]["revision"], 3);
    assert_eq!(row["diagnostic"], "Pinned content conflicts with its loaded immutable address.");
    assert_eq!(report["outdated_instances"], json!([]));
}

/// Pin an owning part using an exact, attributed symbol from a registered source.
fn pinned_part_fixture() -> (Session, Documents, Key, PartDefinition) {
    use crate::{library::sources::part_digest, part_instances::PLACE_PART};

    let (mut session, mut documents, key) = fixture(2);
    let part = PartDefinition::read(
        &json!({"format":"pcbcraft-part/1","library":key.library,"id":"owned-part","revision":1,
        "name":"Pinned owning part","license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Revision query test","locator":"synthetic-native-record"},
        "symbol":{"library":key.library,"id":key.id,"revision":1},"packages":{}})
        .to_string(),
    )
    .unwrap();
    documents.insert("parts/owned-r1.json".into(), part.encode().unwrap());
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    let digest = part_digest(session.libraries().resolve_part(&part.reference()).unwrap());
    session
        .apply(
            PLACE_PART,
            &json!({"library":key.library,"definition":part.id,"definition_revision":1,
        "definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    (session, documents, key, part)
}

/// A replacement can resolve normally in the catalog but conflict with a saved dependency.
/// Neither an equal owner nor a newer owner (with its exact old source absent) hides this.
#[test]
fn available_revisions_checks_shared_dependency_content_after_source_replacement() {
    let (mut session, mut documents, key, mut part) = pinned_part_fixture();
    let snapshot = session.snapshot().unwrap();
    let saved = encode_documents(snapshot.clone()).unwrap();
    let instance = snapshot.components().next().unwrap().id();
    let symbol_path = documents.keys().find(|path| path.starts_with("symbols/") && path.contains("-r1.")).unwrap().clone();
    let original_symbol = documents[&symbol_path].clone();
    let mut changed: Value = serde_json::from_str(&original_symbol).unwrap();
    changed["name"] = json!("Different content at the same address");
    documents.insert(symbol_path.clone(), changed.to_string());

    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    for revision in [1, 2] {
        if revision == 2 {
            session.execute(REMOVE, &json!({"source":"replacement"})).unwrap();
            documents.remove("parts/owned-r1.json");
            part.revision = 2;
            documents.insert("parts/owned-r2.json".into(), part.encode().unwrap());
        }
        session.execute(LOAD, &json!({"source":"replacement","documents":documents})).unwrap();
        assert!(!session.libraries().conflicting(&key));
        assert!(session.libraries().resolve_part(&part.reference()).is_ok());
        let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
        let row = &report["instances"][0]["items"][0];
        assert_eq!(row["status"], "Conflict");
        assert_eq!(row["pinned"]["revision"], 1);
        assert_eq!(row["latest"]["revision"], revision);
        assert!(row["latest_digest"].is_null());
        assert_eq!(row["diagnostic"], format!("Pinned Symbol {}/{} r1 conflicts with its loaded immutable address.", key.library, key.id));
        assert_eq!(report["outdated_instances"], json!([]));
    }

    session.execute(REMOVE, &json!({"source":"replacement"})).unwrap();
    documents.insert(symbol_path, original_symbol);
    session.execute(LOAD, &json!({"source":"clean-newer-only","documents":documents})).unwrap();
    let old_owner = crate::library::parts::Reference { revision: 1, ..part.reference() };
    assert_eq!(session.libraries().resolve_part(&old_owner).unwrap_err(), format!("Unavailable Part {}/{} r1.", key.library, part.id));
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let row = &report["instances"][0]["items"][0];
    assert_eq!(row["status"], "Update available");
    assert_eq!(row["latest"]["revision"], 2);
    assert_eq!(row["latest_digest"].as_str().unwrap().len(), 64);
    assert!(row["diagnostic"].is_null());
    assert_eq!(report["outdated_instances"], json!([instance]));
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(snapshot).unwrap(), saved);
}

/// A quarantined pin remains the first diagnostic even when the newest part cannot resolve.
#[test]
fn available_revisions_keeps_pin_conflict_and_latest_failure_without_fallback() {
    let (mut session, mut documents, key, mut part) = pinned_part_fixture();
    let before = session.snapshot().unwrap();
    let mut changed_pin = part.clone();
    changed_pin.name = "Conflicting owning part".into();
    documents.insert("parts/owned-r1.json".into(), changed_pin.encode().unwrap());
    part.revision = 2;
    documents.insert("parts/owned-r2.json".into(), part.encode().unwrap());
    let middle = part.reference();
    part.revision = 3;
    part.symbol.as_mut().unwrap().revision = 99;
    documents.insert("parts/owned-r3.json".into(), part.encode().unwrap());
    session.execute(LOAD, &json!({"source":"conflict","documents":documents})).unwrap();
    let pin = Key { library: key.library.clone(), kind: Kind::Part, id: part.id.clone(), revision: 1 };
    assert!(session.libraries().conflicting(&pin));
    assert!(session.libraries().resolve_part(&middle).is_ok());
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let row = &report["instances"][0]["items"][0];
    assert_eq!(row["status"], "Conflict");
    assert_eq!(row["pinned"], json!(pin));
    assert_eq!(row["latest"]["revision"], 3);
    assert!(row["latest_digest"].is_null());
    assert_eq!(
        row["diagnostic"],
        format!("Pinned content conflicts with its loaded immutable address. Unavailable Symbol {}/{} r99.", key.library, key.id)
    );
    assert_eq!(report["outdated_instances"], json!([]));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Revision comparison includes the u32 boundary and never wraps to an older address.
#[test]
fn available_revisions_handles_maximum_revision_and_identity_neighbors() {
    let (mut session, mut documents, mut key) = fixture(u32::MAX);
    place(&mut session, &key);
    key.revision = u32::MAX;
    place(&mut session, &key);
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["outdated_instances"].as_array().unwrap().len(), 1);
    assert_eq!(report["instances"][0]["items"][0]["latest"]["revision"], u32::MAX);
    assert_eq!(report["instances"][1]["items"][0]["status"], "Matches source");
    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    for text in documents.values_mut().filter(|text| text.contains("pcbcraft-symbol/")) {
        let mut symbol: Value = serde_json::from_str(text).unwrap();
        symbol["id"] = json!("neighbor");
        *text = symbol.to_string();
    }
    session.execute(LOAD, &json!({"source":"neighbors","documents":documents})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["outdated_instances"], json!([]));
    assert!(report["instances"][0]["items"][0]["latest"].is_null());
}

/// Standalone assignments report their footprint independently of the symbol, before a
/// board exists, using the same pin and mapping accepted by the shared assignment command.
#[test]
fn available_revisions_includes_standalone_footprint_choices() {
    let (mut session, mut documents, key) = fixture(2);
    let manifest = documents["library.json"].clone();
    let first = prepare(Arc::new(Draft::new(DraftKind::Footprint).unwrap()), &manifest, 0, None, false).unwrap();
    let mut later = first.item().as_ref().clone();
    later.name = "Later package".into();
    let second = prepare(Arc::new(later), &manifest, 1, Some(1), false).unwrap();
    documents.extend(first.documents().clone());
    documents.extend(second.documents().clone());
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    let mut current = key;
    current.revision = 2;
    place(&mut session, &current);
    let instance = session.project().unwrap().components().next().unwrap().id();
    let footprint = first.item().footprint().unwrap();
    session
        .apply(
            ASSIGN_FOOTPRINT,
            &json!({"component":instance,"library":footprint.library,"definition":footprint.id,
        "definition_revision":1,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["outdated_instances"], json!([instance]));
    let items = report["instances"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["pinned"], json!(current));
    assert_eq!(items[0]["status"], "Matches source");
    assert_eq!(items[1]["pinned"], json!({"library":footprint.library,"kind":"footprint","id":footprint.id,"revision":1}));
    assert_eq!(items[1]["status"], "Update available");
}

/// Native parts update as owning records: later drawings alone are not independent updates.
/// The highest unresolved dependency graph is surfaced, never replaced by an older candidate.
#[test]
fn available_revisions_uses_owning_parts_and_surfaces_unresolved_latest() {
    use crate::{
        library::{
            parts::{PartDefinition, Reference},
            sources::part_digest,
        },
        part_instances::PLACE_PART,
    };
    let (mut session, mut documents, key) = fixture(2);
    let mut part = PartDefinition::read(
        &json!({"format":"pcbcraft-part/1","library":key.library,"id":"owned-part","revision":1,
        "name":"Owned part","license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Revision query test","locator":"synthetic-native-record"},
        "symbol":{"library":key.library,"id":key.id,"revision":1},"packages":{}})
        .to_string(),
    )
    .unwrap();
    documents.insert("parts/owned-r1.json".into(), part.encode().unwrap());
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    let reference = Reference { library: key.library.clone(), id: part.id.clone(), revision: 1 };
    let digest = part_digest(session.libraries().resolve_part(&reference).unwrap());
    session
        .apply(
            PLACE_PART,
            &json!({"library":key.library,"definition":part.id,"definition_revision":1,
        "definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    let before = session.snapshot().unwrap();
    let current = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(current["outdated_instances"], json!([]));
    assert_eq!(current["instances"][0]["items"].as_array().unwrap().len(), 1);
    assert_eq!(current["instances"][0]["items"][0]["pinned"]["kind"], "part");
    assert_eq!(current["instances"][0]["items"][0]["status"], "Matches source");
    part.revision = 2;
    part.name = "Later owning part".into();
    part.symbol.as_mut().unwrap().revision = 2;
    documents.insert("parts/owned-r2.json".into(), part.encode().unwrap());
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert_eq!(report["outdated_instances"], json!([before.components().next().unwrap().id()]));
    assert_eq!(report["instances"][0]["items"][0]["latest"]["revision"], 2);
    part.revision = 3;
    part.symbol.as_mut().unwrap().revision = 99;
    documents.insert("parts/owned-r3.json".into(), part.encode().unwrap());
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    let unresolved = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    let item = &unresolved["instances"][0]["items"][0];
    assert_eq!(item["status"], "Unresolved");
    assert_eq!(item["latest"]["revision"], 3);
    assert!(item["latest_digest"].is_null());
    assert!(item["diagnostic"].as_str().unwrap().contains("99"));
    assert_eq!(unresolved["outdated_instances"], json!([]));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// An unresolved highest part stays Unresolved even when it equals or predates the pin.
#[test]
fn available_revisions_surfaces_unresolved_equal_and_older_latest() {
    use crate::{
        library::{parts::PartDefinition, sources::part_digest},
        part_instances::PLACE_PART,
    };
    for pinned_revision in [1, 2] {
        let (mut session, mut documents, key) = fixture(2);
        let mut part = PartDefinition::read(
            &json!({"format":"pcbcraft-part/1","library":key.library,"id":"owned-part","revision":pinned_revision,
            "name":"Pinned owning part","license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Revision query test","locator":"synthetic-native-record"},
            "symbol":{"library":key.library,"id":key.id,"revision":1},"packages":{}})
            .to_string(),
        )
        .unwrap();
        documents.insert(format!("parts/owned-r{pinned_revision}.json"), part.encode().unwrap());
        session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
        let digest = part_digest(session.libraries().resolve_part(&part.reference()).unwrap());
        session
            .apply(
                PLACE_PART,
                &json!({"library":key.library,"definition":part.id,"definition_revision":pinned_revision,
                "definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
            )
            .unwrap();
        let before = session.snapshot().unwrap();
        part.revision = 1;
        let unresolved =
            Documents::from([("library.json".into(), documents["library.json"].clone()), ("parts/owned-r1.json".into(), part.encode().unwrap())]);
        session.execute(LOAD, &json!({"source":"demo","documents":unresolved})).unwrap();
        let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
        let row = &report["instances"][0]["items"][0];
        assert_eq!(row["pinned"]["revision"], pinned_revision);
        assert_eq!(row["latest"]["revision"], 1);
        assert_eq!(row["status"], "Unresolved");
        assert!(row["latest_digest"].is_null());
        assert_eq!(row["diagnostic"], format!("Unavailable Symbol {}/{} r1.", key.library, key.id));
        assert_eq!(report["outdated_instances"], json!([]));
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}

/// Neighboring kinds and libraries cannot match a pin with the same item ID.
#[test]
fn available_revisions_keeps_identity_namespaces_separate() {
    let (mut session, documents, key) = fixture(2);
    place(&mut session, &key);
    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    let manifest = documents["library.json"].clone();
    let mut footprint = Draft::new(DraftKind::Footprint).unwrap();
    footprint.id = key.id.clone();
    let mut neighbors = prepare(Arc::new(footprint), &manifest, 100, None, true).unwrap().documents().clone();
    neighbors.insert("library.json".into(), manifest);
    session.execute(LOAD, &json!({"source":"kind-neighbor","documents":neighbors})).unwrap();
    let mut other_library = documents.clone();
    let mut manifest: Value = serde_json::from_str(&other_library["library.json"]).unwrap();
    manifest["id"] = json!("other-library");
    other_library.insert("library.json".into(), manifest.to_string());
    session.execute(LOAD, &json!({"source":"library-neighbor","documents":other_library})).unwrap();
    let report = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap();
    assert!(report["instances"][0]["items"][0]["latest"].is_null());
    assert_eq!(report["outdated_instances"], json!([]));
    // Project-only is deliberate, not inferred from absence. Exercise the existing
    // classifier with an original local record, independent of source registrations.
    let original = super::super::NativeSource::load(&documents).unwrap();
    let mut local = original.items().iter().find(|item| item.key == key).unwrap().clone();
    local.key.library = "project-query-test".into();
    let local = inspect_revision(&Inventory::default(), &local, session.libraries());
    assert_eq!(local["status"], "Project-only");
    assert!(local["latest"].is_null());
}

/// No-project and malformed command parameters fail for the named reason without edits;
/// an empty project has no invented instance or source-current claims.
#[test]
fn available_revisions_validates_command_inputs_and_empty_projects() {
    let mut session = Session::default();
    assert!(!session.available(AVAILABLE_REVISIONS));
    let error = session.execute(AVAILABLE_REVISIONS, &json!({})).unwrap_err();
    assert!(matches!(error, CommandError::Unavailable(_)));
    assert!(error.to_string().contains("no project is open"));
    session.apply("project.new", &json!({})).unwrap();
    assert!(session.available(AVAILABLE_REVISIONS));
    let before = session.inspect();
    for params in [json!({"unexpected":true}), json!([]), json!(1), json!("bad")] {
        let error = session.execute(AVAILABLE_REVISIONS, &params).unwrap_err();
        assert!(matches!(error, CommandError::InvalidParams(_)));
        assert!(error.to_string().contains("parameter"), "{error}");
        assert_eq!(session.inspect(), before);
    }
    let report = session.execute(AVAILABLE_REVISIONS, &Value::Null).unwrap();
    assert_eq!(report["instances"], json!([]));
    assert_eq!(report["outdated_instances"], json!([]));
}
