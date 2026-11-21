use super::*;
use pcbcraft_engine::{
    Session,
    persistence::{ENTRY_POINT, decode_packed, encode_documents},
};
use std::collections::BTreeMap;

/// Production commands/encoding provide an old-layout fixture without invented project JSON.
fn source(root: &Path) -> Documents {
    let mut session = Session::default();
    session.open_snapshot(pcbcraft_engine::persistence::decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap());
    session.apply("project.rename", &serde_json::json!({"name":"Conversion fixture"})).unwrap();
    let expected = encode_documents(session.snapshot().unwrap()).unwrap();
    fs::create_dir(root).unwrap();
    for (path, bytes) in &expected {
        let path = root.join(if path == ENTRY_POINT { "project.pc" } else { path });
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    pcbcraft_engine::persistence::migrate_board_layout(expected, ".").unwrap()
}

/// Fingerprint all source payloads independently from the conversion implementation.
fn files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, result: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, result);
            } else {
                result.insert(path.strip_prefix(root).unwrap().to_owned(), fs::read(path).unwrap());
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

/// Both new forms retain document bytes and stable identity, and source files/locks stay untouched.
#[test]
fn old_split_folder_converts_to_new_folder_and_package_without_source_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    let expected = source(&root);
    fs::write(root.join("README.md"), b"User-owned note").unwrap();
    let original = files(&root);
    let folder = dir.path().join("converted");
    let packed = dir.path().join("converted.pc");
    convert(&root.join("project.pc"), &folder, Encoding::Json).unwrap();
    assert_eq!(crate::load(&folder).unwrap(), expected);
    convert(&root, &packed, Encoding::Zip).unwrap();
    assert_eq!(decode_packed(&fs::read(&packed).unwrap()).unwrap().documents, expected);
    assert_eq!(files(&root), original);
    assert!(!dir.path().join(".old.pcbcraft-lock").exists());
    assert_eq!(crate::open(&root).unwrap().id(), crate::open(&folder).unwrap().id());
}

/// Refused inputs cannot create a destination or replace a pre-existing file/folder.
#[test]
fn conversion_refuses_overlap_existing_destinations_and_invalid_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    source(&root);
    let original = files(&root);
    for encoding in [Encoding::Json, Encoding::Zip] {
        assert!(convert(&root, &root.join("copy.pc"), encoding).is_err());
        assert!(convert(&root, &root, encoding).is_err());
        let existing = dir.path().join("existing.pc");
        fs::write(&existing, b"keep").unwrap();
        assert!(convert(&root, &existing, encoding).is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"keep");
    }
    assert_eq!(files(&root), original);
    let plain = dir.path().join("standalone.pc");
    fs::write(&plain, b"{}").unwrap();
    let target = dir.path().join("bad.pc");
    assert!(convert(&plain, &target, Encoding::Zip).is_err());
    assert!(!target.exists());
    fs::write(root.join("project.pc"), b"{}").unwrap();
    assert!(convert(&root, &target, Encoding::Zip).is_err());
    assert!(!target.exists());
}

/// On Windows, ASCII and superscript-digit device names are refused for folders and archives,
/// with or without extensions or trailing ASCII spaces, before creating anything (#156).
#[cfg(windows)]
#[test]
fn windows_conversion_refuses_device_named_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    let expected = source(&root);
    let original = files(&root);
    let out = dir.path().join("out");
    fs::create_dir(&out).unwrap();
    for device in ["con", "NUL", "com1", "aux", "lpt9", "COM¹", "com²", "CoM³", "LPT¹", "lpt²", "LpT³"] {
        for name in [device.to_owned(), format!("{device}.pc"), format!("{device} "), format!("{device}  .pc")] {
            for encoding in [Encoding::Zip, Encoding::Json] {
                let error = convert(&root, &out.join(&name), encoding).unwrap_err();
                assert!(error.contains("Windows reserves the name"), "{name}: {error}");
                assert!(fs::read_dir(&out).unwrap().next().is_none(), "{name}: nothing is created");
                assert_eq!(files(&root), original, "{name}: source is unchanged");
            }
        }
    }
    for name in ["console.pc", "COM¹-extra.pc"] {
        let destination = out.join(name);
        convert(&root, &destination, Encoding::Zip).unwrap();
        assert_eq!(decode_packed(&fs::read(destination).unwrap()).unwrap().documents, expected);
    }
}

/// Device names remain ordinary conversion destinations elsewhere; both forms reopen with
/// the expected documents and leave the source unchanged.
#[cfg(not(windows))]
#[test]
fn other_platforms_convert_superscript_device_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    let expected = source(&root);
    let original = files(&root);
    for device in ["COM¹", "com²", "CoM³", "LPT¹", "lpt²", "LpT³"] {
        let archive = dir.path().join(format!("{device}.pc"));
        convert(&root, &archive, Encoding::Zip).unwrap();
        assert_eq!(decode_packed(&fs::read(archive).unwrap()).unwrap().documents, expected);
        let folder = dir.path().join(device);
        convert(&root, &folder, Encoding::Json).unwrap();
        assert_eq!(crate::load(&folder).unwrap(), expected);
    }
    assert_eq!(files(&root), original);
}

/// Non-Windows conversion preserves trailing spaces in the chosen destination; both output
/// forms reopen without changing source bytes or silently writing to a trimmed name.
#[cfg(not(windows))]
#[test]
fn other_platforms_convert_device_names_with_trailing_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    let expected = source(&root);
    let original = files(&root);
    for (encoding, output) in [(Encoding::Zip, "archives"), (Encoding::Json, "folders")] {
        let out = dir.path().join(output);
        fs::create_dir(&out).unwrap();
        for name in ["NUL ", "COM¹ ", "LPT²  ", "nul  .pc"] {
            let destination = out.join(name);
            convert(&root, &destination, encoding).unwrap();
            let documents = match encoding {
                Encoding::Zip => decode_packed(&fs::read(&destination).unwrap()).unwrap().documents,
                Encoding::Json => crate::load(&destination).unwrap(),
            };
            assert_eq!(documents, expected, "{name:?}");
            if name.ends_with(' ') {
                assert!(!out.join(name.trim_end_matches(' ')).exists(), "{name:?}: destination was not trimmed");
            }
        }
    }
    assert_eq!(files(&root), original);
}

/// Pending recovery, an active writer and mixed entry points are never changed or recovered.
#[test]
fn conversion_refuses_busy_recovery_pending_and_mixed_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    source(&root);
    let target = dir.path().join("new");
    let lock = crate::open_lock_file(&dir.path().join(".old.pcbcraft-lock")).unwrap();
    lock.lock().unwrap();
    assert!(convert(&root, &target, Encoding::Json).is_err());
    assert!(!target.exists());
    lock.unlock().unwrap();
    fs::create_dir(root.join(".pcbcraft-save")).unwrap();
    assert!(convert(&root, &target, Encoding::Json).is_err());
    assert!(root.join(".pcbcraft-save").is_dir());
    assert!(!target.exists());
    fs::remove_dir(root.join(".pcbcraft-save")).unwrap();
    fs::copy(root.join("project.pc"), root.join(ENTRY_POINT)).unwrap();
    assert!(convert(&root, &target, Encoding::Json).is_err());
    assert!(!target.exists());
}

/// Symlinked inputs and destinations that resolve inside the source cannot bypass copy isolation.
#[cfg(unix)]
#[test]
fn conversion_refuses_symlink_sources_and_nested_destination_aliases() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old");
    source(&root);
    let alias = dir.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert!(convert(&alias, &dir.path().join("new"), Encoding::Json).is_err());
    assert!(convert(&root, &alias.join("nested"), Encoding::Json).is_err());
    let file_alias = dir.path().join("old.pc");
    symlink(root.join("project.pc"), &file_alias).unwrap();
    assert!(convert(&file_alias, &dir.path().join("new.pc"), Encoding::Zip).is_err());
}

/// A preflight rejection must not leave a new folder or lock that would prevent a corrected retry.
#[test]
fn conversion_preflight_failure_leaves_no_destination() {
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("converted");
    assert!(publish(&destination, Documents::new(), Encoding::Json).is_err());
    assert!(fs::read_dir(parent.path()).unwrap().next().is_none());
}

/// An imported project written as a new folder or package opens through the ordinary path and
/// encodes back to exactly the bytes written, so a later unchanged save rewrites nothing. The
/// destination is never replaced.
#[test]
fn a_created_import_reopens_and_saves_back_unchanged() {
    let text = include_str!("../../../kicad/tests/fixtures/divider.kicad_sch");
    let imported = pcbcraft_engine::kicad_import::import(text, "Divider").unwrap();
    let dir = tempfile::tempdir().unwrap();
    for name in ["divider", "divider.pc"] {
        let destination = dir.path().join(name);
        create(imported.project.clone(), &destination, encoding_for(&destination)).unwrap();
        let opened = crate::open(&destination).unwrap();
        let written = if encoding_for(&destination) == Encoding::Zip {
            decode_packed(&fs::read(&destination).unwrap()).unwrap().documents
        } else {
            crate::folder::load(&destination).unwrap()
        };
        assert_eq!(encode_documents(opened.clone()).unwrap(), written, "{name}");
        assert_eq!(opened.symbols().len(), 6);
        assert!(create(imported.project.clone(), &destination, encoding_for(&destination)).unwrap_err().contains("unused destination"));
    }
}
