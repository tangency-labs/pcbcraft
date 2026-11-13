//! Read-only, bounded native library folders. Call from a worker, never the editor loop.
use pcbcraft_engine::library::sources::{Documents, MAX_FILE_BYTES, MAX_FILES, MAX_SOURCE_BYTES, NativeSource, is_document_path};
use std::{fs, io::Read, path::Path};

/// Read only library.json and flat native item directories. Symlinks and special
/// files are refused. Bad item files become individual diagnostics; a bad manifest or source
/// exceeding aggregate bounds fails without returning a partial snapshot. No locks or writes occur.
pub fn load(root: &Path) -> Result<NativeSource, String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !metadata.is_dir() {
        return Err("Library source must be a directory, not a symlink or special file".into());
    }
    let mut documents = Documents::new();
    documents.insert("library.json".into(), read_file(&root.join("library.json"))?);
    let mut problems = Vec::new();
    let mut total = documents["library.json"].len();
    let mut count = 1;
    for directory in ["symbols", "footprints", "categories", "parts", "manufacturer-parts"] {
        let path = root.join(directory);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{directory}: {error}")),
        };
        if !metadata.is_dir() {
            problems.push(format!("{directory}: must be a regular directory, not a symlink"));
            continue;
        }
        let mut paths = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            count += 1;
            if count > MAX_FILES {
                return Err(format!("Library source exceeds {MAX_FILES} files"));
            }
            paths.push(entry.map_err(|e| e.to_string())?.path());
        }
        paths.sort();
        for path in paths {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                problems.push(format!("{directory}: non-UTF-8 filename is unsupported"));
                continue;
            };
            let relative = format!("{directory}/{name}");
            if !is_document_path(&relative) {
                problems.push(format!("{relative}: unsupported native item path"));
                continue;
            }
            match read_file(&path) {
                Ok(text) => {
                    total += text.len();
                    if total > MAX_SOURCE_BYTES {
                        return Err("Library source exceeds 8 MiB".into());
                    }
                    documents.insert(relative, text);
                }
                Err(error) => problems.push(format!("{relative}: {error}")),
            }
        }
    }
    let mut source = NativeSource::load(&documents)?;
    source.add_adapter_problems(problems);
    Ok(source)
}

/// Bound regular-file reads even if a file grows after metadata was checked; invalid UTF-8 fails.
fn read_file(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Not a regular file (symlinks and special files are unsupported)".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("File exceeds 1 MiB".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Not a regular file".into());
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES as u64 + 1).read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() > MAX_FILE_BYTES {
        return Err("File exceeds 1 MiB".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Categorized parts and a multi-unit part resolve through the real bounded folder adapter.
    #[test]
    fn native_part_examples_resolve_exact_dependencies_from_a_folder() {
        use pcbcraft_engine::library::{parts::Reference, sources::Catalog};
        use std::sync::Arc;
        let source = load(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-parts"))).unwrap();
        assert_eq!(source.items().len(), 14);
        assert!(source.problems().is_empty(), "{:?}", source.problems());
        let mut catalog = Catalog::default();
        catalog.install("demo", Arc::new(source)).unwrap();
        for id in ["resistor", "connector", "ic"] {
            let reference = Reference { library: "parts-demo".into(), id: id.into(), revision: 1 };
            let result = catalog.resolve_part(&reference).unwrap();
            assert!(result.category.is_some());
            assert_eq!(result.footprints.is_empty(), id != "resistor");
        }
        let reference = Reference { library: "parts-demo".into(), id: "dual-amplifier".into(), revision: 1 };
        let result = catalog.resolve_part(&reference).unwrap();
        assert_eq!(result.symbol.units().len(), 2);
        assert_eq!(result.footprints.len(), 1);
    }

    /// The documented original fixture is an actual loadable symbol-and-footprint source.
    #[test]
    fn documented_fixture_loads_both_native_item_kinds() {
        let source = load(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library"))).unwrap();
        assert!(source.problems().is_empty());
        assert_eq!(source.items().len(), 2);
    }

    /// An original source reuses our licensed starter geometry under a different library identity.
    fn fixture(root: &Path) {
        fs::create_dir(root.join("symbols")).unwrap();
        fs::write(
            root.join("library.json"),
            r#"{"format":"pcbcraft-library/1","id":"local-demo","name":"Local demo","license":"MIT OR Apache-2.0"}"#,
        )
        .unwrap();
        fs::write(root.join("symbols/resistor.json"), include_str!("../../../libraries/starter/symbols/resistor.json")).unwrap();
    }

    /// Folder moves retain identities; bad files don't hide good ones, and no source file is written.
    #[test]
    fn folder_loading_is_read_only_and_independent_of_location() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        fs::create_dir(&original).unwrap();
        fixture(&original);
        fs::write(original.join("symbols/bad.json"), "bad").unwrap();
        fs::write(original.join("symbols/large.json"), vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
        let loaded = load(&original).unwrap();
        assert_eq!(loaded.items().len(), 1);
        assert_eq!(loaded.problems().len(), 2);
        let bytes = fs::read(original.join("symbols/resistor.json")).unwrap();
        let moved = root.path().join("moved");
        fs::rename(&original, &moved).unwrap();
        let again = load(&moved).unwrap();
        assert_eq!(again.items()[0].key, loaded.items()[0].key);
        assert_eq!(bytes, fs::read(moved.join("symbols/resistor.json")).unwrap());
        assert!(load(&original).is_err());
    }

    /// A symlink cannot make a source quietly ingest another folder's files.
    #[cfg(unix)]
    #[test]
    fn symlinks_are_diagnosed_without_hiding_regular_items() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        std::os::unix::fs::symlink(root.path().join("symbols/resistor.json"), root.path().join("symbols/linked.json")).unwrap();
        let loaded = load(root.path()).unwrap();
        assert_eq!(loaded.items().len(), 1);
        assert!(loaded.problems().iter().any(|problem| problem.contains("linked.json") && problem.contains("symlink")));
    }
}

/// Validate a bundle before reserving an unused destination, then write only bounded native
/// documents. Existing paths are never replaced; partial writes remove only our new folder.
/// Registration belongs to the host and happens only after this function succeeds.
pub fn install_bundle(bytes: &[u8], destination: &Path) -> Result<NativeSource, String> {
    use std::io::Write;
    let documents = NativeSource::bundle_documents(bytes)?;
    let source = NativeSource::load(&documents)?;
    let parent = destination.parent().ok_or("Installation requires a destination folder")?;
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let name = destination.file_name().filter(|name| *name != "." && *name != "..").ok_or("Choose a new folder name")?;
    let destination = parent.join(name);
    fs::create_dir(&destination).map_err(|e| format!("Choose an unused destination; no existing folder is overwritten: {e}"))?;
    let write = || -> Result<(), String> {
        for (relative, text) in &documents {
            let path = destination.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
            file.write_all(text.as_bytes()).and_then(|()| file.sync_all()).map_err(|e| e.to_string())?;
        }
        let directories: std::collections::BTreeSet<_> = documents.keys().filter_map(|path| std::path::Path::new(path).parent()).collect();
        for directory in directories {
            super::folder::sync_dir(&destination.join(directory))?;
        }
        super::folder::sync_dir(&destination)?;
        super::folder::sync_dir(&parent)
    };
    if let Err(error) = write() {
        return match fs::remove_dir_all(&destination) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; incomplete installation remains at {}: {cleanup}", destination.display())),
        };
    }
    Ok(source)
}

/// Check the regular root and its permission bits on the I/O worker. The destructive
/// operation rechecks permissions and lets the platform enforce ACLs when moving to trash.
pub fn writable(root: &Path) -> bool {
    fs::symlink_metadata(root).is_ok_and(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
}

/// Read the actual folder on a worker before offering destructive source management.
/// Symlinks, missing manifests and read-only roots are refused; count includes all revisions.
pub fn deletion_snapshot(root: &Path) -> Result<NativeSource, String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.permissions().readonly() {
        return Err("Delete is available only for writable library folders".into());
    }
    load(root)
}

/// Move a confirmed library folder to the platform's trash. A failed or unavailable trash
/// service preserves the registration and reports an error; there is no permanent-delete fallback.
pub fn trash_library(root: &Path) -> Result<(), String> {
    deletion_snapshot(root)?;
    let path = if root.is_absolute() { root.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(root) };
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let status = std::process::Command::new("gio").arg("trash").arg("--").arg(&path).status();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("osascript")
        .args(["-e", "on run argv\ntell application \"Finder\" to delete (POSIX file (item 1 of argv) as alias)\nend run", "--"])
        .arg(&path)
        .status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:PCBCRAFT_TRASH_PATH, 'OnlyErrorDialogs', 'SendToRecycleBin')"])
        .env("PCBCRAFT_TRASH_PATH", &path).status();
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos", target_os = "windows")))]
    return Err(format!("System trash is unavailable for {}", path.display()));
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos", target_os = "windows"))]
    if status.map_err(|e| format!("Could not start system trash: {e}"))?.success()
        && fs::symlink_metadata(root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        Ok(())
    } else {
        Err("System trash did not remove the library folder; registration retained".into())
    }
}

#[cfg(test)]
mod installation_tests {
    use super::*;
    use pcbcraft_engine::library::sources::{Catalog, Documents};
    use std::sync::Arc;
    /// Original native fixtures exercise validation and no-clobber publication through real I/O.
    fn bundle() -> Vec<u8> {
        let documents = Documents::from([
            (
                "library.json".into(),
                r#"{"format":"pcbcraft-library/1","id":"install-demo","name":"Install demo","license":"MIT OR Apache-2.0"}"#.into(),
            ),
            ("symbols/resistor.json".into(), include_str!("../../../libraries/starter/symbols/resistor.json").into()),
        ]);
        serde_json::to_vec(&serde_json::json!({"format":"pcbcraft-library-bundle/1","documents":documents})).unwrap()
    }
    /// A completed installation round-trips through the folder loader and detects redundant content.
    #[test]
    fn install_validates_before_writing_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("library");
        let bytes = bundle();
        let source = install_bundle(&bytes, &destination).unwrap();
        assert_eq!(source.items().len(), 1);
        let original = fs::read(destination.join("symbols/resistor.json")).unwrap();
        let mut catalog = Catalog::default();
        catalog.install("installed", Arc::new(source)).unwrap();
        assert!(catalog.contains_source(&NativeSource::from_bundle(&bytes).unwrap()));
        assert!(install_bundle(&bytes, &destination).is_err());
        assert_eq!(fs::read(destination.join("symbols/resistor.json")).unwrap(), original);
        assert_eq!(load(&destination).unwrap().items()[0].inspect(), catalog.source_items("installed")[0].inspect());
        let bad = root.path().join("bad");
        assert!(install_bundle(b"bad", &bad).is_err());
        assert!(!bad.exists());
        let mut traversal: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        traversal["documents"]["../escape.json"] = serde_json::json!("{}");
        assert!(install_bundle(&serde_json::to_vec(&traversal).unwrap(), &bad).is_err());
        assert!(!bad.exists());
    }
    /// A destination symlink is an existing path and cannot be followed or replaced.
    #[cfg(unix)]
    #[test]
    fn installation_refuses_symlink_and_readonly_delete() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("link");
        std::os::unix::fs::symlink(root.path(), &destination).unwrap();
        assert!(install_bundle(&bundle(), &destination).is_err());
        assert!(!root.path().join("library.json").exists());
        assert!(deletion_snapshot(&destination).is_err());
        let actual = root.path().join("library");
        install_bundle(&bundle(), &actual).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o555)).unwrap();
        assert!(deletion_snapshot(&actual).is_err());
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
