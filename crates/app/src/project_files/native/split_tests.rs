//! Filesystem and worker acceptance tests for folder saves and v2 archive saves.
use super::*;
use crate::project_files::{Job, ProjectFiles, Update};
use pcbcraft_engine::persistence::encode_packed;
use pcbcraft_engine::{
    DELETE_SYMBOL, MOVE_SYMBOL, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    persistence::{decode, encode_documents},
};
use serde_json::json;
use std::sync::{Barrier, mpsc};

/// Start from the checked-in migration fixture so comparisons retain stable UUIDs.
fn session() -> Session {
    let mut s = Session::default();
    s.open_snapshot(decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap());
    s
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
    assert!(harness.query_by_label("Move Parts library").is_none());
    harness.key_press(egui::Key::A);
    harness.run();
    harness.get_by_label("Move Parts library");
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

/// Recovery also works when the first save crashed before project.pc itself was installed.
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
    fs::write(root.join("board.json"), b"broken").unwrap();
    assert!(matches!(complete(&mut files, &mut s, load(&root).map(Outcome::Opened)), Some(Update::Error(_))));
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    assert!(files.dirty(&s));
    let saved = files.saved.as_ref().unwrap();
    assert_eq!(
        save_target(snapshot, Some(&saved.snapshot), saved.target.clone(), &mut |_| Ok(())).err().unwrap(),
        "Project folder changed on disk. Use Save As to keep your edits in a separate folder."
    );
    assert_eq!(fs::read(root.join("board.json")).unwrap(), b"broken");
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
    fs::write(root.join("board.json"), b"external").unwrap();
    assert!(folder::load(&root).is_err());
    assert_eq!(fs::read(root.join("board.json")).unwrap(), b"external");
    fs::write(root.join("board.json"), &before["board.json"]).unwrap();
    assert_eq!(folder::load(&root).unwrap(), after);
}

/// Normal folder opens leave unrelated root files alone and refuse unknown owned files and links.
#[test]
fn unknown_owned_files_and_symlinks_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let d = encode_documents(session().snapshot().unwrap()).unwrap();
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
        fs::remove_file(root.join("board.json")).unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, &d["board.json"]).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("board.json")).unwrap();
        assert!(load(&root).is_err());
        assert!(folder::save(&root, Some(&d), &d, &mut |_| Ok(())).is_err());
        assert_eq!(fs::read(outside).unwrap(), d["board.json"]);
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
fn v2_archive_saves_and_folder_conversion_preserve_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let s = session();
    let mut bytes = encode_packed(s.snapshot().unwrap()).unwrap();
    let comment = b"custom comment";
    let length = bytes.len();
    bytes[length - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    let path = dir.path().join("project.pc.zip");
    fs::write(&path, &bytes).unwrap();
    let loaded = load(&path).unwrap();
    assert!(!loaded.target.legacy);
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
    let path = dir.path().join("hand-zipped.pc.zip");
    fs::write(&path, include_bytes!("../../../tests/fixtures/split/two-resistors-no-manifest.pc.zip")).unwrap();
    let loaded = load(&path).unwrap();
    assert!(loaded.target.needs_manifest);
    assert!(loaded.target.open_notice().contains("without a manifest"));
    let documents = loaded.target.documents.clone().unwrap();
    let outcome = save_target(loaded.snapshot.clone(), Some(&loaded.snapshot), loaded.target, &mut |_| Ok(())).unwrap();
    let Outcome::Saved(saved, _) = outcome else { panic!("expected saved project") };
    assert!(saved.target.added_manifest());
    let reopened = load(&path).unwrap();
    assert!(!reopened.target.needs_manifest);
    assert_eq!(reopened.target.documents.as_deref(), Some(documents.as_ref()));
    let mut custom = (*documents).clone();
    custom.get_mut(ENTRY_POINT).unwrap().extend_from_slice(b"\n  ");
    let root = dir.path().join("custom");
    folder::save(&root, None, &custom, &mut |_| Ok(())).unwrap();
    let loaded = load(&root).unwrap();
    let converted = working_documents(&loaded.snapshot, Some(&loaded.snapshot), loaded.target.documents.as_deref()).unwrap();
    let packed = encode_packed_documents(converted).unwrap();
    assert_eq!(decode_packed(&packed).unwrap().documents.as_ref(), Some(&custom));
}

/// The shared headless reader accepts the same folder, entry point and archive as desktop.
#[test]
fn headless_and_desktop_read_the_same_split_project() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/split");
    let expected = encode_documents(session().snapshot().unwrap()).unwrap();
    for path in [
        fixture.join("two-resistors"),
        fixture.join("two-resistors/project.pc"),
        fixture.join("two-resistors.pc.zip"),
        fixture.join("two-resistors-no-manifest.pc.zip"),
    ] {
        assert_eq!(encode_documents(pcbcraft_storage::open(&path).unwrap()).unwrap(), expected);
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
    assert!(folder::save(&root, Some(&before), &after, &mut |phase| if phase == stop { Err("crash".to_owned()) } else { Ok(()) }).is_err());
    assert_eq!(folder::load(&root).unwrap(), after);
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
        [(SaveFormat::Folder, "blinky", "blinky"), (SaveFormat::Archive, "board", "board.pc.zip"), (SaveFormat::Archive, "led.pc.zip", "led.pc.zip")]
    {
        let mut s = untitled();
        let mut files = ProjectFiles::default();
        assert!(files.dirty(&s));
        let (revision, depth) = (s.revision(), s.inspect()["can_undo"].clone());
        let result = save_new(s.snapshot().unwrap(), None, format, dir.path().join(chosen));
        assert!(matches!(complete(&mut files, &mut s, result), Some(Update::Message(text)) if text == "Project saved"));
        let name = file.strip_suffix(".pc.zip").unwrap_or(file);
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

    fs::write(dir.path().join("taken.pc.zip"), b"someone else's file").unwrap();
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
    assert_eq!(fs::read(dir.path().join("taken.pc.zip")).unwrap(), b"someone else's file");
}

/// The first save's suggested file name is the project's, with characters a file name can't hold
/// replaced.
#[test]
fn first_save_suggests_the_project_name() {
    assert_eq!(file_name_for("Untitled"), "Untitled");
    assert_eq!(file_name_for("Power: 5V/3V3"), "Power- 5V-3V3");
    assert_eq!(file_name_for(" .. "), "project");
}
