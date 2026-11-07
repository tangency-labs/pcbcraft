//! Native filesystem access, shared by desktop file workers and the headless CLI.

use pcbcraft_engine::{
    Project,
    persistence::{ENTRY_POINT, Encoding, decode_documents, decode_file, read_bytes},
};
use std::{
    fs::{self, File},
    path::Path,
    sync::Arc,
};

mod folder;
pub mod library;
pub use folder::{load, save};

/// Open a folder, its project.pc entry point, a packed archive or a legacy single-file project.
/// Interrupted folder saves recover under the writer lock before any documents are adopted.
pub fn open(path: &Path) -> Result<Arc<Project>, String> {
    open_with(path, load)
}

/// Read a disposable preview without creating locks, recovering interrupted saves, or writing
/// project data. Busy, invalid or recovery-pending projects return an error for a placeholder.
pub fn preview(path: &Path) -> Result<Arc<Project>, String> {
    open_with(path, folder::preview)
}

/// Share bounded format detection and decoding while the caller selects recovery or read-only I/O.
fn open_with(path: &Path, folder: fn(&Path) -> Result<pcbcraft_engine::persistence::Documents, String>) -> Result<Arc<Project>, String> {
    let name = path.file_name().ok_or("Project path has no filename")?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let path = parent.join(name);
    if name == ENTRY_POINT && parent.join(".pcbcraft-save").try_exists().map_err(|e| e.to_string())? {
        return decode_documents(&folder(&parent)?);
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        return decode_documents(&folder(&path)?);
    }
    if !metadata.is_file() {
        return Err("Project path is not a regular file or directory.".to_owned());
    }
    let bytes = read_bytes(File::open(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if Encoding::detect(&bytes) == Encoding::Json {
        let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if matches!(header["format"].as_str(), Some("pcbcraft-project/5" | "pcbcraft-project/6")) {
            if name != ENTRY_POINT {
                return Err("A split project's entry point must be named project.pc.".to_owned());
            }
            return decode_documents(&folder(&parent)?);
        }
    }
    decode_file(&bytes).map(|(project, _)| project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::{
        Session,
        persistence::{encode_documents, encode_packed},
    };
    use std::{collections::BTreeMap, path::PathBuf};

    /// A real project with an original resistor, created through the shared command set.
    fn project() -> Arc<Project> {
        let mut session = Session::default();
        session.apply("project.new", &serde_json::json!({"name":"Preview"})).unwrap();
        session
            .apply(
                pcbcraft_engine::PLACE_SYMBOL,
                &serde_json::json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()}),
            )
            .unwrap();
        session.snapshot().unwrap()
    }

    /// Capture every name and file byte so previews cannot silently create locks or recover data.
    fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut output = BTreeMap::new();
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                output.extend(tree(&path));
            } else {
                output.insert(path.clone(), fs::read(path).unwrap());
            }
        }
        output
    }

    /// Preview folders and entry points without creating a save lock, and archives without writes.
    #[test]
    fn preview_is_read_only_and_uses_the_native_formats() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        for (path, bytes) in encode_documents(project()).unwrap() {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let archive = dir.path().join("project.pc.zip");
        fs::write(&archive, encode_packed(project()).unwrap()).unwrap();
        let before = tree(dir.path());
        for path in [&root, &root.join(ENTRY_POINT), &archive] {
            assert_eq!(preview(path).unwrap().symbols().len(), 1);
        }
        assert_eq!(tree(dir.path()), before);
    }

    /// Contended projects return immediately; pending recovery stays untouched until an explicit
    /// ordinary open completes it. Neither failure can turn a thumbnail into a write operation.
    #[test]
    fn preview_refuses_active_writers_and_never_recovers_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let original = project();
        let documents = encode_documents(original.clone()).unwrap();
        save(&root, None, &documents, &mut |_| Ok(())).unwrap();
        let lock = fs::OpenOptions::new().read(true).write(true).open(dir.path().join(".project.pcbcraft-lock")).unwrap();
        lock.try_lock().unwrap();
        assert!(preview(&root).is_err());
        lock.unlock().unwrap();
        let changed = encode_documents(Arc::new(original.renamed("Changed").unwrap())).unwrap();
        let result = save(&root, Some(&documents), &changed, &mut |phase| {
            if phase == "journal" { Err("Simulated interruption".to_owned()) } else { Ok(()) }
        });
        assert!(result.is_err());
        let before = tree(dir.path());
        assert!(preview(&root).is_err());
        assert!(preview(&root.join(ENTRY_POINT)).is_err());
        assert_eq!(tree(dir.path()), before);
        assert_eq!(open(&root).unwrap().name, "Changed");
    }
}
