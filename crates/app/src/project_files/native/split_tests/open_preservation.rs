//! Native Open preserves settled project documents, including during in-memory migration.

use super::{fs, load};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const FOLDER_LOCK: &str = ".previous.pcbcraft-lock";

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
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

/// Give the fixture an old mtime so even an immediate byte-for-byte rewrite is detectable.
fn stage(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(path).unwrap().set_times(fs::FileTimes::new().set_modified(stamp)).unwrap();
}

/// Capture all entries, bytes and file mtimes below the fixture root. Folder tests may exclude
/// only the exact sibling lock path; the same name anywhere inside the project is included.
fn saved_tree(root: &Path, excluded: Option<&Path>) -> BTreeMap<PathBuf, Entry> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if Some(path.as_path()) == excluded {
                continue;
            }
            let kind = entry.file_type().unwrap();
            let value = if kind.is_dir() {
                pending.push(path.clone());
                Entry::Directory
            } else {
                assert!(kind.is_file(), "unexpected nonregular entry: {}", path.display());
                Entry::File { bytes: fs::read(&path).unwrap(), modified: entry.metadata().unwrap().modified().unwrap() }
            };
            entries.insert(path.strip_prefix(root).unwrap().to_owned(), value);
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

/// Normal desktop and headless native loaders can prepare archives in memory without
/// leaving a rewritten archive, journal or sibling lock when the loaded state is dropped.
#[test]
fn frozen_archive_open_and_close_preserve_saved_bytes() -> Result<(), String> {
    for &(file, bytes) in ARCHIVES {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(file);
        stage(&path, bytes);
        let before = saved_tree(root.path(), None);
        drop(load(&path).map_err(|error| format!("desktop open {file}: {error}"))?);
        assert_eq!(saved_tree(root.path(), None), before, "desktop Open changed {file} or left a sidecar");
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
    for &(file, bytes) in OLD_FOLDER {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        stage(&path, bytes);
    }
    let entry = root.join("project.pc");
    let lock = temp.path().join(FOLDER_LOCK);
    assert!(!lock.exists());
    let before = saved_tree(temp.path(), Some(&lock));

    drop(load(&entry).unwrap());
    assert_eq!(saved_tree(temp.path(), Some(&lock)), before, "desktop Open changed the saved folder or left another sidecar");
    assert_lock_released(&lock);

    drop(pcbcraft_storage::open(&entry).unwrap());
    assert_eq!(saved_tree(temp.path(), Some(&lock)), before, "headless Open changed the saved folder or left another sidecar");
    assert_lock_released(&lock);
}
