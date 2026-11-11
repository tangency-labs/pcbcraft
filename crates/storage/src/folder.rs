//! Recoverable publication of the independently diffable project documents.

use pcbcraft_engine::persistence::{
    Documents, ENTRY_POINT, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS, MAX_DOCUMENTS_BYTES, decode_documents, is_document_path,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

const TRANSACTION: &str = ".pcbcraft-save";
const JOURNAL_LIMIT: usize = 4 * 1024 * 1024;
/// Names that Windows reads as a device, not a file, when they come before the first dot of a
/// file or folder name in a usual (not `\\?\`) path, so `C:\…\con.r2.json` means the console
/// (#156). Document paths use only
/// lowercase ASCII letters, digits and hyphens, so these are the only device names a path can
/// hold. Checked with `GetFullPathNameW` on Windows 10: `com0`, `lpt0` and `com10` are ordinary
/// names there.
const WINDOWS_DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5",
    "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    changes: Vec<Change>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Change {
    path: String,
    before: Option<Payload>,
    after: Option<Payload>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    size: usize,
    sha256: String,
}

/// Recover and read under the writer lock. If opening the lock file is denied by filesystem
/// permissions, allow a read without recovery only while no committed journal exists.
pub fn load(root: &Path) -> Result<Documents, String> {
    regular_directory(root)?;
    let lock = lock(root, true)?;
    if lock.can_recover {
        recover(root, &mut |_| Ok(()))?;
    } else {
        require_no_journal(root)?;
    }
    let documents = read_documents(root, false)?;
    if !lock.can_recover {
        require_no_journal(root)?;
    }
    decode_documents(&documents)?;
    Ok(documents)
}

/// Read a disposable, validated preview without recovering data or holding a writer lock
/// during I/O. Probe existing locks and journals before/after reading; concurrent changes may
/// make the picture stale, but can never make it authoritative editor state or block a save.
pub(super) fn preview(root: &Path) -> Result<Documents, String> {
    regular_directory(root)?;
    let path = lock_path(root)?;
    let had_lock = probe_preview_lock(&path)?;
    require_no_journal(root)?;
    let documents = read_documents(root, false)?;
    require_no_journal(root)?;
    if probe_preview_lock(&path)? && !had_lock {
        return Err("Project changed while reading its preview.".to_owned());
    }
    Ok(documents)
}

/// Read an old split folder for conversion without creating locks or replaying its journal.
/// An existing lock stays shared across both reads; an introduced lock, pending recovery, mixed
/// layout or detected source change refuses conversion without modifying the source.
pub(super) fn load_legacy(root: &Path) -> Result<Documents, String> {
    regular_directory(root)?;
    let lock_path = lock_path(root)?;
    let file = match fs::symlink_metadata(&lock_path) {
        Ok(metadata) if metadata.is_file() => {
            let file = File::open(&lock_path).map_err(error)?;
            file.try_lock_shared().map_err(|e| format!("Could not lock conversion source: {e}"))?;
            Some(file)
        }
        Ok(_) => return Err("Save lock is not a regular file.".to_owned()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(error(e)),
    };
    let lock = FolderLock { file, can_recover: false };
    require_no_journal(root)?;
    if root.join(ENTRY_POINT).try_exists().map_err(error)? {
        return Err("Conversion requires the previous project.pc layout, without project.json.".to_owned());
    }
    let documents = read_documents_at(root, false, "project.pc")?;
    require_no_journal(root)?;
    if documents != read_documents_at(root, false, "project.pc")? || (lock.file.is_none() && probe_preview_lock(&lock_path)?) {
        return Err("Project changed while reading conversion source.".to_owned());
    }
    require_no_journal(root)?;
    pcbcraft_engine::persistence::convert_legacy_documents(documents)
}

/// Check an existing lock nonblockingly and immediately release it, creating no file. Keeping
/// a shared lock across a slow preview read would make an explicit Open/Save fail behind it.
fn probe_preview_lock(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            let file = File::open(path).map_err(error)?;
            file.try_lock_shared().map_err(|e| format!("Could not lock preview: {e}"))?;
            file.unlock().map_err(error)?;
            Ok(true)
        }
        Ok(_) => Err("Save lock is not a regular file.".to_owned()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(error(e)),
    }
}

/// Validate document schemas and platform filenames before creating any destination or lock.
/// Folder saves and explicit conversion share this preflight; portable ZIPs retain ZIP rules.
pub(super) fn preflight(documents: &Documents) -> Result<(), String> {
    decode_documents(documents)?;
    // The app saves through canonical `\\?\` paths, where Windows writes these names as ordinary
    // files, but Git for Windows then can't add them and programs that open them by their usual
    // path reach the device instead. Through a usual path the publish fails after the journal
    // commit, so recovery would fail on every open. Refuse before the lock, staging folder or
    // journal exist, leaving the destination as it was (#156).
    if cfg!(windows) {
        refuse_windows_device_names(documents)?;
    }
    Ok(())
}

/// Stage and flush every changed payload and its backup before atomically publishing a
/// recovery journal. Once published, recovery completes the transaction before any open.
/// A checkpoint failure simulates interruption; committed journals remain available for recovery.
pub fn save(
    root: &Path,
    expected: Option<&Documents>,
    documents: &Documents,
    checkpoint: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    preflight(documents)?;
    let _lock = lock(root, false)?;
    if !root.exists() {
        fs::create_dir(root).map_err(error)?;
        sync_dir(root.parent().ok_or("missing project parent")?)?;
    }
    regular_directory(root)?;
    recover(root, &mut |_| Ok(()))?;
    let baseline = read_documents(root, false)?;
    match expected {
        None if !baseline.is_empty() => return Err("That folder already contains a project. Choose a new or empty folder.".to_owned()),
        Some(expected) if expected != &baseline => {
            return Err("Project folder changed on disk. Use Save As to keep your edits in a separate folder.".to_owned());
        }
        _ => {}
    }
    if &baseline == documents {
        return Ok(());
    }
    crate::layout::preflight_ownership(root, &baseline, documents)?;
    let stage = tempfile::Builder::new().prefix(".pcbcraft-stage-").tempdir_in(root).map_err(error)?;
    let mut changes = Vec::new();
    let paths: BTreeSet<_> = baseline.keys().chain(documents.keys()).collect();
    for path in paths {
        let (before, after) = (baseline.get(path), documents.get(path));
        if before == after {
            continue;
        }
        let index = changes.len();
        let mut payload = |bytes: Option<&Vec<u8>>, suffix: &str| -> Result<Option<Payload>, String> {
            bytes
                .map(|bytes| {
                    write_synced(&stage.path().join(format!("{index}.{suffix}")), bytes)?;
                    checkpoint("payload")?;
                    Ok(Payload { size: bytes.len(), sha256: digest(bytes) })
                })
                .transpose()
        };
        changes.push(Change { path: path.clone(), before: payload(before, "old")?, after: payload(after, "new")? });
    }
    let journal = serde_json::to_vec(&Journal { version: 1, changes }).map_err(error)?;
    if journal.len() > JOURNAL_LIMIT {
        return Err("Save journal exceeds its size limit.".to_owned());
    }
    write_synced(&stage.path().join("journal.json"), &journal)?;
    sync_dir(stage.path())?;
    checkpoint("staged")?;
    // Recheck after staging; an external editor need not cooperate with our lock.
    if read_documents(root, false)? != baseline {
        return Err("Project folder changed while saving.".to_owned());
    }
    crate::layout::preflight_ownership(root, &baseline, documents)?;
    fs::rename(stage.path(), root.join(TRANSACTION)).map_err(error)?;
    sync_dir(root)?;
    checkpoint("journal")?;
    recover(root, checkpoint)
}

/// Replay a validated journal. Each path must still hold its old or new bytes, and every
/// staged payload is checked before the first mutation. Unrelated changes stop recovery.
fn recover(root: &Path, checkpoint: &mut impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    let transaction = root.join(TRANSACTION);
    match fs::symlink_metadata(&transaction) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(e)),
        Ok(metadata) if !metadata.is_dir() => return Err("Recovery journal path is not a directory.".to_owned()),
        Ok(_) => {}
    }
    let root_directory = crate::bound::Directory::open(root).map_err(error)?;
    let recovery = root_directory.child(TRANSACTION, false).map_err(error)?.ok_or("Missing recovery directory")?;
    let journal_bytes = recovery.read("journal.json", JOURNAL_LIMIT).map_err(error)?.ok_or("Missing recovery journal")?;
    let journal: Journal = serde_json::from_slice(&journal_bytes).map_err(|e| format!("Invalid save journal: {e}"))?;
    if journal.version != 1 || journal.changes.is_empty() || journal.changes.len() > MAX_DOCUMENTS * 2 {
        return Err("Unsupported or oversized save journal.".to_owned());
    }
    let mut seen = BTreeSet::new();
    let mut replacements = Vec::new();
    let mut total = 0;
    for (index, change) in journal.changes.iter().enumerate() {
        if !is_document_path(&change.path) || !seen.insert(&change.path) || (change.before.is_none() && change.after.is_none()) {
            return Err(format!("Invalid save journal path: {}", change.path));
        }

        let mut payload = |claimed: &Option<Payload>, suffix: &str| -> Result<Option<Vec<u8>>, String> {
            claimed
                .as_ref()
                .map(|claimed| {
                    if claimed.size > MAX_DOCUMENT_BYTES {
                        return Err("Recovery payload exceeds its size limit.".to_owned());
                    }
                    let bytes = recovery.read(&format!("{index}.{suffix}"), claimed.size).map_err(error)?.ok_or("Missing recovery payload")?;
                    total += bytes.len();
                    if total > MAX_DOCUMENTS_BYTES * 2 || bytes.len() != claimed.size || digest(&bytes) != claimed.sha256 {
                        return Err("Recovery payload size or digest mismatch.".to_owned());
                    }
                    Ok(bytes)
                })
                .transpose()
        };
        let before = payload(&change.before, "old")?;
        let after = payload(&change.after, "new")?;
        let current = bound_existing(&root_directory, &change.path)?;
        if current != before && current != after {
            return Err(format!("{} changed outside the interrupted save; recovery stopped.", change.path));
        }
        replacements.push((change.path.clone(), before, after));
    }
    // Validate the complete proposed project before completing an interrupted transaction.
    let mut result = read_documents(root, true)?;
    for (path, _, after) in &replacements {
        if let Some(bytes) = after {
            result.insert(path.clone(), bytes.clone());
        } else {
            result.remove(path);
        }
    }
    decode_documents(&result)?;
    for (index, (relative, before, after)) in replacements.into_iter().enumerate() {
        let bound = bind_document(&root_directory, &relative, true)?.ok_or("Missing publication parent")?;
        // This checkpoint exercises a directory being replaced after resolution. Publication
        // remains bound to the captured parent, and the final consistency check refuses adoption.
        checkpoint(&format!("resolved:{relative}"))?;
        let current = bound.parent.read(&bound.name, MAX_DOCUMENT_BYTES).map_err(error)?;
        if current != before && current != after {
            return Err(format!("{relative} changed during recovery; publication stopped."));
        }
        let removed_board = after.is_none() && relative.ends_with("/board.json");
        if current != after {
            if let Some(bytes) = after {
                let temporary = format!("publication-{index}.tmp");
                recovery.remove_file(&temporary).map_err(error)?;
                let mut file = recovery.create(&temporary).map_err(error)?;
                #[cfg(unix)]
                if let Some(previous) = bound.parent.open_file(&bound.name).map_err(error)? {
                    file.set_permissions(previous.metadata().map_err(error)?.permissions()).map_err(error)?;
                }
                file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(error)?;
                drop(file);
                recovery.rename(&temporary, &bound.parent, &bound.name).map_err(error)?;
            } else {
                bound.parent.remove_file(&bound.name).map_err(error)?;
            }
            bound.parent.sync().map_err(error)?;
        }
        if removed_board {
            bound.remove_empty_parent()?;
        }
        checkpoint(&format!("published:{relative}"))?;
    }
    // A moved/replaced parent must not turn a successful write to the captured directory into
    // a successful save of a different visible tree. Keep the journal for an explicit recovery.
    if read_documents(root, true)? != result {
        return Err("Project directory changed during publication; recovery stopped.".into());
    }
    // Close the recovery directory before renaming it on Windows, whose captured directory
    // handles intentionally deny deletion/renaming. The source and cleanup parents stay bound.
    drop(recovery);
    let cleanup = tempfile::Builder::new().prefix(".pcbcraft-stage-").tempdir_in(root).map_err(error)?;
    let cleanup_directory = crate::bound::Directory::open(cleanup.path()).map_err(error)?;
    root_directory.rename(TRANSACTION, &cleanup_directory, "completed").map_err(error)?;
    root_directory.sync().map_err(error)?;
    cleanup_directory.sync().map_err(error)?;
    checkpoint("completed")?;
    Ok(())
}

/// Fail with a message naming the first document whose path Windows reads as a device (#156),
/// suggesting a `.pc` archive, which stores documents as ZIP entries rather than files.
fn refuse_windows_device_names(documents: &Documents) -> Result<(), String> {
    match documents.keys().find_map(|path| windows_device_name(path).map(|device| (path, device))) {
        Some((path, device)) => Err(format!(
            "{path}: Windows reserves the name \"{device}\" for a device, so this project can't be saved as a folder here. \
             Save it as a .pc archive, or change that library or item ID."
        )),
        None => Ok(()),
    }
}

/// Return the first part of a `/`-separated document path, such as `con` in
/// `library/symbols/workshop/con.r2.json`, whose text before its first dot is one of
/// [`WINDOWS_DEVICE_NAMES`], ignoring ASCII case. `None` means every part is an ordinary name.
fn windows_device_name(relative: &str) -> Option<&str> {
    relative
        .split('/')
        .map(|part| part.split_once('.').map_or(part, |(stem, _)| stem))
        .find(|stem| WINDOWS_DEVICE_NAMES.iter().any(|device| stem.eq_ignore_ascii_case(device)))
}

/// Read the managed files only. Root-level README/Git files are left alone; files under
/// sheets, library and assets must belong to the supported schema, apart from regular OS metadata
/// files. Links and special files are refused even when named like OS metadata. A folder
/// containing only the old `project.pc` entry point is refused before reading managed data.
fn read_documents(root: &Path, recovering: bool) -> Result<Documents, String> {
    read_documents_at(root, recovering, ENTRY_POINT)
}

/// Read the selected split layout using the normal file/size limits and managed-path policy.
fn read_documents_at(root: &Path, recovering: bool, entry_point: &str) -> Result<Documents, String> {
    // An interrupted save may temporarily contain both removed and added definitions.
    let multiplier = if recovering { 2 } else { 1 };
    let mut entries = 0;
    regular_directory(root)?;
    if entry_point == ENTRY_POINT && !root.join(ENTRY_POINT).try_exists().map_err(error)? && root.join("project.pc").try_exists().map_err(error)? {
        return Err(pcbcraft_engine::persistence::UNSUPPORTED_PROJECT_LAYOUT.to_owned());
    }
    let mut documents = Documents::new();
    for name in [entry_point, "board.json"] {
        if let Some(bytes) = existing(&root.join(name))? {
            documents.insert(name.to_owned(), bytes);
        }
    }
    if let Some(entry) = documents.get(entry_point)
        && let Some(path) = pcbcraft_engine::persistence::project_board_path(entry)?
        && path != "board.json"
        && let Some(bytes) = bound_existing(&crate::bound::Directory::open(root).map_err(error)?, &path)?
    {
        documents.insert(path, bytes);
    }
    for name in ["sheets", "library", "assets"] {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => scan(root, &path, &mut documents, 0, multiplier, &mut entries)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(error(e)),
        }
    }
    if documents.len() > MAX_DOCUMENTS * multiplier || documents.values().map(Vec::len).sum::<usize>() > MAX_DOCUMENTS_BYTES * multiplier {
        return Err("Project folder exceeds its document or size limit.".to_owned());
    }
    Ok(documents)
}

/// Walk bounded owned subdirectories, leaving ordinary OS metadata untouched and refusing
/// every other unexpected file before reading it. Ignored files still count toward the scan bound.
fn scan(root: &Path, directory: &Path, documents: &mut Documents, depth: usize, multiplier: usize, entries: &mut usize) -> Result<(), String> {
    regular_directory(directory)?;
    if depth > 3 {
        return Err(format!("{}: unexpected project directory", directory.display()));
    }
    for entry in fs::read_dir(directory).map_err(error)? {
        *entries += 1;
        if *entries > MAX_DOCUMENTS * multiplier * 2 {
            return Err("Project has too many directory entries.".to_owned());
        }
        let path = entry.map_err(error)?.path();
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        if metadata.is_dir() {
            scan(root, &path, documents, depth + 1, multiplier, entries)?;
            continue;
        }
        let relative = path.strip_prefix(root).map_err(error)?.to_str().ok_or("Invalid document filename")?.replace(std::path::MAIN_SEPARATOR, "/");
        if metadata.is_file() && matches!(path.file_name().and_then(|name| name.to_str()), Some(".DS_Store" | "Thumbs.db" | "desktop.ini")) {
            continue;
        }
        if !is_document_path(&relative) || !metadata.is_file() {
            return Err(format!("{relative}: unexpected project file"));
        }
        documents.insert(relative, read_regular(&path, MAX_DOCUMENT_BYTES)?);
        if documents.len() > MAX_DOCUMENTS * multiplier || documents.values().map(Vec::len).sum::<usize>() > MAX_DOCUMENTS_BYTES * multiplier {
            return Err("Project folder exceeds its document or size limit.".to_owned());
        }
    }
    Ok(())
}

/// The persistent sibling lock name, shared by ordinary file jobs and read-only previews.
fn lock_path(root: &Path) -> Result<PathBuf, String> {
    let parent = root.parent().ok_or("Project folder has no parent")?;
    let name = root.file_name().ok_or("Project folder has no name")?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(".pcbcraft-lock");
    Ok(parent.join(lock_name))
}

/// Hold a nonblocking sibling lock across recovery, baseline checks and publication. Loading
/// may use an existing read-only lock in shared mode, or read unlocked if it cannot be created.
/// Either fallback requires a permission/read-only error and no journal; contention and unsafe
/// lock paths remain fatal, and a shared lock never authorizes recovery.
fn lock(root: &Path, allow_read_only: bool) -> Result<FolderLock, String> {
    let path = lock_path(root)?;
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && !metadata.is_file()
    {
        return Err("Save lock is not a regular file.".to_owned());
    }
    let file = match crate::open_lock_file(&path) {
        Ok(file) => file,
        Err(e) if allow_read_only && matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem) => {
            require_no_journal(root)?;
            return match File::open(&path) {
                Ok(file) => {
                    file.try_lock_shared().map_err(|e| format!("Could not lock project folder: {e}"))?;
                    Ok(FolderLock { file: Some(file), can_recover: false })
                }
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(FolderLock { file: None, can_recover: false }),
                Err(e) => Err(error(e)),
            };
        }
        Err(e) => return Err(error(e)),
    };
    file.try_lock().map_err(|e| format!("Could not lock project folder: {e}"))?;
    Ok(FolderLock { file: Some(file), can_recover: true })
}

/// Refuse an unlocked read if recovery is pending or its absence cannot be established.
/// Use symlink metadata so even a dangling journal link is treated as an obstruction.
fn require_no_journal(root: &Path) -> Result<(), String> {
    match fs::symlink_metadata(root.join(TRANSACTION)) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(e)),
        Ok(_) => Err("Project folder needs recovery, but its save lock is not writable. Open a writable copy to recover it.".to_owned()),
    }
}

/// Explicit unlock prevents a concurrently forked child retaining our advisory lock after
/// the save finishes; closing this process's descriptor alone need not release that lock.
struct FolderLock {
    file: Option<File>,
    can_recover: bool,
}

impl Drop for FolderLock {
    /// Release the lock on every success and error path, including interrupted staging.
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.unlock();
        }
    }
}

/// A validated document leaf and its captured parent, with its containing directory for
/// optional empty-board cleanup. Windows ancestor handles remain alive throughout publication.
struct BoundDocument {
    parent: crate::bound::Directory,
    name: String,
    containing: Option<(crate::bound::Directory, String)>,
}

impl BoundDocument {
    /// Release the leaf-parent handle before Windows removal; a symlink or nonempty directory
    /// is never removed. Repeating cleanup after an interrupted deletion remains harmless.
    fn remove_empty_parent(self) -> Result<(), String> {
        let Self { parent, containing, .. } = self;
        drop(parent);
        if let Some((directory, name)) = containing
            && directory.remove_dir(&name).is_ok()
        {
            directory.sync().map_err(error)?;
        }
        Ok(())
    }
}

/// Traverse one no-follow directory component at a time and retain its verified handle.
/// Missing parents represent absent documents unless publication explicitly creates them.
fn bind_document(root: &crate::bound::Directory, relative: &str, create: bool) -> Result<Option<BoundDocument>, String> {
    if !is_document_path(relative) {
        return Err("Unsafe project document path.".into());
    }
    let mut pieces = relative.split('/').peekable();
    let mut parent = root.clone();
    let mut containing = None;
    while let Some(piece) = pieces.next() {
        if pieces.peek().is_none() {
            return Ok(Some(BoundDocument { parent, name: piece.into(), containing }));
        }
        let Some(child) = parent.child(piece, create).map_err(|error| format!("{relative}: {error}"))? else { return Ok(None) };
        containing = Some((parent, piece.into()));
        parent = child;
    }
    Err("Missing project document leaf".into())
}

/// Read through the same no-follow parent binding used for writes, preserving absence.
fn bound_existing(root: &crate::bound::Directory, relative: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(bound) = bind_document(root, relative, false)? else { return Ok(None) };
    bound.parent.read(&bound.name, MAX_DOCUMENT_BYTES).map_err(|error| format!("{relative}: {error}"))
}

/// Refuse symlinked or non-directory roots and owned subdirectories.
fn regular_directory(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).map_err(error)?.is_dir() { Ok(()) } else { Err(format!("{}: not a regular directory", path.display())) }
}

/// A missing regular file is an absent document; other failures must remain visible.
fn existing(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(error(e)),
        Ok(_) => read_regular(path, MAX_DOCUMENT_BYTES).map(Some),
    }
}

/// Bound the actual bytes read even if a file grows after its metadata is inspected.
fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path).map_err(error)?.is_file() {
        return Err(format!("{}: not a regular file", path.display()));
    }
    let mut bytes = Vec::new();
    File::open(path).map_err(error)?.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() > limit {
        return Err(format!("{}: exceeds size limit", path.display()));
    }
    Ok(bytes)
}

/// Write a new staging file and flush it before making the journal visible.
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(error)?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(error)
}

/// Sync directory entry changes where the platform exposes directory fsync.
pub(super) fn sync_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path).and_then(|file| file.sync_all()).map_err(error)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Fingerprint recovery payloads independently of ZIP manifests.
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Preserve filesystem errors for the status bar without discarding the underlying cause.
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::{Session, library::sources, persistence::encode_documents};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// Documents for a project that pins one symbol from a loaded native library, placed through
    /// production commands, so both IDs reach the folder paths.
    fn pinning(library: &str, symbol: &str) -> Documents {
        encode_documents(pinned_project(library, symbol)).unwrap()
    }

    /// The project behind [`pinning`], for checks that need it rather than its folder documents.
    fn pinned_project(library: &str, symbol: &str) -> std::sync::Arc<pcbcraft_engine::Project> {
        let mut definition: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../libraries/starter/symbols/resistor.json"))).unwrap();
        definition["id"] = json!(symbol);
        let revision = u32::try_from(definition["revision"].as_u64().unwrap()).unwrap();
        let source = sources::Documents::from([
            (
                "library.json".to_owned(),
                json!({"format":"pcbcraft-library/1","id":library,"name":"Workshop","license":"MIT OR Apache-2.0"}).to_string(),
            ),
            // An ordinary file name on purpose: an item's ID comes from its JSON, not its file.
            ("symbols/part.json".to_owned(), definition.to_string()),
        ]);
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        session.execute(sources::LOAD, &json!({"source":"workshop","documents":source})).unwrap();
        let digest = sources::symbol_digest(&session.libraries().symbol(library, symbol, revision).unwrap()).unwrap();
        let place = json!({"library":library,"definition":symbol,"definition_revision":revision,"definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()});
        session.apply(pcbcraft_engine::PLACE_SYMBOL, &place).unwrap();
        session.snapshot().unwrap()
    }

    /// Every folder and file below `root`, with file bytes, to show what a save wrote.
    fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut found = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    found.insert(relative, None);
                    pending.push(path);
                } else {
                    found.insert(relative, Some(fs::read(&path).unwrap()));
                }
            }
        }
        found
    }

    /// Any part of a document path can name a device, judged by its text before the first dot.
    /// Longer names that only start with a device name stay ordinary (#156).
    #[test]
    fn device_names_are_found_in_any_part_of_a_document_path() {
        for (path, device) in [
            ("library/symbols/workshop/con.r2.json", "con"),
            ("library/footprints/aux/sot23.r1.json", "aux"),
            ("library/symbols/nul/prn.r1.json", "nul"),
            ("library/symbols/workshop/com1.r3.json", "com1"),
            ("library/symbols/lpt9/resistor.r2.json", "lpt9"),
            ("library/symbols/workshop/CON.r2.json", "CON"),
        ] {
            assert_eq!(windows_device_name(path), Some(device), "{path}");
        }
        for path in [
            ENTRY_POINT,
            "board.json",
            "sheets/d731c904-8549-4214-9fba-9e6487892c7a.json",
            "library/symbols/workshop/console.r2.json",
            "library/symbols/con-1/aux-jack.r2.json",
            "library/symbols/workshop/com10.r1.json",
            "library/symbols/workshop/com0.r1.json",
            "library/footprints/workshop/lpt0.r1.json",
        ] {
            assert_eq!(windows_device_name(path), None, "{path}");
        }
    }

    /// Conversion runs the same Windows preflight before exclusively creating its destination.
    #[cfg(windows)]
    #[test]
    fn windows_conversion_refuses_device_names_before_destination_creation() {
        let parent = tempfile::tempdir().unwrap();
        for (library, symbol) in [("workshop", "con"), ("workshop", "aux"), ("nul", "resistor")] {
            let root = parent.path().join("converted");
            let error = crate::conversion::publish(&root, pinning(library, symbol), pcbcraft_engine::persistence::Encoding::Json).unwrap_err();
            assert!(error.contains("Windows reserves the name"), "{error}");
            assert!(tree(parent.path()).is_empty());
        }
    }

    /// On Windows, a folder save that needs a device-named file or folder fails with a message
    /// naming it before anything is written. A new destination isn't created, and an existing
    /// project keeps its files, lock and journal state, and still opens (#156).
    #[cfg(windows)]
    #[test]
    fn windows_folder_saves_refuse_device_names_before_writing() {
        // The app's Save passes a canonical `\\?\` root (`project_files::native::normalize`), where
        // Windows would write the file; other callers may pass a usual root. Both are refused.
        for canonical in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let base = if canonical { fs::canonicalize(parent.path()).unwrap() } else { parent.path().to_path_buf() };
            let root = base.join("board");
            for (library, symbol) in [("workshop", "con"), ("workshop", "aux"), ("nul", "resistor")] {
                let error = save(&root, None, &pinning(library, symbol), &mut |_| Ok(())).unwrap_err();
                assert!(error.contains("Windows reserves the name") && error.contains(".pc"), "{error}");
                assert!(tree(parent.path()).is_empty(), "{library}/{symbol} wrote {:?}", tree(parent.path()));
            }
            let ordinary = pinning("workshop", "resistor");
            save(&root, None, &ordinary, &mut |_| Ok(())).unwrap();
            let before = tree(parent.path());
            let error = save(&root, Some(&ordinary), &pinning("workshop", "con"), &mut |_| Ok(())).unwrap_err();
            assert!(error.starts_with("library/symbols/workshop/con.r"), "{error}");
            assert_eq!(tree(parent.path()), before);
            assert_eq!(load(&root).unwrap(), ordinary);
        }
    }

    /// Elsewhere these are ordinary names: folders save and reopen with the same documents, and
    /// a repeated save leaves every directory, file byte, lock and journal state unchanged.
    #[cfg(not(windows))]
    #[test]
    fn other_platforms_save_device_names_as_ordinary_files() {
        for (library, symbol) in [("workshop", "con"), ("workshop", "aux"), ("nul", "resistor"), ("con", "aux")] {
            let parent = tempfile::tempdir().unwrap();
            let root = parent.path().join("board");
            let documents = pinning(library, symbol);
            save(&root, None, &documents, &mut |_| Ok(())).unwrap();
            assert_eq!(load(&root).unwrap(), documents);
            let before = tree(parent.path());
            save(&root, Some(&documents), &documents, &mut |_| Ok(())).unwrap();
            assert_eq!(tree(parent.path()), before);
        }
    }

    /// Archives retain device-named library and symbol IDs on every platform because document
    /// paths are ZIP entries, so the alternative suggested by the Windows refusal reopens intact.
    #[test]
    fn archives_preserve_device_named_documents() {
        for (library, symbol) in [("workshop", "con"), ("workshop", "aux"), ("nul", "resistor"), ("con", "aux")] {
            let parent = tempfile::tempdir().unwrap();
            let project = pinned_project(library, symbol);
            let archive = parent.path().join("board.pc");
            fs::write(&archive, pcbcraft_engine::persistence::encode_packed(project.clone()).unwrap()).unwrap();
            assert_eq!(encode_documents(crate::open(&archive).unwrap()).unwrap(), encode_documents(project).unwrap());
        }
    }
}
