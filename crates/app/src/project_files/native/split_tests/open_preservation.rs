//! Native Open preserves settled project documents, including during in-memory migration.

use super::{fs, load};
use std::{
    path::Path,
    time::{Duration, SystemTime},
};

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
fn stage(path: &Path, bytes: &[u8]) -> SystemTime {
    fs::write(path, bytes).unwrap();
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(path).unwrap().set_times(fs::FileTimes::new().set_modified(stamp)).unwrap();
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Check the payload and timestamp after each reader, not just after both.
fn assert_unchanged(path: &Path, bytes: &[u8], modified: SystemTime) {
    assert!(fs::read(path).unwrap().as_slice() == bytes, "{}: saved bytes changed", path.display());
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified, "{}: modification time changed", path.display());
}

/// Normal desktop and headless native loaders can prepare archives in memory without
/// leaving a rewritten archive, journal or sibling lock when the loaded state is dropped.
#[test]
fn frozen_archive_open_and_close_preserve_saved_bytes() -> Result<(), String> {
    for &(file, bytes) in ARCHIVES {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(file);
        let modified = stage(&path, bytes);
        drop(load(&path).map_err(|error| format!("desktop open {file}: {error}"))?);
        assert_unchanged(&path, bytes, modified);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "desktop open left a sidecar for {file}");
        drop(pcbcraft_storage::open(&path).map_err(|error| format!("headless open {file}: {error}"))?);
        assert_unchanged(&path, bytes, modified);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "headless open left a sidecar for {file}");
    }
    Ok(())
}

/// Opening a settled previous folder leaves its original documents and entry-point name
/// alone. The persistent sibling lock is normal folder coordination, not a saved upgrade.
#[test]
fn previous_folder_open_and_close_preserve_saved_documents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("previous");
    let mut originals = Vec::new();
    for &(file, bytes) in OLD_FOLDER {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let modified = stage(&path, bytes);
        originals.push((path, bytes, modified));
    }
    let entry = root.join("project.pc");
    let lock = temp.path().join(".previous.pcbcraft-lock");
    assert!(!lock.exists());
    drop(load(&entry).unwrap());
    for (path, bytes, modified) in &originals {
        assert_unchanged(path, bytes, *modified);
    }
    assert!(!root.join("project.json").exists());
    assert!(!root.join(".pcbcraft-save").exists());
    assert_eq!(fs::metadata(&lock).unwrap().len(), 0);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    let lock_stamp = fs::metadata(&lock).unwrap().modified().unwrap();

    drop(pcbcraft_storage::open(&entry).unwrap());
    for (path, bytes, modified) in &originals {
        assert_unchanged(path, bytes, *modified);
    }
    assert!(!root.join("project.json").exists());
    assert!(!root.join(".pcbcraft-save").exists());
    assert_unchanged(&lock, b"", lock_stamp);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}
