use super::*;
use pcbcraft_engine::{
    Session,
    persistence::{decode, decode_documents, decode_packed, encode_documents, encode_packed_documents},
};
use serde_json::json;

/// Use the original command-generated migration circuit, retaining its historical flat encoding.
fn legacy() -> Arc<Project> {
    decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap()
}

/// Every checkpoint of a board move recovers to one complete, validated layout.
#[test]
fn migration_recovers_at_every_publication_checkpoint() {
    let original = legacy();
    let before = encode_documents(original.clone()).unwrap();
    let after_project = prepare(original.clone(), None, Some(&before)).unwrap();
    let after = pcbcraft_engine::persistence::migrate_board_layout(before.clone(), ".").unwrap();
    let board_path = after_project.board_document_path().unwrap();
    assert_eq!(after[&board_path], before["board.json"]);
    assert_eq!(decode_documents(&after).unwrap().board_target(), original.board_target());
    let probe = tempfile::tempdir().unwrap();
    let root = probe.path().join("probe");
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let mut steps = Vec::new();
    crate::save(&root, Some(&before), &after, &mut |step| {
        steps.push(step.to_owned());
        Ok(())
    })
    .unwrap();
    assert!(steps.iter().any(|step| step == "published:board.json"));
    assert!(steps.iter().any(|step| step == &format!("published:{board_path}")));
    for (failed_step, step) in steps.iter().enumerate() {
        let trial = tempfile::tempdir().unwrap();
        let root = trial.path().join("project");
        crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
        let mut index = 0;
        assert!(
            crate::save(&root, Some(&before), &after, &mut |_| {
                let fail = index == failed_step;
                index += 1;
                if fail { Err("simulated interruption".into()) } else { Ok(()) }
            })
            .is_err()
        );
        let committed = root.join(".pcbcraft-save").exists() || step == "completed";
        let reopened = crate::load(&root).unwrap();
        assert_eq!(reopened, if committed { after.clone() } else { before.clone() }, "{}", step);
        let snapshot = decode_documents(&reopened).unwrap();
        assert_eq!(snapshot.board_target(), original.board_target());
        assert_eq!(snapshot.inspect_schematic(), original.inspect_schematic());
        crate::save(&root, Some(&reopened), &reopened, &mut |_| Err("unchanged save must not stage".into())).unwrap();
    }
}

/// The four reserved areas and case-insensitive occupied prefixes remain user-owned.
#[test]
fn foreign_areas_choose_and_retain_a_free_namespace() {
    for area in AREAS {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let original = legacy();
        let before = encode_documents(original.clone()).unwrap();
        crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
        fs::create_dir(root.join(area.to_uppercase())).unwrap();
        fs::write(root.join(area.to_uppercase()).join("notes.txt"), b"keep").unwrap();
        let id = original.id().simple().to_string();
        fs::create_dir(root.join(format!("PCBCRAFT-DATA-{}", id[..12].to_uppercase()))).unwrap();
        let prepared = prepare(original, Some(&root), Some(&before)).unwrap();
        assert_eq!(prepared.layout_root(), Some(format!("pcbcraft-data-{}", &id[..16]).as_str()));
        let after = pcbcraft_engine::persistence::migrate_board_layout(before.clone(), prepared.layout_root().unwrap()).unwrap();
        crate::save(&root, Some(&before), &after, &mut |_| Ok(())).unwrap();
        assert_eq!(fs::read(root.join(area.to_uppercase()).join("notes.txt")).unwrap(), b"keep");
        assert!(!root.join("board.json").exists());
        assert_eq!(crate::load(&root).unwrap(), after);
        assert!(Arc::ptr_eq(&prepared, &prepare(prepared.clone(), Some(&root), Some(&after)).unwrap()));
        let packed = decode_packed(&encode_packed_documents(after.clone()).unwrap()).unwrap();
        assert_eq!(packed.documents, after);
    }
}

/// A newly occupied destination detected after staging fails without publishing or claiming it.
#[test]
fn namespace_created_during_staging_is_never_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let before = encode_documents(legacy()).unwrap();
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let after = pcbcraft_engine::persistence::migrate_board_layout(before.clone(), ".").unwrap();
    assert!(
        crate::save(&root, Some(&before), &after, &mut |step| {
            if step == "staged" {
                fs::create_dir(root.join("boards")).unwrap();
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(crate::load(&root).unwrap(), before);
    assert!(!root.join(".pcbcraft-save").exists());
    assert!(root.join("boards").is_dir());
}

/// Only the declared board is loaded; unrelated boards are not scanned, adopted or packaged.
#[test]
fn reader_loads_only_the_registered_board_and_rejects_missing_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let documents = pcbcraft_engine::persistence::migrate_board_layout(encode_documents(legacy()).unwrap(), ".").unwrap();
    crate::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    fs::create_dir(root.join("boards/unregistered")).unwrap();
    fs::write(root.join("boards/unregistered/board.json"), b"user data").unwrap();
    assert_eq!(crate::load(&root).unwrap(), documents);
    let path = project_board_path(&documents[ENTRY_POINT]).unwrap().unwrap();
    fs::remove_file(root.join(path)).unwrap();
    assert!(crate::load(&root).is_err());
}

/// New schematic-only projects save no dummy board, and later creation gets a registered folder.
#[test]
fn new_zero_and_one_board_projects_use_folder_format() {
    for create_board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":create_board})).unwrap();
        let snapshot = session.snapshot().unwrap();
        let docs = encode_documents(snapshot.clone()).unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/14");
        assert_eq!(entry["layout_root"], ".");
        assert_eq!(entry["boards"].as_array().unwrap().len(), usize::from(create_board));
        assert!(!docs.contains_key("board.json"));
        assert_eq!(decode_documents(&docs).unwrap().board_target(), snapshot.board_target());
    }
}

/// A link in any registered board parent is refused before reading another directory's contents.
#[cfg(unix)]
#[test]
fn registered_board_parent_links_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let docs = pcbcraft_engine::persistence::migrate_board_layout(encode_documents(legacy()).unwrap(), ".").unwrap();
    crate::save(&root, None, &docs, &mut |_| Ok(())).unwrap();
    fs::rename(root.join("boards"), root.join("real-boards")).unwrap();
    std::os::unix::fs::symlink(root.join("real-boards"), root.join("boards")).unwrap();
    assert!(crate::load(&root).is_err());
}

/// A new board never claims a case-only matching user folder, even inside an owned area.
#[test]
fn new_board_uses_full_uuid_when_the_short_folder_is_occupied() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    session.apply("pcb.create_board", &json!({"name":"CON","expected_revision":session.revision()})).unwrap();
    let snapshot = session.snapshot().unwrap();
    let occupied = root.join("boards").join(snapshot.board_folder().unwrap().to_uppercase());
    fs::create_dir_all(&occupied).unwrap();
    fs::write(occupied.join("notes.txt"), b"user data").unwrap();
    let prepared = prepare(snapshot.clone(), Some(&root), Some(&before)).unwrap();
    assert_eq!(prepared.board_folder().unwrap(), format!("con-{}", prepared.board_target().unwrap().board));
    let full = root.join("boards").join(prepared.board_folder().unwrap());
    fs::create_dir(&full).unwrap();
    assert!(prepare(snapshot, Some(&root), Some(&before)).is_err());
    fs::remove_dir(full).unwrap();
    let after = encode_documents(prepared.clone()).unwrap();
    crate::save(&root, Some(&before), &after, &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read(occupied.join("notes.txt")).unwrap(), b"user data");
    assert_eq!(crate::load(&root).unwrap(), after);
    assert!(Arc::ptr_eq(&prepared, &prepare(prepared.clone(), Some(&root), Some(&after)).unwrap()));
}

/// Replacing a resolved board directory with an external symlink cannot redirect recovery's
/// write. The visible-tree mismatch retains the journal; restoring the parent recovers normally.
#[cfg(unix)]
#[test]
fn replacing_a_resolved_board_parent_cannot_overwrite_an_external_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("board.json"), b"must survive").unwrap();
    let project = Arc::new(legacy().with_board_folders(".").unwrap());
    let before = encode_documents(project.clone()).unwrap();
    crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let mut session = Session::default();
    session.open_snapshot(project);
    session.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = session.project().unwrap().board_document_path().unwrap();
    let parent = root.join(&path).parent().unwrap().to_path_buf();
    let retained = root.join("retained-board");
    let mut swapped = false;
    assert!(
        crate::save(&root, Some(&before), &after, &mut |step| {
            if step == format!("resolved:{path}") && !swapped {
                fs::rename(&parent, &retained).unwrap();
                std::os::unix::fs::symlink(&outside, &parent).unwrap();
                swapped = true;
            }
            Ok(())
        })
        .is_err()
    );
    assert!(swapped);
    assert_eq!(fs::read(outside.join("board.json")).unwrap(), b"must survive");
    assert_eq!(fs::read(retained.join("board.json")).unwrap(), after[&path]);
    assert!(root.join(".pcbcraft-save").exists());
    fs::remove_file(&parent).unwrap();
    fs::rename(retained, &parent).unwrap();
    assert_eq!(crate::load(&root).unwrap(), after);
    assert_eq!(fs::read(outside.join("board.json")).unwrap(), b"must survive");
}
