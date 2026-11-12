//! Filesystem and worker acceptance tests for folder saves and v3 archive saves.
use super::*;
use crate::project_files::{Job, ProjectFiles, Update};
use pcbcraft_engine::persistence::encode_packed;
use pcbcraft_engine::{
    DELETE_SYMBOL, MOVE_SYMBOL, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    persistence::{decode, encode_documents},
};
use serde_json::json;
use std::{
    fs::OpenOptions,
    sync::{Barrier, mpsc},
};

/// Start from the checked-in migration fixture so comparisons retain stable UUIDs.
fn session() -> Session {
    let mut s = Session::default();
    s.open_snapshot(Arc::new(
        decode(include_bytes!("../../../tests/fixtures/two-resistors.pc"))
            .unwrap()
            .with_board_folders(".")
            .unwrap()
            .with_sheet_occurrences()
            .unwrap(),
    ));
    s
}

/// A previous split folder opens through desktop/preview and upgrades on an unchanged Save.
/// The saved layout is adopted without changing revision or payload bytes.
#[test]
fn previous_split_folder_opens_previews_and_saves_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    let original = decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap();
    let documents = encode_documents(original.clone()).unwrap();
    fs::create_dir(&root).unwrap();
    for (path, bytes) in &documents {
        let path = root.join(if path == ENTRY_POINT { "project.pc" } else { path });
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let entry = root.join("project.pc");
    let opened = load(&entry).unwrap();
    assert!(crate::schematic_canvas::Thumbnail::new(&pcbcraft_storage::preview(&entry).unwrap()).is_some());
    assert!(!root.join(ENTRY_POINT).exists());
    let mut s = Session::default();
    let mut files = ProjectFiles::default();
    let previous = (opened.snapshot.clone(), opened.target.clone());
    complete(&mut files, &mut s, Ok(Outcome::Opened(opened)));
    let revision = s.revision();
    assert!(!files.dirty(&s), "opening a legacy project does not invent electrical edits");
    let result = save(s.snapshot().unwrap(), Some(previous), None);
    assert!(matches!(complete(&mut files, &mut s, result), Some(Update::Message(_))));
    assert_eq!(s.revision(), revision);
    assert!(!files.dirty(&s));
    assert!(!entry.exists());
    assert_eq!(load(&entry).unwrap().snapshot.id(), original.id());
    let board = s.project().unwrap().board_document_path().unwrap();
    assert_eq!(fs::read(root.join(board)).unwrap(), documents["board.json"]);
    for (path, bytes) in &documents {
        if path != ENTRY_POINT && path != "board.json" {
            assert_eq!(fs::read(root.join(path)).unwrap(), *bytes);
        }
    }
}

/// Apply ordinary engine mutations with the current revision.
fn apply(s: &mut Session, command: &str, mut params: serde_json::Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Change component ownership, geometry, board data and library-copy lifetime in one transaction.
fn replacement(s: &mut Session) {
    apply(s, DELETE_SYMBOL, json!({"symbol":"symbol-2"}));
    apply(s, DELETE_SYMBOL, json!({"symbol":"symbol-3"}));
    apply(s, PLACE_SYMBOL, json!({"definition":"led","x_nm":0,"y_nm":0}));
    let id = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    apply(s, PLACE_FOOTPRINT, json!({"symbol":id,"definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0}));
}

/// Deliver worker results through the same editor-side revision and dirty-state logic.
fn complete(files: &mut ProjectFiles, session: &mut Session, result: Result<Outcome, String>) -> Option<Update> {
    let (send, receiver) = mpsc::channel();
    files.job = Some(Job { receiver, revision: session.revision() });
    send.send(result).unwrap();
    files.poll(session, &egui::Context::default())
}

/// Desktop, shared CLI/preview and archive readers recognize schematic package intent.
/// Folder saves can create and undo a board without losing pinned assets or user-owned files.
#[test]
fn schematic_package_choices_open_and_save_through_native_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    apply(&mut s, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let component = s.project().unwrap().components().next().unwrap().id();
    apply(
        &mut s,
        pcbcraft_engine::ASSIGN_FOOTPRINT,
        json!({"component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"}}),
    );
    let snapshot = s.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    let root = dir.path().join("schematic-packages");
    let archive = dir.path().join("schematic-packages.pc");
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    fs::write(root.join("README.md"), b"User notes").unwrap();
    fs::write(&archive, encode_packed(snapshot).unwrap()).unwrap();
    for path in [&root, &root.join(ENTRY_POINT), &archive] {
        assert_eq!(encode_documents(folder::open(path).unwrap()).unwrap(), documents);
        assert_eq!(encode_documents(folder::preview(path).unwrap()).unwrap(), documents);
        let loaded = load(path).unwrap();
        assert!(loaded.snapshot.board_target().is_none());
        assert_eq!(loaded.snapshot.footprint_assignments()[0]["mapping"], json!({"1":"2","2":"1"}));
        assert_eq!(encode_documents(loaded.snapshot.clone()).unwrap(), documents);
        save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
        assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
    }
    assert!(!root.join("board.json").exists());
    apply(&mut s, "pcb.create_board", json!({}));
    let created = encode_documents(s.snapshot().unwrap()).unwrap();
    folder::save(&root, Some(&documents), &created, &mut |_| Ok(())).unwrap();
    assert!(load(&root.join(ENTRY_POINT)).unwrap().snapshot.board_target().is_some());
    s.apply("edit.undo", &json!({})).unwrap();
    let undone = encode_documents(s.snapshot().unwrap()).unwrap();
    folder::save(&root, Some(&created), &undone, &mut |_| Ok(())).unwrap();
    assert!(!root.join("board.json").exists());
    assert_eq!(encode_documents(load(&root.join(ENTRY_POINT)).unwrap().snapshot).unwrap(), documents);
    assert_eq!(fs::read(root.join("README.md")).unwrap(), b"User notes");
}

/// Both desktop and CLI readers recognize legacy metadata format 11 and explicit-owner
/// format 12 through a folder, its entry
/// point or a packed archive, preserving the allocation watermark and unchanged saved bytes.
#[test]
fn component_metadata_opens_and_saves_through_every_native_entry_path() {
    for explicit in [None, Some(false), Some(true)] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = match explicit {
            None => session(),
            Some(create_board) => {
                let mut session = Session::default();
                session.apply("project.new", &json!({"create_board":create_board})).unwrap();
                apply(&mut session, pcbcraft_engine::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
                session
            }
        };
        let mut metadata = pcbcraft_engine::schematic::fields::Metadata::default();
        metadata.attributes.do_not_populate = true;
        apply(&mut s, pcbcraft_engine::SET_SYMBOL_FIELDS, json!({"symbol":"symbol-2","reference":"R1","value":"1k","metadata":metadata}));
        let snapshot = s.snapshot().unwrap();
        let documents = encode_documents(snapshot.clone()).unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&documents[ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/16");
        let root = dir.path().join("metadata");
        let archive = dir.path().join("metadata.pc");
        folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
        fs::write(&archive, encode_packed(snapshot).unwrap()).unwrap();
        for path in [&root, &root.join(ENTRY_POINT), &archive] {
            assert_eq!(encode_documents(folder::open(path).unwrap()).unwrap(), documents);
            assert_eq!(encode_documents(folder::preview(path).unwrap()).unwrap(), documents);
            let loaded = load(path).unwrap();
            assert_eq!(encode_documents(loaded.snapshot.clone()).unwrap(), documents);
            let before = if path == &archive { Some(fs::read(path).unwrap()) } else { None };
            save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
            assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
            if let Some(before) = before {
                assert_eq!(fs::read(path).unwrap(), before);
            }
        }
    }
}

/// Initial Home stays visible through pending, cancelled and failed opens; a completed open
/// enters the loaded schematic through the same file-result polling used by the real window.
#[test]
fn startup_home_enters_the_workspace_only_after_a_successful_open() {
    use egui_kittest::{Harness, kittest::Queryable};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let documents = encode_documents(session().snapshot().unwrap()).unwrap();
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| crate::PcbApp::new(cc));
    for result in [Ok(Outcome::Cancelled), Err("Could not open fixture".to_owned()), load(&root).map(Outcome::Opened)] {
        let (send, receiver) = mpsc::channel();
        let app = harness.state_mut();
        app.files.job = Some(Job { receiver, revision: app.session.revision() });
        // A pending file job deliberately keeps repainting while waiting for its worker.
        harness.step();
        harness.get_by_label("Open Project");
        assert!(harness.state().session().project().is_none());
        let opened = matches!(&result, Ok(Outcome::Opened(_)));
        send.send(result).unwrap();
        harness.run();
        assert_eq!(harness.state().home, !opened);
        assert_eq!(harness.state().session().project().is_some(), opened);
    }
    harness.get_by_label("Toggle Net Inspector");
    assert!(harness.query_by_label("Close Parts Catalog").is_none());
    harness.key_press(egui::Key::A);
    harness.run();
    harness.get_by_label("Close Parts Catalog");
    harness.get_by_label("Resistor");
    assert_eq!(encode_documents(harness.state().session().snapshot().unwrap()).unwrap(), documents);
}

/// Interrupt after each durable step, including deletions and cleanup, then recover before open.
#[test]
fn every_interrupted_folder_save_reopens_as_old_or_new() {
    let mut s = session();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    replacement(&mut s);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    let probe = tempfile::tempdir().unwrap();
    let root = probe.path().join("project");
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let mut checkpoints = Vec::new();
    folder::save(&root, Some(&before), &after, &mut |phase| {
        checkpoints.push(phase.to_owned());
        Ok(())
    })
    .unwrap();
    for (stop, phase) in checkpoints.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
        fs::write(root.join("README.md"), b"keep me").unwrap();
        let metadata = ["sheets/.DS_Store", "library/Thumbs.db", "library/symbols/starter/desktop.ini"];
        for path in metadata {
            fs::write(root.join(path), b"OS metadata").unwrap();
        }
        let mut index = 0;
        assert!(
            folder::save(&root, Some(&before), &after, &mut |_| {
                let fail = index == stop;
                index += 1;
                if fail { Err("interrupted".to_owned()) } else { Ok(()) }
            })
            .is_err()
        );
        let reopened = folder::load(&root).unwrap_or_else(|e| panic!("after {phase}: {e}"));
        let expected = if phase == "payload" || phase == "staged" { &before } else { &after };
        assert_eq!(&reopened, expected, "after {phase}");
        assert!(!root.join(".pcbcraft-save").exists());
        assert_eq!(fs::read(root.join("README.md")).unwrap(), b"keep me");
        for path in metadata {
            assert_eq!(fs::read(root.join(path)).unwrap(), b"OS metadata");
        }
    }
}

/// Recovery also works when the first save crashed before project.json itself was installed.
#[test]
fn recover_first_save_through_the_entry_point() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("new-project");
    let d = encode_documents(session().snapshot().unwrap()).unwrap();
    assert!(folder::save(&root, None, &d, &mut |phase| if phase == "journal" { Err("crash".to_owned()) } else { Ok(()) }).is_err());
    assert!(!root.join(ENTRY_POINT).exists());
    let loaded = load(&root.join(ENTRY_POINT)).unwrap();
    assert_eq!(loaded.target.documents.as_deref(), Some(&d));
}

/// No-op saves preserve exact bytes and metadata, while another document's edit preserves
/// noncanonical but valid formatting in the entry point and saved library copy.
#[test]
fn unchanged_documents_are_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut d = encode_documents(session().snapshot().unwrap()).unwrap();
    d.get_mut(ENTRY_POINT).unwrap().extend_from_slice(b"\n  ");
    folder::save(&root, None, &d, &mut |_| Ok(())).unwrap();
    fs::write(root.join("sheets/.DS_Store"), b"OS metadata").unwrap();
    let loaded = load(&root).unwrap();
    let old = loaded.snapshot.clone();
    let target = loaded.target;
    let modified = fs::metadata(root.join(ENTRY_POINT)).unwrap().modified().unwrap();
    save_target(old.clone(), Some(&old), target.clone(), &mut |_| panic!("no-op must not stage")).unwrap();
    assert_eq!(fs::metadata(root.join(ENTRY_POINT)).unwrap().modified().unwrap(), modified);
    let mut s = Session::default();
    s.open_snapshot(old.clone());
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}));
    save_target(s.snapshot().unwrap(), Some(&old), target, &mut |_| Ok(())).unwrap();
    let after = folder::load(&root).unwrap();
    assert_eq!(after[ENTRY_POINT], d[ENTRY_POINT]);
    assert_eq!(fs::metadata(root.join(ENTRY_POINT)).unwrap().modified().unwrap(), modified);
    let changed: Vec<_> = d.keys().filter(|path| d[*path] != after[*path]).collect();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].starts_with("sheets/"));
    assert_eq!(fs::read(root.join("sheets/.DS_Store")).unwrap(), b"OS metadata");
}

/// Opening flat version-5 input preserves bytes; its first save migrates even without edits.
/// Later geometry and sheet-lifecycle changes retain all unaffected payload bytes.
#[test]
fn version_five_first_save_migrates_and_sheet_edits_preserve_unaffected_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let original = decode_packed(include_bytes!("../../../tests/fixtures/split/two-resistors.pc")).unwrap().documents;
    folder::save(&root, None, &original, &mut |_| Ok(())).unwrap();
    let loaded = load(&root.join(ENTRY_POINT)).unwrap();
    assert_eq!(folder::load(&root).unwrap(), original);
    save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
    let migrated = pcbcraft_engine::persistence::migrate_board_layout(original.clone(), ".").unwrap();
    assert_eq!(folder::load(&root).unwrap(), migrated);
    let loaded = load(&root).unwrap();
    let old = loaded.snapshot.clone();
    save_target(old.clone(), Some(&old), loaded.target.clone(), &mut |_| panic!("second save must not stage")).unwrap();
    let mut s = Session::default();
    s.open_snapshot(old.clone());
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1_270_000,"y_nm":0}));
    save_target(s.snapshot().unwrap(), Some(&old), loaded.target, &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read(root.join(ENTRY_POINT)).unwrap(), migrated[ENTRY_POINT]);
    let loaded = load(&root).unwrap();
    apply(&mut s, pcbcraft_engine::sheets::CREATE_SHEET, json!({"name":"Power"}));
    save_target(s.snapshot().unwrap(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
    let after = folder::load(&root).unwrap();
    let header: serde_json::Value = serde_json::from_slice(&after[ENTRY_POINT]).unwrap();
    assert_eq!(header["format"], "pcbcraft-project/16");
    assert_eq!(pcbcraft_storage::open(&root.join(ENTRY_POINT)).unwrap().sheets().len(), 2);
    for (path, bytes) in
        migrated.iter().filter(|(path, _)| path.as_str() != ENTRY_POINT && !path.starts_with("sheets/") && !path.ends_with("/instances.json"))
    {
        assert_eq!(&after[path], bytes);
    }
}

/// Interrupted sheet deletion recovers a complete project, including its component records,
/// saved definitions and document removal. A pre-journal interruption retains the old project.
#[test]
fn interrupted_sheet_deletion_recovers_without_partial_document_sets() {
    let mut s = session();
    apply(&mut s, pcbcraft_engine::sheets::CREATE_SHEET, json!({"name":"Power"}));
    let power = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, PLACE_SYMBOL, json!({"sheet":power.id(),"definition":"led","x_nm":0,"y_nm":0}));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    apply(&mut s, pcbcraft_engine::sheets::DELETE_SHEET, json!({"sheet":power.id()}));
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    for phase in [
        "staged".to_owned(),
        "journal".to_owned(),
        format!("published:{ENTRY_POINT}"),
        format!("published:sheets/{}.json", power.id()),
        "completed".to_owned(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
        assert!(folder::save(&root, Some(&before), &after, &mut |at| if at == phase { Err("interrupted".into()) } else { Ok(()) }).is_err());
        assert_eq!(folder::load(&root).unwrap(), if phase == "staged" { before.clone() } else { after.clone() });
    }
}

/// External changes and incomplete documents cannot replace the active project or clear edits.
#[test]
fn failed_folder_opens_and_stale_saves_preserve_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let d = encode_documents(session().snapshot().unwrap()).unwrap();
    folder::save(&root, None, &d, &mut |_| Ok(())).unwrap();
    assert_eq!(
        folder::save(&root, None, &d, &mut |_| panic!("Save As must not overwrite a project")).unwrap_err(),
        "That folder already contains a project. Choose a new or empty folder."
    );
    assert_eq!(folder::load(&root).unwrap(), d);
    let mut s = Session::default();
    let mut files = ProjectFiles::default();
    complete(&mut files, &mut s, load(&root).map(Outcome::Opened));
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}));
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let board = pcbcraft_engine::persistence::project_board_path(&d[ENTRY_POINT]).unwrap().unwrap();
    fs::write(root.join(&board), b"broken").unwrap();
    let result = load(&root).map(Outcome::Opened);
    assert!(result.as_ref().err().unwrap().contains(&board));
    assert!(matches!(complete(&mut files, &mut s, result), Some(Update::Error(_))));
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    assert!(files.dirty(&s));
    let saved = files.saved.as_ref().unwrap();
    assert_eq!(
        save_target(snapshot, Some(&saved.snapshot), saved.target.clone(), &mut |_| Ok(())).err().unwrap(),
        "Project folder changed on disk. Use Save As to keep your edits in a separate folder."
    );
    assert_eq!(fs::read(root.join(&board)).unwrap(), b"broken");
}

/// Overlapping writers use a nonblocking lock and only one publishes from the same baseline.
#[test]
fn concurrent_folder_writers_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut a = session();
    let before = encode_documents(a.snapshot().unwrap()).unwrap();
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let mut b = session();
    apply(&mut a, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}));
    apply(&mut b, MOVE_SYMBOL, json!({"symbol":"symbol-3","x_nm":1270000,"y_nm":0}));
    let a = encode_documents(a.snapshot().unwrap()).unwrap();
    let b = encode_documents(b.snapshot().unwrap()).unwrap();
    let barrier = Barrier::new(2);
    let run = |d: &Documents| {
        barrier.wait();
        folder::save(&root, Some(&before), d, &mut |_| Ok(()))
    };
    let (left, right) = std::thread::scope(|scope| {
        let first = scope.spawn(|| run(&a));
        let second = scope.spawn(|| run(&b));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(folder::load(&root).unwrap(), if left.is_ok() { a } else { b });
}

/// A completed save adopts its own snapshot, leaving edits made while the worker ran dirty.
#[test]
fn folder_save_does_not_mark_later_edits_saved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let d = encode_documents(session().snapshot().unwrap()).unwrap();
    folder::save(&root, None, &d, &mut |_| Ok(())).unwrap();
    let mut s = Session::default();
    let mut files = ProjectFiles::default();
    complete(&mut files, &mut s, load(&root).map(Outcome::Opened));
    let old = files.saved.as_ref().unwrap().snapshot.clone();
    let target = files.saved.as_ref().unwrap().target.clone();
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}));
    let staged = s.snapshot().unwrap();
    let (send, receiver) = mpsc::channel();
    files.job = Some(Job { receiver, revision: s.revision() });
    std::thread::scope(|scope| {
        let (ready, wait) = mpsc::sync_channel(0);
        let (release, resume) = mpsc::sync_channel(0);
        scope.spawn(move || {
            let outcome = save_target(staged, Some(&old), target, &mut |phase| {
                if phase == "staged" {
                    ready.send(()).unwrap();
                    resume.recv().unwrap();
                }
                Ok(())
            });
            send.send(outcome).unwrap();
        });
        wait.recv().unwrap();
        apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-3","x_nm":1270000,"y_nm":0}));
        release.send(()).unwrap();
    });
    assert!(matches!(files.poll(&mut s, &egui::Context::default()), Some(Update::Message(_))));
    assert!(files.dirty(&s));
    let disk = load(&root).unwrap().snapshot;
    assert_eq!(disk.symbols()[1].position.x_nm, 25400000);
    assert_eq!(s.project().unwrap().symbols()[1].position.x_nm, 1270000);
}

/// Corrupt journals and external edits during an interrupted transaction stop recovery without
/// applying its remaining replacements. Fixing the obstruction makes the journal recoverable.
#[test]
fn corrupt_recovery_payload_and_external_changes_are_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut s = session();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let board_path = s.project().unwrap().board_document_path().unwrap();
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    replacement(&mut s);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(folder::save(&root, Some(&before), &after, &mut |phase| if phase == "journal" { Err("crash".to_owned()) } else { Ok(()) }).is_err());
    let path = root.join(".pcbcraft-save/0.new");
    let payload = fs::read(&path).unwrap();
    fs::write(&path, b"bad").unwrap();
    assert!(folder::load(&root).is_err());
    assert_eq!(fs::read(root.join(ENTRY_POINT)).unwrap(), before[ENTRY_POINT]);
    fs::write(&path, payload).unwrap();
    fs::write(root.join(&board_path), b"external").unwrap();
    assert!(folder::load(&root).is_err());
    assert_eq!(fs::read(root.join(&board_path)).unwrap(), b"external");
    fs::write(root.join(&board_path), &before[&board_path]).unwrap();
    assert_eq!(folder::load(&root).unwrap(), after);
}

/// Normal folder opens leave unrelated root files alone and refuse unknown owned files and links.
#[test]
fn unknown_owned_files_and_symlinks_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let d = encode_documents(session().snapshot().unwrap()).unwrap();
    #[cfg(unix)]
    let board_path = pcbcraft_engine::persistence::project_board_path(&d[ENTRY_POINT]).unwrap().unwrap();
    folder::save(&root, None, &d, &mut |_| Ok(())).unwrap();
    fs::write(root.join("README.md"), b"notes").unwrap();
    assert!(load(&root).is_ok());
    fs::write(root.join("sheets/notes.txt"), b"notes").unwrap();
    assert!(load(&root).is_err());
    fs::remove_file(root.join("sheets/notes.txt")).unwrap();
    #[cfg(unix)]
    {
        let hidden = root.join("sheets/.DS_Store");
        std::os::unix::fs::symlink(root.join(ENTRY_POINT), &hidden).unwrap();
        assert!(load(&root).is_err());
        assert!(folder::save(&root, Some(&d), &d, &mut |_| Ok(())).is_err());
        fs::remove_file(hidden).unwrap();
        fs::remove_file(root.join(&board_path)).unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, &d[&board_path]).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(&board_path)).unwrap();
        assert!(load(&root).is_err());
        assert!(folder::save(&root, Some(&d), &d, &mut |_| Ok(())).is_err());
        assert_eq!(fs::read(outside).unwrap(), d[&board_path]);
    }
}

/// Restore directory permissions on assertion failures too, so temporary projects can be removed.
#[cfg(unix)]
struct ReadOnlyDirectory(PathBuf, fs::Permissions);

#[cfg(unix)]
impl ReadOnlyDirectory {
    /// Deny lock-file creation while allowing traversal and reads of the project beneath it.
    fn new(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::metadata(path).unwrap().permissions();
        fs::set_permissions(path, fs::Permissions::from_mode(0o555)).unwrap();
        Self(path.to_owned(), permissions)
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyDirectory {
    /// Restore the original mode before the enclosing temporary directory is cleaned up.
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

/// Probe the actual filesystem rather than assuming mode bits constrain this process. Privileged
/// containers may still create files; restore permissions and remove the probe in either case.
#[cfg(unix)]
fn directory_permissions_enforced(path: &Path) -> bool {
    let read_only = ReadOnlyDirectory::new(path);
    let probe = tempfile::Builder::new().prefix(".permission-probe-").tempfile_in(path);
    drop(read_only);
    match probe {
        Ok(file) => {
            file.close().unwrap();
            false
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => true,
        Err(error) => panic!("Could not probe directory permissions: {error}"),
    }
}

/// Desktop and headless readers accept read-only locations, while saves and pending recovery
/// still require a writable lock. Skip when directory modes are not enforced; otherwise a blocked
/// recovery must leave every published byte unchanged.
#[cfg(unix)]
#[test]
fn read_only_folders_open_without_creating_locks_but_cannot_recover_or_save() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    if !directory_permissions_enforced(dir.path()) {
        eprintln!("Skipping read-only assertions: this process can write through directory mode 0o555.");
        return;
    }
    let root = dir.path().join("project");
    let lock = dir.path().join(".project.pcbcraft-lock");
    let mut s = session();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    fs::remove_file(&lock).unwrap();
    let read_only = ReadOnlyDirectory::new(dir.path());
    assert_eq!(load(&root).unwrap().target.documents.as_deref(), Some(&before));
    assert_eq!(encode_documents(pcbcraft_storage::open(&root.join(ENTRY_POINT)).unwrap()).unwrap(), before);
    assert!(!lock.exists());
    assert!(folder::save(&root, Some(&before), &before, &mut |_| panic!("read-only save must not stage")).is_err());
    drop(read_only);

    // An existing lock remains authoritative even if it can only be opened for reading.
    let held = OpenOptions::new().read(true).write(true).create_new(true).open(&lock).unwrap();
    held.try_lock().unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o444)).unwrap();
    let read_only = ReadOnlyDirectory::new(dir.path());
    assert!(folder::load(&root).unwrap_err().contains("Could not lock"));
    held.unlock().unwrap();
    assert_eq!(folder::load(&root).unwrap(), before);
    assert!(folder::save(&root, Some(&before), &before, &mut |_| panic!("read-only save must not stage")).is_err());
    drop(read_only);
    drop(held);
    fs::remove_file(&lock).unwrap();

    replacement(&mut s);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(folder::save(&root, Some(&before), &after, &mut |phase| if phase == "journal" { Err("crash".to_owned()) } else { Ok(()) }).is_err());
    fs::remove_file(&lock).unwrap();
    let read_only = ReadOnlyDirectory::new(dir.path());
    assert!(folder::load(&root).unwrap_err().contains("needs recovery"));
    assert!(pcbcraft_storage::open(&root.join(ENTRY_POINT)).unwrap_err().contains("needs recovery"));
    for (path, bytes) in &before {
        assert_eq!(&fs::read(root.join(path)).unwrap(), bytes);
    }
    assert!(root.join(".pcbcraft-save/journal.json").is_file());
    assert!(!lock.exists());
    drop(read_only);
    assert_eq!(folder::load(&root).unwrap(), after);
}

/// Contention and invalid lock paths must not be mistaken for a read-only project location.
#[test]
fn folder_load_never_bypasses_a_busy_or_invalid_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = dir.path().join(".project.pcbcraft-lock");
    let documents = encode_documents(session().snapshot().unwrap()).unwrap();
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    let lock = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    lock.try_lock().unwrap();
    assert!(folder::load(&root).unwrap_err().contains("Could not lock"));
    lock.unlock().unwrap();
    drop(lock);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(folder::load(&root).unwrap_err().contains("not a regular file"));
}

/// No-op archive saves retain exact imported bytes, and folder/archive conversion retains IDs.
#[test]
fn v3_archive_saves_and_folder_conversion_preserve_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let s = session();
    let mut bytes = encode_packed(s.snapshot().unwrap()).unwrap();
    let comment = b"custom comment";
    let length = bytes.len();
    bytes[length - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    let path = dir.path().join("project.pc");
    fs::write(&path, &bytes).unwrap();
    let loaded = load(&path).unwrap();
    save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| panic!("unchanged archive must not stage")).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let d = encode_documents(loaded.snapshot).unwrap();
    folder::save(&root, None, &d, &mut |_| Ok(())).unwrap();
    let reopened = load(&root.join(ENTRY_POINT)).unwrap();
    assert_eq!(encode_documents(decode_packed(&bytes).unwrap().project).unwrap(), encode_documents(reopened.snapshot).unwrap());
}

/// Independent sheet edits merge using Git's ordinary text merge and survive cross-file validation.
#[test]
fn independent_edits_merge_in_a_split_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = session();
    let before = encode_documents(a.snapshot().unwrap()).unwrap();
    let path = before.keys().find(|p| p.starts_with("sheets/")).unwrap();
    let mut b = session();
    apply(&mut a, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0}));
    apply(&mut b, MOVE_SYMBOL, json!({"symbol":"symbol-3","x_nm":1270000,"y_nm":0}));
    for (name, bytes) in [
        ("base", &before[path]),
        ("a", &encode_documents(a.snapshot().unwrap()).unwrap()[path]),
        ("b", &encode_documents(b.snapshot().unwrap()).unwrap()[path]),
    ] {
        fs::write(dir.path().join(name), bytes).unwrap();
    }
    let merged = std::process::Command::new("git")
        .args(["merge-file", "-p"])
        .arg(dir.path().join("a"))
        .arg(dir.path().join("base"))
        .arg(dir.path().join("b"))
        .output()
        .unwrap();
    assert!(merged.status.success());
    let mut d = before.clone();
    d.insert(path.clone(), merged.stdout);
    let project = decode_documents(&d).unwrap();
    assert!(project.symbols().iter().all(|p| p.position.x_nm == 1270000));
}

/// A manifest-less archive gains a manifest on its first save, and the editor reports that change.
#[test]
fn manifest_less_archive_save_adds_manifest_and_preserves_document_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hand-zipped.pc");
    fs::write(&path, include_bytes!("../../../tests/fixtures/split/two-resistors-no-manifest.pc")).unwrap();
    let loaded = load(&path).unwrap();
    assert!(loaded.target.needs_manifest);
    assert!(loaded.target.open_notice().contains("upgrade the project package"));
    let documents = loaded.target.documents.clone().unwrap();
    let outcome = save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
    let Outcome::Saved(saved, _) = outcome else { panic!("expected saved project") };
    assert!(saved.target.added_manifest());
    let reopened = load(&path).unwrap();
    assert!(!reopened.target.needs_manifest);
    let migrated = pcbcraft_engine::persistence::migrate_board_layout((*documents).clone(), ".").unwrap();
    assert_eq!(reopened.target.documents.as_deref(), Some(&migrated));
    let mut custom = (*documents).clone();
    custom.get_mut(ENTRY_POINT).unwrap().extend_from_slice(b"\n  ");
    let root = dir.path().join("custom");
    folder::save(&root, None, &custom, &mut |_| Ok(())).unwrap();
    let loaded = load(&root).unwrap();
    let converted = working_documents(&loaded.snapshot, Some(&loaded.snapshot), loaded.target.documents.as_deref()).unwrap();
    let packed = encode_packed_documents(converted).unwrap();
    assert_eq!(decode_packed(&packed).unwrap().documents, custom);
}

/// The shared headless reader accepts the same folder, entry point and archive as desktop.
#[test]
fn headless_and_desktop_read_the_same_split_project() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/split");
    let expected = encode_documents(session().snapshot().unwrap()).unwrap();
    for path in [
        fixture.join("two-resistors"),
        fixture.join("two-resistors/project.json"),
        fixture.join("two-resistors.pc"),
        fixture.join("two-resistors-no-manifest.pc"),
    ] {
        assert_eq!(
            pcbcraft_engine::persistence::migrate_board_layout(encode_documents(pcbcraft_storage::open(&path).unwrap()).unwrap(), ".").unwrap(),
            expected
        );
    }
}

/// Material-aware folders and archives use the real native open/save paths without external
/// libraries, and a failed save cannot clear dirty state or replace the saved baseline.
#[test]
fn material_setup_survives_native_folder_and_archive_saves() {
    use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP};
    let dir = tempfile::tempdir().unwrap();
    let mut s = session();
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    let mut setup = s.inspect()["project"]["board"]["setup"].clone();
    let material = json!({"id":"00000000-0000-0000-0000-000000000101","revision":1});
    setup["materials"] = json!([{"id":material["id"],"revision":1,"name":"User dielectric","family":"Custom composite",
        "dielectric":[{"dk_millionths":4200000,"frequency_hz":1000000000,"basis":"typical","source":{"document":"Synthetic source"}}]
    }]);
    setup["stackup"]["strata"][1]["construction"] = json!({"kind":"core","material":material});
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":setup}));
    let snapshot = s.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    let root = dir.path().join("material-board");
    let archive = dir.path().join("material-board.pc");
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    fs::write(&archive, encode_packed(snapshot.clone()).unwrap()).unwrap();
    for path in [&root, &archive] {
        let loaded = load(path).unwrap();
        assert_eq!(encode_documents(loaded.snapshot.clone()).unwrap(), documents);
        let outcome = save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target.clone(), &mut |_| Ok(())).unwrap();
        let mut files = ProjectFiles::default();
        s.open_snapshot(loaded.snapshot.clone());
        complete(&mut files, &mut s, Ok(outcome));
        assert!(!files.dirty(&s));
        assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
        let mut edited = setup.clone();
        edited["stackup"]["strata"][1]["thickness_nm"] = json!(1500000);
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":edited}));
        let failed = save_target(s.snapshot().unwrap(), Some(&loaded.snapshot), loaded.target, &mut |_| Err("Interrupted before publish".to_owned()));
        assert!(failed.is_err());
        complete(&mut files, &mut s, failed);
        assert!(files.dirty(&s));
        assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
    }
}

/// Recovery may briefly see both old and new symbol files, exceeding the final project's
/// total limit. It must still finish deleting the obsolete copy before validating the result.
#[test]
fn recovery_allows_the_bounded_union_of_old_and_new_documents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut s = session();
    let mut before = encode_documents(s.snapshot().unwrap()).unwrap();
    replacement(&mut s);
    let mut after = encode_documents(s.snapshot().unwrap()).unwrap();
    for documents in [&mut before, &mut after] {
        for (path, bytes) in documents.iter_mut() {
            let padding = if path.starts_with("library/") {
                15
            } else if path == ENTRY_POINT || path.starts_with("sheets/") {
                10
            } else {
                0
            };
            if padding != 0 {
                bytes.resize(padding * 1024 * 1024, b' ');
            }
        }
    }
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    let new_library = after.keys().find(|path| path.starts_with("library/")).unwrap();
    let stop = format!("published:{new_library}");
    let interrupted = folder::save(&root, Some(&before), &after, &mut |phase| if phase == stop { Err("crash".to_owned()) } else { Ok(()) });
    assert_eq!(interrupted.unwrap_err(), "crash", "must reach the intended interruption checkpoint");
    let recovered = folder::load(&root).unwrap();
    assert_eq!(recovered.keys().collect::<Vec<_>>(), after.keys().collect::<Vec<_>>());
    for (path, bytes) in &after {
        assert!(recovered[path] == *bytes, "document {path} differs after recovery");
    }
}

/// Start an Untitled project with one placed part, as New Project does before any save.
fn untitled() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &serde_json::Value::Null).unwrap();
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    s
}

/// A first save names an Untitled project after its folder or archive, in the saved file and in
/// the open session, without a new revision or undo step. Undo keeps the name, and the project
/// is clean once its saved snapshot is current again.
#[test]
fn first_save_names_an_untitled_project_after_its_destination() {
    let dir = tempfile::tempdir().unwrap();
    for (format, chosen, file) in
        [(SaveFormat::Folder, "blinky", "blinky"), (SaveFormat::Archive, "board", "board.pc"), (SaveFormat::Archive, "led.pc", "led.pc")]
    {
        let mut s = untitled();
        let mut files = ProjectFiles::default();
        assert!(files.dirty(&s));
        let (revision, depth) = (s.revision(), s.inspect()["can_undo"].clone());
        let result = save_new(s.snapshot().unwrap(), None, format, dir.path().join(chosen));
        assert!(matches!(complete(&mut files, &mut s, result), Some(Update::Message(text)) if text == "Project saved"));
        let name = file.strip_suffix(".pc").unwrap_or(file);
        assert_eq!(s.project().unwrap().name, name);
        assert_eq!((s.revision(), s.inspect()["can_undo"].clone()), (revision, depth));
        assert!(!files.dirty(&s), "taking the file's name is part of the save");
        assert_eq!(pcbcraft_storage::open(&dir.path().join(file)).unwrap().name, name);
        s.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert_eq!(s.project().unwrap().name, name, "undo doesn't bring Untitled back");
        s.apply("edit.redo", &serde_json::Value::Null).unwrap();
        assert!(!files.dirty(&s));
    }
    // A project given a name keeps it, whatever its destination is called.
    let mut s = Session::default();
    s.apply("project.new", &json!({"name": "Mine"})).unwrap();
    let mut files = ProjectFiles::default();
    let result = save_new(s.snapshot().unwrap(), None, SaveFormat::Folder, dir.path().join("elsewhere"));
    complete(&mut files, &mut s, result);
    assert_eq!(s.project().unwrap().name, "Mine");
    assert_eq!(pcbcraft_storage::open(&dir.path().join("elsewhere")).unwrap().name, "Mine");
}

/// Edits made while the first save runs stay in the session, under the new name and unsaved. A
/// failed or cancelled first save leaves the project Untitled with its history and dirty state.
#[test]
fn first_save_keeps_later_edits_and_failures_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = untitled();
    let mut files = ProjectFiles::default();
    let snapshot = s.snapshot().unwrap();
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1_270_000,"y_nm":0}));
    let result = save_new(snapshot, None, SaveFormat::Folder, dir.path().join("divider"));
    let update = complete(&mut files, &mut s, result);
    assert!(matches!(update, Some(Update::Message(text)) if text == "Earlier version saved. Current edits are unsaved."));
    assert_eq!(s.project().unwrap().name, "divider");
    assert_eq!(s.inspect()["project"]["schematic"]["symbols"][0]["x_nm"], 1_270_000);
    assert!(files.dirty(&s));
    s.apply("edit.undo", &serde_json::Value::Null).unwrap();
    assert!(!files.dirty(&s), "the saved snapshot is the one before the move");

    fs::write(dir.path().join("taken.pc"), b"someone else's file").unwrap();
    for result in [
        save_new(untitled().snapshot().unwrap(), None, SaveFormat::Archive, dir.path().join("taken")),
        save_new(untitled().snapshot().unwrap(), None, SaveFormat::Folder, dir.path().join("missing").join("board")),
        Ok(Outcome::Cancelled),
    ] {
        let mut s = untitled();
        let mut files = ProjectFiles::default();
        let before = s.inspect();
        let failed = result.is_err();
        let update = complete(&mut files, &mut s, result);
        assert_eq!(matches!(update, Some(Update::Error(_))), failed);
        assert_eq!(s.inspect(), before);
        assert_eq!(s.project().unwrap().name, "Untitled");
        assert!(files.dirty(&s));
    }
    assert_eq!(fs::read(dir.path().join("taken.pc")).unwrap(), b"someone else's file");
}

/// The first save's suggested file name is the project's, with characters a file name can't hold
/// replaced.
#[test]
fn first_save_suggests_the_project_name() {
    assert_eq!(file_name_for("Untitled"), "Untitled");
    assert_eq!(file_name_for("Power: 5V/3V3"), "Power- 5V-3V3");
    assert_eq!(file_name_for(" .. "), "project");
}

/// History follows accepted results, never cancelled, failed, disconnected or stale opens.
/// A second recent open cannot replace an active receiver and silently drop its file result.
#[test]
fn recent_history_records_only_successful_file_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.pc");
    let mut session = session();
    fs::write(&path, encode_packed(session.snapshot().unwrap()).unwrap()).unwrap();
    let mut files = ProjectFiles::default();
    assert!(matches!(complete(&mut files, &mut session, load(&path).map(Outcome::Opened)), Some(Update::Opened)));
    let before = files.recent.to_text();
    assert_eq!(files.recent.entries()[0].path, path);
    for outcome in [Ok(Outcome::Cancelled), Err("Failed to open".to_owned())] {
        complete(&mut files, &mut session, outcome);
        assert_eq!(files.recent.to_text(), before);
    }
    let (send, receiver) = mpsc::channel();
    let ctx = egui::Context::default();
    files.job = Some(Job { receiver, revision: session.revision() });
    assert!(files.open_path(path.clone(), session.revision(), &ctx).is_err());
    assert!(files.busy());
    send.send(load(&path).map(Outcome::Opened)).unwrap();
    session.apply("project.rename", &json!({"name":"Keep edits"})).unwrap();
    assert!(matches!(files.poll(&mut session, &ctx), Some(Update::Error(_))));
    assert_eq!(session.project().unwrap().name, "Keep edits");
    assert_eq!(files.recent.to_text(), before);
    let (send, receiver) = mpsc::channel();
    files.job = Some(Job { receiver, revision: session.revision() });
    drop(send);
    assert!(matches!(files.poll(&mut session, &ctx), Some(Update::Error(_))));
    assert!(!files.busy());
    assert_eq!(files.recent.to_text(), before);
}

/// Rotated placements survive the production folder/archive save paths, and an unchanged Save
/// retains the exact bytes. An interrupted subsequent save leaves the previously saved placement.
#[test]
fn footprint_transform_saves_offline_in_folders_and_archives() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session();
    replacement(&mut s);
    let footprint = s.inspect()["project"]["board"]["footprints"][0]["id"].clone();
    apply(&mut s, pcbcraft_engine::SET_FOOTPRINT_TRANSFORM, json!({"footprint":footprint,"x_nm":2_540_000,"y_nm":-1_270_000,"rotation":270}));
    let snapshot = s.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    let root = dir.path().join("rotated-board");
    let archive = dir.path().join("rotated-board.pc");
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    fs::write(&archive, encode_packed(snapshot.clone()).unwrap()).unwrap();
    for path in [&root, &archive] {
        let loaded = load(path).unwrap();
        let saved_bytes = if path == &archive { Some(fs::read(path).unwrap()) } else { None };
        assert_eq!(encode_documents(loaded.snapshot.clone()).unwrap(), documents);
        save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target.clone(), &mut |_| Ok(())).unwrap();
        if let Some(saved_bytes) = saved_bytes {
            assert_eq!(fs::read(path).unwrap(), saved_bytes);
        }
        assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
        s.open_snapshot(loaded.snapshot.clone());
        apply(&mut s, pcbcraft_engine::SET_FOOTPRINT_TRANSFORM, json!({"footprint":footprint,"x_nm":0,"y_nm":0,"rotation":90}));
        assert!(
            save_target(s.snapshot().unwrap(), Some(&loaded.snapshot), loaded.target, &mut |_| Err("Interrupted before publish".into())).is_err()
        );
        assert_eq!(encode_documents(load(path).unwrap().snapshot).unwrap(), documents);
    }
}

/// A failed migration leaves session metadata unchanged; success adopts one root across Undo/Redo
/// without changing electrical revision, and an unchanged second save does not stage any bytes.
#[test]
fn first_save_adopts_layout_only_on_success_and_keeps_it_through_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut s = Session::default();
    s.open_snapshot(decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap());
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    folder::save(&root, None, &before, &mut |_| Ok(())).unwrap();
    fs::create_dir(root.join("boards")).unwrap();
    fs::write(root.join("boards/notes.txt"), b"keep").unwrap();
    let mut files = ProjectFiles::default();
    complete(&mut files, &mut s, load(&root).map(Outcome::Opened));
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1_270_000,"y_nm":0}));
    let current = s.snapshot().unwrap();
    let revision = s.revision();
    let old = files.saved.as_ref().unwrap();
    let failed = save_target(current.clone(), Some(&old.snapshot), old.target.clone(), &mut |_| Err("before publication".into()));
    assert!(failed.is_err());
    complete(&mut files, &mut s, failed);
    assert!(Arc::ptr_eq(&current, &s.snapshot().unwrap()));
    assert!(s.project().unwrap().layout_root().is_none());
    assert!(!s.project().unwrap().has_sheet_occurrences());
    assert_eq!(folder::load(&root).unwrap(), before);
    let old = files.saved.as_ref().unwrap();
    let saved = save_target(current, Some(&old.snapshot), old.target.clone(), &mut |_| Ok(()));
    complete(&mut files, &mut s, saved);
    assert!(!files.dirty(&s));
    assert_eq!(s.revision(), revision);
    let adopted = s.project().unwrap().layout_root().unwrap().to_owned();
    let occurrences = s.project().unwrap().sheet_instances();
    assert!(s.project().unwrap().has_sheet_occurrences());
    let board_id = s.project().unwrap().board_target();
    assert!(adopted.starts_with("pcbcraft-data-"));
    assert!(!root.join("board.json").exists());
    for command in ["edit.undo", "edit.redo"] {
        s.apply(command, &json!({})).unwrap();
        assert_eq!(s.project().unwrap().layout_root(), Some(adopted.as_str()));
        assert_eq!(s.project().unwrap().board_target(), board_id);
        assert_eq!(s.project().unwrap().sheet_instances(), occurrences);
        let old = files.saved.as_ref().unwrap();
        let result = save_target(s.snapshot().unwrap(), Some(&old.snapshot), old.target.clone(), &mut |_| Ok(()));
        complete(&mut files, &mut s, result);
        assert!(!root.join("board.json").exists());
    }
    let old = files.saved.as_ref().unwrap();
    save_target(s.snapshot().unwrap(), Some(&old.snapshot), old.target.clone(), &mut |_| Err("no-op must not stage".into())).unwrap();
    assert_eq!(fs::read(root.join("boards/notes.txt")).unwrap(), b"keep");
}

/// Intervening edits remain dirty after migration; undo reaches the exact saved snapshot.
#[test]
fn migration_adoption_preserves_edits_made_while_the_worker_runs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut s = Session::default();
    s.open_snapshot(decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap());
    let captured = s.snapshot().unwrap();
    folder::save(&root, None, &encode_documents(captured.clone()).unwrap(), &mut |_| Ok(())).unwrap();
    let loaded = load(&root).unwrap();
    let result = save_target(captured.clone(), Some(&captured), loaded.target, &mut |_| Ok(()));
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1_270_000,"y_nm":0}));
    let mut files = ProjectFiles::default();
    complete(&mut files, &mut s, result);
    assert!(files.dirty(&s));
    assert_eq!(s.project().unwrap().layout_root(), Some("."));
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(!files.dirty(&s));
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &files.saved.as_ref().unwrap().snapshot));
}

/// A sheet created after capture retains its inspected identity when Save adopts metadata;
/// undo reaches the saved snapshot and redo restores the same occurrence without a second use.
#[test]
fn occurrence_adoption_preserves_sheets_created_after_save_capture() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut s = Session::default();
    let old = decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap();
    s.open_snapshot(Arc::new(old.with_board_folders(".").unwrap()));
    let captured = s.snapshot().unwrap();
    let initial = captured.sheet_instances();
    let documents = encode_documents(captured.clone()).unwrap();
    folder::save(&root, None, &documents, &mut |_| Ok(())).unwrap();
    let loaded = load(&root).unwrap();
    assert!(!loaded.snapshot.has_sheet_occurrences());
    let result = save_target(captured.clone(), Some(&captured), loaded.target, &mut |_| Ok(()));
    apply(&mut s, pcbcraft_engine::sheets::CREATE_SHEET, json!({"name":"Created during save"}));
    let edited = s.project().unwrap().sheet_instances();
    let mut files = ProjectFiles::default();
    complete(&mut files, &mut s, result);
    assert!(files.dirty(&s));
    assert!(s.project().unwrap().has_sheet_occurrences());
    assert_eq!(s.project().unwrap().sheet_instances(), edited);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(!files.dirty(&s));
    assert_eq!(s.project().unwrap().sheet_instances(), initial);
    s.apply("edit.redo", &json!({})).unwrap();
    assert!(files.dirty(&s));
    assert_eq!(s.project().unwrap().sheet_instances(), edited);
    let saved = files.saved.as_ref().unwrap();
    let result = save_target(s.snapshot().unwrap(), Some(&saved.snapshot), saved.target.clone(), &mut |_| Ok(()));
    complete(&mut files, &mut s, result);
    assert!(!files.dirty(&s));
    assert_eq!(load(&root).unwrap().snapshot.sheet_instances(), edited);
}
