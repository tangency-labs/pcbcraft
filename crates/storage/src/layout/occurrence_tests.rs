use super::*;
use pcbcraft_engine::{
    Session,
    persistence::{decode, decode_documents, encode_documents, project_instances_path},
};
use serde_json::{Value, json};

/// Retain an original project/14 board-folder fixture, not a previous filename or flat layout.
fn project_fourteen() -> Arc<Project> {
    Arc::new(decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap().with_board_folders(".").unwrap())
}

/// Inject a process interruption at every journal checkpoint and reopen the complete old or
/// new document set, including when the instance file and entry point are temporarily mixed.
fn check_every_checkpoint(before: &Documents, after: &Documents) {
    let probe = tempfile::tempdir().unwrap();
    let root = probe.path().join("probe");
    crate::save(&root, None, before, &mut |_| Ok(())).unwrap();
    let mut steps = Vec::new();
    crate::save(&root, Some(before), after, &mut |step| {
        steps.push(step.to_owned());
        Ok(())
    })
    .unwrap();
    assert!(steps.iter().any(|step| step.ends_with("instances.json")));
    for (failed, step) in steps.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        crate::save(&root, None, before, &mut |_| Ok(())).unwrap();
        let mut index = 0;
        assert!(
            crate::save(&root, Some(before), after, &mut |_| {
                let stop = index == failed;
                index += 1;
                if stop { Err("interrupted".into()) } else { Ok(()) }
            })
            .is_err()
        );
        let committed = root.join(".pcbcraft-save").exists() || step == "completed";
        let loaded = crate::load(&root).unwrap();
        assert_eq!(&loaded, if committed { after } else { before }, "{step}");
        decode_documents(&loaded).unwrap();
        crate::save(&root, Some(&loaded), &loaded, &mut |_| Err("unchanged save must not stage".into())).unwrap();
    }
}

/// Current board folders gain one instance file atomically, keeping board paths and payloads.
#[test]
fn project_fourteen_occurrence_migration_recovers_at_every_checkpoint() {
    let source = project_fourteen();
    let before = encode_documents(source.clone()).unwrap();
    let after_project = prepare(source.clone(), None, Some(&before)).unwrap();
    assert_eq!(source.board_document_path(), after_project.board_document_path());
    let after = encode_documents(after_project).unwrap();
    assert_eq!(after.len(), before.len() + 1);
    assert!(project_instances_path(&before[ENTRY_POINT]).unwrap().is_none());
    assert!(project_instances_path(&after[ENTRY_POINT]).unwrap().is_some());
    check_every_checkpoint(&before, &after);
}

/// First-board creation and its undo transfer the same records between their two storage homes.
#[test]
fn first_board_instance_transfer_and_undo_recover_at_every_checkpoint() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let records = s.project().unwrap().sheet_instances();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply("pcb.create_board", &json!({"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.project().unwrap().sheet_instances(), records);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    check_every_checkpoint(&before, &after);
    check_every_checkpoint(&after, &before);
}

/// A foreign instance filename is not owned merely because its enclosing board folder is.
#[test]
fn occurrence_migration_never_overwrites_unregistered_instance_files() {
    for leaf in ["instances.json", "INSTANCES.JSON"] {
        let snapshot = project_fourteen();
        let before = encode_documents(snapshot.clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
        let board = Path::new(&snapshot.board_document_path().unwrap()).parent().unwrap().to_owned();
        let foreign = root.join(&board).join(leaf);
        fs::write(&foreign, b"user-owned instance notes").unwrap();
        let after = encode_documents(prepare(snapshot, Some(&root), Some(&before)).unwrap()).unwrap();
        let error = crate::save(&root, Some(&before), &after, &mut |_| Ok(())).unwrap_err();
        assert!(error.contains("unregistered"), "{error}");
        assert_eq!(fs::read(&foreign).unwrap(), b"user-owned instance notes");
        assert_eq!(crate::load(&root).unwrap(), before);
        assert!(!root.join(".pcbcraft-save").exists());
    }
}

/// An external writer racing staging must still be refused before journal publication, even
/// though project/14's reader correctly ignores the newly appearing unregistered instance file.
#[test]
fn occurrence_migration_rechecks_foreign_instance_files_after_staging() {
    let snapshot = project_fourteen();
    let before = encode_documents(snapshot.clone()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let prepared = prepare(snapshot, Some(&root), Some(&before)).unwrap();
    let foreign = root.join(prepared.instances_document_path().unwrap());
    let after = encode_documents(prepared).unwrap();
    let result = crate::save(&root, Some(&before), &after, &mut |step| {
        if step == "staged" {
            fs::write(&foreign, b"late user file").unwrap();
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("unregistered"));
    assert_eq!(fs::read(&foreign).unwrap(), b"late user file");
    assert_eq!(crate::load(&root).unwrap(), before);
    assert!(!root.join(".pcbcraft-save").exists());
}

/// Migration uses the previously registered custom root and preserves unrelated board files.
#[test]
fn occurrence_migration_keeps_registered_namespaces_and_user_notes() {
    let snapshot = Arc::new(project_fourteen().with_layout_root("pcbcraft-data-test").unwrap());
    let before = encode_documents(snapshot.clone()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let board = snapshot.board_document_path().unwrap();
    let notes = root.join(Path::new(&board).parent().unwrap()).join("notes.txt");
    fs::write(&notes, b"keep my notes").unwrap();
    let prepared = prepare(snapshot, Some(&root), Some(&before)).unwrap();
    assert_eq!(prepared.layout_root(), Some("pcbcraft-data-test"));
    assert!(prepared.instances_document_path().unwrap().starts_with("pcbcraft-data-test/boards/"));
    let after = encode_documents(prepared).unwrap();
    crate::save(&root, Some(&before), &after, &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read(notes).unwrap(), b"keep my notes");
    assert_eq!(after[&board], before[&board]);
    assert_eq!(crate::load(&root).unwrap(), after);
    assert!(!root.join("board.json").exists());
    assert_eq!(serde_json::from_slice::<Value>(&after[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/16");
}
