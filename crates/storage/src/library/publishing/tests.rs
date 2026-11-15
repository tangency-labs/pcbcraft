use super::*;
use pcbcraft_engine::authoring::{
    Action, Item, Kind as EditorKind, Session,
    publishing::{new_library, prepare},
};
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
        let first_bytes = first.documents().keys().map(|path| (path.clone(), std::fs::read(root.join(path)).unwrap())).collect::<Vec<_>>();
        let loaded = super::super::load(&root).unwrap();
        let record = &loaded.items()[0];
        let reopened = if kind == EditorKind::Symbol {
            Item::edit_symbol(&record.symbol().unwrap()).unwrap()
        } else {
            Item::edit_footprint(&record.footprint().unwrap()).unwrap()
        };
        assert_eq!(reopened.id, first.item().id);
        let mut session = Session::new(reopened, true).unwrap();
        session.bind_published();
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
