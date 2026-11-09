//! Native filesystem access, shared by desktop file workers and the headless CLI.

use pcbcraft_engine::{
    Project,
    persistence::{ENTRY_POINT, Encoding, decode_documents, decode_file, read_bytes},
};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    sync::Arc,
};

mod folder;
pub mod library;
pub use folder::{load, save};

/// Windows' `FILE_ATTRIBUTE_HIDDEN`, which Explorer uses to hide a file.
#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// Open the sibling save lock at `path` for reading and writing, creating it if it's missing and
/// never truncating it. It serializes writers of a folder or archive and is kept after they finish.
/// On Windows a new lock file is created hidden, as its leading dot hides it on Linux and macOS;
/// Windows applies the attribute only when it creates the file, so an existing lock file opens
/// with its attributes unchanged. Errors are the operating system's, for the caller to report.
pub fn open_lock_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.attributes(FILE_ATTRIBUTE_HIDDEN);
    }
    options.open(path)
}

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
        if matches!(header["format"].as_str(), Some("pcbcraft-project/5" | "pcbcraft-project/6" | "pcbcraft-project/7" | "pcbcraft-project/9")) {
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

    /// Folder publication adds/removes only the managed physical file as creation is redone/undone.
    #[test]
    fn schematic_only_folder_can_create_undo_and_reopen_its_board() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("optional-board");
        let mut session = Session::default();
        session.apply("project.new", &serde_json::json!({})).unwrap();
        let empty = encode_documents(session.snapshot().unwrap()).unwrap();
        save(&root, None, &empty, &mut |_| Ok(())).unwrap();
        fs::write(root.join("README.md"), b"User notes").unwrap();
        assert!(!root.join("board.json").exists());
        assert!(open(&root.join(ENTRY_POINT)).unwrap().board_target().is_none());
        session.apply("pcb.create_board", &serde_json::json!({"expected_revision":session.revision()})).unwrap();
        let target = session.project().unwrap().board_target().unwrap();
        let created = encode_documents(session.snapshot().unwrap()).unwrap();
        save(&root, Some(&empty), &created, &mut |_| Ok(())).unwrap();
        assert!(root.join("board.json").exists());
        assert_eq!(open(&root).unwrap().board_target(), Some(target));
        assert_eq!(preview(&root.join(ENTRY_POINT)).unwrap().board_target(), Some(target));
        session.apply("edit.undo", &serde_json::json!({})).unwrap();
        let undone = encode_documents(session.snapshot().unwrap()).unwrap();
        save(&root, Some(&created), &undone, &mut |_| Ok(())).unwrap();
        assert!(!root.join("board.json").exists());
        assert!(open(&root).unwrap().board_target().is_none());
        assert_eq!(fs::read(root.join("README.md")).unwrap(), b"User notes");
        session.apply("edit.redo", &serde_json::json!({})).unwrap();
        assert_eq!(session.project().unwrap().board_target(), Some(target));
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

    /// On Windows a folder's new save lock is created hidden, as its leading dot hides it on Linux
    /// and macOS. A visible lock file left by an earlier build keeps its attributes and still
    /// serializes access: a held lock refuses an open, and the project opens once it's released.
    #[cfg(windows)]
    #[test]
    fn windows_save_locks_are_created_hidden() {
        use std::os::windows::fs::MetadataExt;
        let hidden = |path: &Path| fs::metadata(path).unwrap().file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0;
        let dir = tempfile::tempdir().unwrap();
        let documents = encode_documents(project()).unwrap();
        save(&dir.path().join("fresh"), None, &documents, &mut |_| Ok(())).unwrap();
        assert!(hidden(&dir.path().join(".fresh.pcbcraft-lock")));

        let root = dir.path().join("earlier");
        let earlier = dir.path().join(".earlier.pcbcraft-lock");
        fs::write(&earlier, b"").unwrap();
        save(&root, None, &documents, &mut |_| Ok(())).unwrap();
        assert!(!hidden(&earlier));
        let held = open_lock_file(&earlier).unwrap();
        held.try_lock().unwrap();
        assert!(open(&root).unwrap_err().contains("Could not lock"));
        held.unlock().unwrap();
        assert_eq!(encode_documents(open(&root).unwrap()).unwrap(), documents);
    }
}
