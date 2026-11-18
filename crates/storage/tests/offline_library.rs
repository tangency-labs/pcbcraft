//! Folder and archive editing uses pinned definitions, not installed library registrations.
//! The storage/engine path has no network transport; this does not exercise browser networking.
#![allow(clippy::unwrap_used)]

use pcbcraft_engine::{
    ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL, Project, SET_FOOTPRINT_TRANSFORM, SET_SYMBOL_FIELDS, Session,
    board::identity::CREATE_BOARD,
    library::sources::{Kind, LOAD, REMOVE},
    persistence::{Documents, encode_documents, encode_packed},
    schematic::Point,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

#[derive(Clone, Copy)]
enum Form {
    Folder,
    Archive,
}

/// Disable both delivered collections, including the starter compatibility catalog.
fn without_sources() -> Session {
    let mut session = Session::default();
    session.set_builtin_library_enabled("basic-components", false).unwrap();
    session.set_builtin_library_enabled("power", false).unwrap();
    assert!(session.libraries().inspect().as_array().unwrap().is_empty());
    assert!(session.libraries().search("", None).is_empty());
    session
}

/// Construct source documents from the repository's original definitions, not saved project JSON.
fn source_documents() -> BTreeMap<String, String> {
    let symbol = include_str!("../../../libraries/basic-components/symbols/resistor-v2.json");
    let mut footprint: Value = serde_json::from_str(include_str!("../../../libraries/basic-components/footprints/resistor-0603.json")).unwrap();
    footprint["library"] = json!("offline-demo");
    BTreeMap::from([
        (
            "library.json".into(),
            json!({"format":"pcbcraft-library/1","id":"offline-demo","name":"Offline demo","license":"MIT OR Apache-2.0"}).to_string(),
        ),
        ("symbols/resistor.json".into(), symbol.into()),
        ("footprints/resistor.json".into(), footprint.to_string()),
    ])
}

/// Apply a revision-checked edit, supplying the board ID for footprint placement and transform.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    if matches!(command, PLACE_ASSIGNED_FOOTPRINT | SET_FOOTPRINT_TRANSFORM) {
        params["board"] = json!(session.project().unwrap().board_target().unwrap().board);
    }
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Pin an external symbol/package and place physical geometry before removing its only registration.
fn fixture() -> Arc<Project> {
    let mut session = without_sources();
    session.apply("project.new", &json!({"name":"Offline library"})).unwrap();
    session.execute(LOAD, &json!({"source":"temporary-source","documents":source_documents()})).unwrap();
    let items = session.libraries().search("", Some(Kind::Symbol));
    assert_eq!(items.len(), 1);
    let symbol = items[0].1.inspect();
    edit(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":symbol["key"]["library"],"definition":symbol["key"]["id"],"definition_revision":symbol["key"]["revision"],"definition_digest":symbol["digest"],"x_nm":0,"y_nm":0}),
    );
    let part = session.project().unwrap().symbols()[0].component_id();
    let items = session.libraries().search("", Some(Kind::Footprint));
    assert_eq!(items.len(), 1);
    let package = items[0].1.inspect();
    edit(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":part,"library":package["key"]["library"],"definition":package["key"]["id"],"definition_revision":package["key"]["revision"],"mapping":{"1":"1","2":"2"}}),
    );
    edit(&mut session, CREATE_BOARD, json!({"name":"Board"}));
    edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part,"x_nm":0,"y_nm":0}));
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let inspection = session.inspect();
    session.execute(REMOVE, &json!({"source":"temporary-source"})).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.inspect(), inspection);
    assert!(session.libraries().inspect().as_array().unwrap().is_empty());
    assert!(session.libraries().search("", None).is_empty());
    let documents = encode_documents(before.clone()).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(session.project().unwrap().board_view(session.project().unwrap().board_target().unwrap()).unwrap().footprints().count(), 0);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents);
    before
}

/// Retain exact pinned document names and bytes, independently of mutable placement/field records.
fn pinned(documents: &Documents) -> Documents {
    documents.iter().filter(|(path, _)| path.starts_with("library/")).map(|(path, bytes)| (path.clone(), bytes.clone())).collect()
}

/// Read the physical placement by the part's stable identity on its explicit board.
fn pose(project: &Project, part: &str) -> (String, Point, u16) {
    let board = project.board_view(project.board_target().unwrap()).unwrap();
    let footprint = board.footprints().find(|footprint| footprint.component == part).unwrap();
    (footprint.id, footprint.position, footprint.rotation)
}

/// Publish through folder storage or the production packed codec, then use the shared disk opener.
/// Archive writes finish and close their handle before a replacement or reopen is attempted.
fn save_open(form: Form, path: &Path, project: Arc<Project>, baseline: Option<&Documents>) -> Arc<Project> {
    let documents = encode_documents(project.clone()).unwrap();
    match form {
        Form::Folder => {
            pcbcraft_storage::save(path, baseline, &documents, &mut |_| Ok(())).unwrap();
            assert_eq!(pcbcraft_storage::load(path).unwrap(), documents);
        }
        Form::Archive => fs::write(path, encode_packed(project).unwrap()).unwrap(),
    }
    let reopened = pcbcraft_storage::open(path).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    reopened
}

/// Check canonical pinned-document equality and stable identities after source-free edits in both forms.
/// Check a no-op folder save and byte-identical archive re-encoding without saving the archive again.
#[test]
fn pinned_library_folder_and_archive_remain_editable_without_registered_sources() {
    let original = fixture();
    let original_documents = encode_documents(original.clone()).unwrap();
    let copies = pinned(&original_documents);
    assert_eq!(copies.len(), 2);
    assert!(copies.contains_key("library/symbols/offline-demo/resistor.r2.json"));
    assert!(copies.contains_key("library/footprints/offline-demo/resistor-0603.r4.json"));
    assert!(!original_documents.values().any(|bytes| String::from_utf8_lossy(bytes).contains("temporary-source")));
    let symbol = &original.symbols()[0];
    assert_eq!(symbol.definition.library, "offline-demo");
    assert_eq!(symbol.definition.provenance.as_ref().unwrap().license, "MIT OR Apache-2.0");
    let symbol_id = symbol.id();
    let part_id = symbol.component_id();
    let original_pose = pose(&original, &part_id);
    assert_eq!(original_pose.1, Point { x_nm: 0, y_nm: 0 });
    assert_eq!(original_pose.2, 0);

    for (form, name) in [(Form::Folder, "folder"), (Form::Archive, "archive.pc")] {
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join(name);
        let reopened = save_open(form, &path, original.clone(), None);
        let mut session = without_sources();
        session.open_snapshot(reopened);
        assert!(session.libraries().search("", None).is_empty());
        assert_eq!(pose(session.project().unwrap(), &part_id), original_pose);
        edit(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":original_pose.0,"x_nm":3_810_000,"y_nm":-2_540_000,"rotation":90}));
        let moved = pose(session.project().unwrap(), &part_id);
        assert_eq!(moved, (original_pose.0.clone(), Point { x_nm: 3_810_000, y_nm: -2_540_000 }, 90));
        edit(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":symbol_id,"reference":"R7","value":"4.7k"}));
        let edited = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_ne!(edited, original_documents);
        session.apply("edit.undo", &Value::Null).unwrap();
        let project = session.project().unwrap();
        assert_eq!(project.component(&project.symbols()[0]).unwrap().value(), "1k");
        assert_eq!(pose(session.project().unwrap(), &part_id), moved);
        session.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), edited);
        assert_eq!(pinned(&edited), copies);
        assert!(session.libraries().inspect().as_array().unwrap().is_empty());
        assert!(session.libraries().search("", None).is_empty());

        let reopened = save_open(form, &path, session.snapshot().unwrap(), Some(&original_documents));
        let mut offline = without_sources();
        offline.open_snapshot(reopened);
        let symbol = &offline.project().unwrap().symbols()[0];
        assert_eq!((symbol.id(), symbol.component_id()), (symbol_id.clone(), part_id.clone()));
        let part = offline.project().unwrap().component(symbol).unwrap();
        assert_eq!((part.reference(), part.value()), ("R7", "4.7k"));
        assert_eq!(pose(offline.project().unwrap(), &part_id), moved);
        assert_eq!(encode_documents(offline.snapshot().unwrap()).unwrap(), edited);
        assert_eq!(pinned(&edited), copies);
        assert!(offline.libraries().search("", None).is_empty());
        match form {
            Form::Folder => {
                let mut writes = Vec::new();
                pcbcraft_storage::save(&path, Some(&edited), &edited, &mut |phase| {
                    writes.push(phase.to_owned());
                    Ok(())
                })
                .unwrap();
                assert!(writes.is_empty(), "no-op save staged writes: {writes:?}");
                assert_eq!(pcbcraft_storage::load(&path).unwrap(), edited);
            }
            Form::Archive => assert_eq!(encode_packed(offline.snapshot().unwrap()).unwrap(), fs::read(&path).unwrap()),
        }
    }
}
