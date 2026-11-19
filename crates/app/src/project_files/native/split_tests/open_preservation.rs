//! Native Open preserves settled project documents, including during in-memory migration.

use super::{fs, load};
use crate::{
    Dialog, PcbApp,
    project_files::{Job, Outcome},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, SystemTime},
};

const FOLDER_LOCK: &str = ".previous.pcbcraft-lock";

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory { modified: SystemTime },
    File { bytes: Vec<u8>, modified: SystemTime },
}

const ARCHIVES: &[(&str, &[u8])] = &[
    ("two-resistors-v2.pc.zip", include_bytes!("../../../../../storage/tests/fixtures/native-open/two-resistors-v2.pc.zip")),
    (
        "two-resistors-previous-no-manifest.pc.zip",
        include_bytes!("../../../../../storage/tests/fixtures/native-open/two-resistors-previous-no-manifest.pc.zip"),
    ),
    ("dual-led-editing-demo-v3.pc", include_bytes!("../../../../../storage/tests/fixtures/native-open/dual-led-editing-demo-v3.pc")),
    ("dual-led-flasher-v3.pc", include_bytes!("../../../../../storage/tests/fixtures/native-open/dual-led-flasher-v3.pc")),
];

// These are the unchanged 94053f21 split-fixture blobs. Its entry-point blob is now
// tracked at project.json; stage those same bytes under the original project.pc path.
const OLD_FOLDER: &[(&str, &[u8])] = &[
    ("project.pc", include_bytes!("../../../../tests/fixtures/split/two-resistors/project.json")),
    ("board.json", include_bytes!("../../../../tests/fixtures/split/two-resistors/board.json")),
    (
        "sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json",
        include_bytes!("../../../../tests/fixtures/split/two-resistors/sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json"),
    ),
    (
        "library/symbols/starter/resistor.r1.json",
        include_bytes!("../../../../tests/fixtures/split/two-resistors/library/symbols/starter/resistor.r1.json"),
    ),
];

/// Give a file or directory an old mtime so even an immediate rewrite is detectable.
fn stamp(handle: fs::File, offset_seconds: u64) {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000 + offset_seconds);
    handle.set_times(fs::FileTimes::new().set_modified(time)).unwrap();
}

/// Stage the unchanged bytes before setting the old mtime.
fn stage(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    stamp(fs::File::options().write(true).open(path).unwrap(), 0);
}

/// Open an existing Windows directory with permission to set its mtime.
#[cfg(windows)]
fn directory_handle(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::File::options().access_mode(FILE_WRITE_ATTRIBUTES).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path).unwrap()
}

/// A read handle suffices to set the mtime of a directory owned by this test.
#[cfg(not(windows))]
fn directory_handle(path: &Path) -> fs::File {
    fs::File::open(path).unwrap()
}

/// Stamp the project root and nested directories after staging has populated them.
fn stage_previous_folder(root: &Path) {
    let mut directories = BTreeSet::new();
    for &(file, bytes) in OLD_FOLDER {
        let path = root.join(file);
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        stage(&path, bytes);
        directories.extend(parent.ancestors().take_while(|ancestor| ancestor.starts_with(root)).map(Path::to_owned));
    }
    for (index, directory) in directories.iter().enumerate() {
        stamp(directory_handle(directory), 2 * index as u64);
    }
}

/// Capture all entries, file bytes and mtimes. Exclude only the exact sibling lock and
/// the synthetic outer root's mtime if that lock is allowed; the project root is still covered.
fn saved_tree(root: &Path, sibling_lock: Option<&Path>) -> BTreeMap<PathBuf, Entry> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        if directory != root || sibling_lock.is_none() {
            let modified = fs::metadata(&directory).unwrap().modified().unwrap();
            entries.insert(directory.strip_prefix(root).unwrap().to_owned(), Entry::Directory { modified });
        }
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if Some(path.as_path()) == sibling_lock {
                continue;
            }
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(path);
            } else {
                assert!(kind.is_file(), "unexpected nonregular entry: {}", path.display());
                let value = Entry::File { bytes: fs::read(&path).unwrap(), modified: entry.metadata().unwrap().modified().unwrap() };
                entries.insert(path.strip_prefix(root).unwrap().to_owned(), value);
            }
        }
    }
    entries
}

/// The sidecar persists, but an independent handle must acquire the exclusive lock without waiting.
fn assert_lock_released(path: &Path) {
    assert!(fs::symlink_metadata(path).unwrap().is_file(), "expected a regular lock file: {}", path.display());
    let lock = fs::File::options().read(true).write(true).open(path).unwrap();
    let acquired = lock.try_lock();
    assert!(acquired.is_ok(), "expected exclusive lock to be available at {}: {acquired:?}", path.display());
    lock.unlock().unwrap();
}

/// Supply the picker result to the real editor frame, then Quit without starting a Save.
fn desktop_open_and_close(path: &Path) -> Result<(), String> {
    let loaded = load(path)?;
    let opened = Arc::clone(loaded.opened.as_ref().unwrap_or(&loaded.snapshot));
    let mut app = PcbApp::default();
    assert!(app.session.snapshot().is_none());
    let ctx = egui::Context::default();
    let (send, receiver) = mpsc::channel();
    app.files.job = Some(Job { receiver, revision: app.session.revision(), operation: "open", destination: None });
    send.send(Ok(Outcome::Opened(loaded))).unwrap();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.frame(ui));
    assert!(app.session.project().is_some_and(|current| current.same_document(&opened)), "editor did not adopt {}", path.display());
    assert!(!app.files.dirty(&app.session), "opening {} made it dirty", path.display());
    assert!(!app.close_allowed);
    super::super::window_tests::request_close(&mut app, &ctx);
    assert!(app.close_allowed, "Quit must proceed without Save for {}", path.display());
    assert!(matches!(app.dialog, Dialog::None));
    assert!(!app.files.busy());
    Ok(())
}

/// Desktop Open/Quit and headless Open/drop preserve the archive without leaving any sidecar.
#[test]
fn frozen_archive_open_and_close_preserve_saved_bytes() -> Result<(), String> {
    for &(file, bytes) in ARCHIVES {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(file);
        stage(&path, bytes);
        stamp(directory_handle(root.path()), 0);
        let before = saved_tree(root.path(), None);
        desktop_open_and_close(&path).map_err(|error| format!("desktop open {file}: {error}"))?;
        assert_eq!(saved_tree(root.path(), None), before, "desktop Open or Quit changed {file} or left a sidecar");
        drop(pcbcraft_storage::open(&path).map_err(|error| format!("headless open {file}: {error}"))?);
        assert_eq!(saved_tree(root.path(), None), before, "headless Open changed {file} or left a sidecar");
    }
    Ok(())
}

/// Preserve the entire settled folder and its parent entries, except for the documented
/// sibling lock file. Verify that each native loader releases the advisory lock.
#[test]
fn previous_folder_open_and_close_preserve_saved_documents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("previous");
    stage_previous_folder(&root);
    let entry = root.join("project.pc");
    let lock = temp.path().join(FOLDER_LOCK);
    assert!(!lock.exists());
    let before = saved_tree(temp.path(), Some(&lock));

    desktop_open_and_close(&entry).unwrap();
    assert_eq!(saved_tree(temp.path(), Some(&lock)), before, "desktop Open or Quit changed the saved folder or left another sidecar");
    assert_lock_released(&lock);

    drop(pcbcraft_storage::open(&entry).unwrap());
    assert_eq!(saved_tree(temp.path(), Some(&lock)), before, "headless Open changed the saved folder or left another sidecar");
    assert_lock_released(&lock);
}

/// A touch changes the oracle for both the project root and a nested directory even when
/// every entry name and file byte is unchanged. The approved sibling alone is ignored.
#[test]
fn folder_oracle_detects_root_and_nested_directory_touches() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("previous");
    stage_previous_folder(&root);
    let lock = temp.path().join(FOLDER_LOCK);
    let before = saved_tree(temp.path(), Some(&lock));
    fs::write(&lock, []).unwrap();
    assert_eq!(saved_tree(temp.path(), Some(&lock)), before);

    for path in [&root, &root.join("sheets")] {
        let before = saved_tree(temp.path(), Some(&lock));
        stamp(directory_handle(path), 120);
        let after = saved_tree(temp.path(), Some(&lock));
        assert_ne!(after, before, "mtime-only touch was missed for {}", path.display());
        assert!(after.keys().eq(before.keys()));
    }
}
