use super::*;
use pcbcraft_engine::authoring::{
    Action, Item, Kind as EditorKind, Session,
    publishing::{new_library, prepare},
};
use serde_json::json;
use std::sync::Arc;

/// Native publication survives a fresh load and retains every immutable revision and item identity.
#[test]
fn publish_restart_reopen_and_publish_again_preserves_older_revisions() {
    for kind in [EditorKind::Symbol, EditorKind::Footprint] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
        create(&root, &manifest).unwrap();
        let first = prepare(Arc::new(Item::new(kind).unwrap()), &manifest, 0, None, false).unwrap();
        publish(&root, &first).unwrap();
        confirm(&root, &first).unwrap();
        if kind == EditorKind::Symbol {
            assert_eq!(first.item().export_native().unwrap()["format"], "pcbcraft-symbol/5");
        }
        let first_bytes = first.documents().keys().map(|path| (path.clone(), std::fs::read(root.join(path)).unwrap())).collect::<Vec<_>>();
        let loaded = super::super::load(&root).unwrap();
        let record = &loaded.items()[0];
        let reopened = if kind == EditorKind::Symbol {
            let symbol = record.symbol().unwrap();
            assert_eq!(symbol.format(), "pcbcraft-symbol/5");
            Item::edit_symbol(&symbol).unwrap()
        } else {
            Item::edit_footprint(&record.footprint().unwrap()).unwrap()
        };
        assert_eq!(reopened.id, first.item().id);
        let mut session = Session::new(reopened, true).unwrap();
        session.bind_published(record.symbol());
        session.execute(0, Action::Delete { objects: vec![3] }).unwrap();
        let second = prepare(session.snapshot(), &manifest, first.item().revision, Some(first.item().revision), false).unwrap();
        publish(&root, &second).unwrap();
        session.confirm_publication(&second);
        assert!(!session.dirty());
        let loaded = super::super::load(&root).unwrap();
        assert_eq!(loaded.items().len(), 2);
        assert!(loaded.items().iter().all(|record| record.key.id == first.item().id));
        assert!(second.item().revision > first.item().revision);
        for (path, bytes) in first_bytes {
            assert_eq!(std::fs::read(root.join(path)).unwrap(), bytes);
        }
        assert!(publish(&root, &first).is_err());
        assert!(create(&root, &manifest).is_err());
    }
}

/// Publishing an edited historical symbol preserves its old file and existing project pin.
#[test]
fn publish_legacy_symbol_as_current_and_reopen_both_revisions() {
    use pcbcraft_engine::{
        PLACE_SYMBOL, Session as ProjectSession,
        library::{
            SYMBOL_FORMAT_V5,
            sources::{Documents, symbol_digest},
        },
        persistence::{decode_documents, encode_documents},
    };

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    create(&root, &manifest).unwrap();
    let path = "symbols/gnd.json";
    let original = include_str!("../../../../../libraries/power/symbols/gnd.json");
    std::fs::create_dir(root.join("symbols")).unwrap();
    std::fs::write(root.join(path), original).unwrap();
    let mut documents = Documents::from([("library.json".into(), manifest.clone()), (path.into(), original.into())]);
    let mut project = ProjectSession::default();
    project.apply("project.new", &json!({"create_board":false})).unwrap();
    project.execute("library.load_source", &json!({"source":"workshop","documents":documents})).unwrap();
    let old = project.libraries().symbol("workshop", "gnd", 1).unwrap();
    project
        .apply(
            PLACE_SYMBOL,
            &json!({"library":"workshop","definition":"gnd","definition_revision":1,
        "definition_digest":symbol_digest(&old).unwrap(),"x_nm":0,"y_nm":0,"expected_revision":project.revision()}),
        )
        .unwrap();

    let mut item = Item::edit_symbol(&old).unwrap();
    item.name = "Local ground".into();
    let prepared = prepare(Arc::new(item), &manifest, 1, Some(1), false).unwrap();
    publish(&root, &prepared).unwrap();
    assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), original);
    let loaded = super::super::load(&root).unwrap();
    assert!(loaded.problems().is_empty());
    let revision = prepared.item().revision;
    let new = loaded.items().iter().find(|record| record.key.revision == revision).unwrap().symbol().unwrap();
    assert_eq!(new.format(), SYMBOL_FORMAT_V5);
    assert_eq!(new.name, "Local ground");
    assert_eq!(new.units(), old.units());
    assert_eq!(new.pins(), old.pins());
    assert_eq!(new.power_terminal(), Some("1"));
    assert_eq!(new.defaults, old.defaults);

    documents.extend(prepared.documents().clone());
    project.execute("library.load_source", &json!({"source":"workshop","documents":documents})).unwrap();
    project
        .apply(
            PLACE_SYMBOL,
            &json!({"library":"workshop","definition":"gnd","definition_revision":revision,
        "definition_digest":symbol_digest(&new).unwrap(),"x_nm":25_400_000,"y_nm":0,"expected_revision":project.revision()}),
        )
        .unwrap();
    let saved = encode_documents(project.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    let pins: Vec<_> = reopened.components().map(|part| part.definition.clone()).collect();
    assert_eq!(old.format(), "pcbcraft-symbol/2");
    assert_eq!(pins, [old, new]);
}

/// Republishing an older footprint retains its exact file and publishes matching land geometry.
#[test]
fn footprint_publication_keeps_legacy_revision_and_uses_current_format() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    create(&root, &manifest).unwrap();
    let mut legacy: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../libraries/basic-components/footprints/resistor-0402.json")).unwrap();
    legacy["library"] = json!("workshop");
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    let path = root.join("footprints/resistor-0402.json");
    std::fs::create_dir(root.join("footprints")).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let source = super::super::load(&root).unwrap();
    let old = source.items()[0].footprint().unwrap();
    assert_eq!(old.format, "pcbcraft-footprint/1");

    let mut item = Item::edit_footprint(&old).unwrap();
    item.name = "Workshop resistor".into();
    let prepared = prepare(Arc::new(item), &manifest, old.revision, Some(old.revision), false).unwrap();
    publish(&root, &prepared).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);

    let source = super::super::load(&root).unwrap();
    assert!(source.problems().is_empty());
    let mut catalog = pcbcraft_engine::library::sources::Catalog::default();
    catalog.install("workshop", Arc::new(source)).unwrap();
    let retained = catalog.footprint("workshop", &old.id, old.revision).unwrap();
    let new = catalog.footprint("workshop", &old.id, prepared.item().revision).unwrap();
    assert_eq!(retained, old);
    assert_eq!(new.format, "pcbcraft-footprint/7");
    assert!(new.orientation.is_none());
    assert_eq!(new.pads, old.pads);
}

/// A revision created after preparation is never overwritten, including after a second publisher races.
#[test]
fn raced_revision_and_replaced_manifest_preserve_draft_and_original() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    create(&root, &manifest).unwrap();
    let mut session = Session::new(Item::new(EditorKind::Footprint).unwrap(), false).unwrap();
    let snapshot = session.snapshot();
    let first = prepare(snapshot.clone(), &manifest, 0, None, false).unwrap();
    publish(&root, &first).unwrap();
    assert!(matches!(publish(&root, &first), Err(Failure::BeforeWrite(_))));
    assert!(Arc::ptr_eq(&session.snapshot(), &snapshot));
    assert!(session.dirty());
    let other = new_library("replacement", "Replacement", "MIT").unwrap();
    std::fs::write(root.join("library.json"), other).unwrap();
    assert!(matches!(publish(&root, &first), Err(Failure::BeforeWrite(_))));
    assert!(confirm(&root, &first).is_err());
    session.execute(0, Action::Delete { objects: vec![3] }).unwrap();
    assert!(session.dirty());
    assert_eq!(session.snapshot().library, "user");
}

/// A crash before final publication leaves only an ignored root stage, and retry uses another stage.
#[test]
fn interrupted_stage_does_not_poison_source_or_prevent_retry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    create(&root, &manifest).unwrap();
    let prepared = prepare(Arc::new(Item::new(EditorKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let path = prepared.documents().keys().next().unwrap();
    let name = path.split_once('/').unwrap().1;
    let stage = root.join(format!(".pcbcraft-{name}-0.pending"));
    std::fs::write(&stage, b"interrupted").unwrap();
    publish(&root, &prepared).unwrap();
    assert_eq!(std::fs::read(stage).unwrap(), b"interrupted");
    assert!(super::super::load(&root).unwrap().problems().is_empty());
}

/// Destination links cannot redirect native revision writes or borrow an unrelated source root.
#[cfg(unix)]
#[test]
fn publication_refuses_symlink_roots_directories_and_locks() {
    for obstruction in ["root", "symbols", "lock"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
        create(&root, &manifest).unwrap();
        let prepared = prepare(Arc::new(Item::new(EditorKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
        match obstruction {
            "root" => {
                std::fs::rename(&root, temp.path().join("original")).unwrap();
                std::os::unix::fs::symlink(&outside, &root).unwrap();
            }
            "symbols" => {
                std::os::unix::fs::symlink(&outside, root.join("symbols")).unwrap();
            }
            _ => {
                std::fs::write(outside.join("lock"), "unchanged").unwrap();
                std::os::unix::fs::symlink(outside.join("lock"), root.join(".library-publish.pcbcraft-lock")).unwrap();
            }
        }
        assert!(publish(&root, &prepared).is_err());
        assert!(!outside.join(prepared.documents().keys().next().unwrap().split_once('/').unwrap().1).exists());
        if obstruction == "lock" {
            assert_eq!(std::fs::read(outside.join("lock")).unwrap(), b"unchanged");
        }
    }
}
