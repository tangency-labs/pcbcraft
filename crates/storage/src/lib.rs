//! Native filesystem access, shared by desktop file workers and the headless CLI.

use pcbcraft_engine::{
    Project,
    persistence::{ENTRY_POINT, Encoding, decode_documents, decode_packed, read_bytes},
};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};

mod bound;
pub mod conversion;
mod folder;
pub mod layout;
pub mod library;
pub use folder::{load, save};

/// Return the plain `C:\…` spelling of a Windows `\\?\C:\…` path when Windows reads that spelling
/// as the same path, and `path` unchanged otherwise (#154). `std::fs::canonicalize` produces the
/// `\\?\` form, which people don't recognise. It stays where the plain spelling would name
/// something else: names ending in a dot or space, very long paths, and, on Windows 10 and
/// earlier, device names such as `CON.pc`. Paths on other platforms never carry the prefix
/// and pass through unchanged.
pub fn plain_path_if_equivalent(path: PathBuf) -> PathBuf {
    let plain = dunce::simplified(&path);
    // dunce conservatively retains device-name prefixes even on Windows versions where
    // the OS treats them as ordinary filenames. Let the OS parser decide for drive paths.
    let drive_path = path.to_str().and_then(|text| text.strip_prefix(r"\\?\")).filter(|text| {
        let bytes = text.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    });
    let plain = if plain == path { drive_path.map(Path::new).unwrap_or(plain) } else { plain };
    // dunce looks only at the text before a name's last dot when it checks device names, so
    // a multi-dot name such as `CON.board.pc` can pass it. Windows 10's parser (GetFullPathNameW, behind `std::path::absolute`)
    // reads that name as `\\.\CON`; Windows 11's reads it as a file. Asking the parser covers
    // both, so the plain form is used only if the parser returns it as is.
    if plain != path && std::path::absolute(plain).is_ok_and(|parsed| parsed == plain) {
        return plain.to_path_buf();
    }
    path
}

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

/// Open a folder, its project.json entry point, or a packed archive.
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
    // Canonical so the desktop and CLI agree on one path, then plain where Windows reads it the
    // same, so errors name the path people chose rather than its `\\?\` form (#154).
    let path = plain_path_if_equivalent(fs::canonicalize(parent).map_err(|e| e.to_string())?.join(name));
    let parent = path.parent().ok_or("Project path has no parent folder")?.to_path_buf();
    // Recent history may still point at the entry removed by a completed migration.
    if name == "project.pc"
        && matches!(fs::symlink_metadata(&path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        && parent.join(ENTRY_POINT).try_exists().map_err(|e| e.to_string())?
    {
        return decode_documents(&folder(&parent)?);
    }
    if (name == ENTRY_POINT || name == "project.pc") && parent.join(".pcbcraft-save").try_exists().map_err(|e| e.to_string())? {
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
        if name == ENTRY_POINT || name == "project.pc" {
            return decode_documents(&folder(&parent)?);
        }
        return Err(pcbcraft_engine::persistence::UNSUPPORTED_PROJECT_LAYOUT.to_owned());
    }
    decode_packed(&bytes).map(|packed| packed.project)
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
        let board_path = session.project().unwrap().board_document_path().unwrap();
        assert!(root.join(&board_path).exists());
        assert_eq!(open(&root).unwrap().board_target(), Some(target));
        assert_eq!(preview(&root.join(ENTRY_POINT)).unwrap().board_target(), Some(target));
        session.apply("edit.undo", &serde_json::json!({})).unwrap();
        let undone = encode_documents(session.snapshot().unwrap()).unwrap();
        save(&root, Some(&created), &undone, &mut |_| Ok(())).unwrap();
        assert!(!root.join(&board_path).exists());
        assert!(open(&root).unwrap().board_target().is_none());
        assert_eq!(fs::read(root.join("README.md")).unwrap(), b"User notes");
        session.apply("edit.redo", &serde_json::json!({})).unwrap();
        assert_eq!(session.project().unwrap().board_target(), Some(target));
        let prepared = crate::layout::prepare(session.snapshot().unwrap(), Some(&root), Some(&undone)).unwrap();
        assert_eq!(prepared.board_document_path().as_deref(), Some(board_path.as_str()));
        let redone = encode_documents(prepared).unwrap();
        save(&root, Some(&undone), &redone, &mut |_| Ok(())).unwrap();
        assert_eq!(load(&root).unwrap(), created);
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
        let archive = dir.path().join("project.pc");
        fs::write(&archive, encode_packed(project()).unwrap()).unwrap();
        let before = tree(dir.path());
        for path in [&root, &root.join(ENTRY_POINT), &archive] {
            assert_eq!(preview(path).unwrap().symbols().len(), 1);
        }
        assert_eq!(tree(dir.path()), before);
    }

    /// Schematic-only previous projects preview without writes, reject mixed entry points,
    /// and detect external changes before a first-save migration can publish anything.
    #[test]
    fn previous_schematic_project_preview_and_migration_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("old");
        fs::create_dir(&root).unwrap();
        let before = encode_documents(project()).unwrap();
        for (path, bytes) in &before {
            let path = root.join(if path == ENTRY_POINT { "project.pc" } else { path });
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let original = tree(dir.path());
        assert_eq!(preview(&root).unwrap().symbols().len(), 1);
        assert_eq!(tree(dir.path()), original);
        let loaded = open(&root).unwrap();
        let prepared = layout::prepare(loaded, Some(&root), Some(&before)).unwrap();
        let after = encode_documents(prepared).unwrap();
        fs::copy(root.join("project.pc"), root.join(ENTRY_POINT)).unwrap();
        assert!(preview(&root).is_err());
        assert!(open(&root).is_err());
        fs::remove_file(root.join(ENTRY_POINT)).unwrap();
        let mut changed = before[ENTRY_POINT].clone();
        changed.push(b' ');
        fs::write(root.join("project.pc"), changed).unwrap();
        let external = tree(&root);
        assert!(save(&root, Some(&before), &after, &mut |_| Ok(())).is_err());
        assert_eq!(tree(&root), external);
        fs::write(root.join("project.pc"), &before[ENTRY_POINT]).unwrap();
        save(&root, Some(&before), &after, &mut |_| Ok(())).unwrap();
        assert!(!root.join("project.pc").exists());
        assert_eq!(preview(&root).unwrap().symbols().len(), 1);
        assert_eq!(load(&root).unwrap(), after);
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
    /// Opening a folder through the shared reader, as `pcbcraft-cli mcp --project` does, names a
    /// failing document by the path people use, not its Windows `\\?\` form (#154). The error
    /// still names the file, so the person can find it.
    #[test]
    fn open_errors_name_documents_without_the_windows_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("oversized");
        save(&root, None, &encode_documents(project()).unwrap(), &mut |_| Ok(())).unwrap();
        fs::write(root.join("board.json"), vec![b' '; 17 * 1024 * 1024]).unwrap();
        let error = open(&root).unwrap_err();
        assert!(error.ends_with("board.json: exceeds size limit"), "{error}");
        assert!(!error.contains(r"\\?\"), "{error}");
    }
}
