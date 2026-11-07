//! Read-only, bounded native library folders. Call from a worker, never the editor loop.
use pcbcraft_engine::library::sources::{Documents, MAX_FILE_BYTES, MAX_FILES, MAX_SOURCE_BYTES, NativeSource, is_document_path};
use std::{fs, io::Read, path::Path};

/// Read only library.json and the flat symbols/footprints directories. Symlinks and special
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
    for directory in ["symbols", "footprints"] {
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
