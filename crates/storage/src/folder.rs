//! Recoverable publication of the independently diffable project documents.

use pcbcraft_engine::persistence::{
    Documents, ENTRY_POINT, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS, MAX_DOCUMENTS_BYTES, decode_documents, is_document_path,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const TRANSACTION: &str = ".pcbcraft-save";
const JOURNAL_LIMIT: usize = 4 * 1024 * 1024;

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

/// Load only after recovering a published journal under the same lock used by writers.
pub fn load(root: &Path) -> Result<Documents, String> {
    regular_directory(root)?;
    let _lock = lock(root)?;
    recover(root, &mut |_| Ok(()))?;
    let documents = read_documents(root, false)?;
    decode_documents(&documents)?;
    Ok(documents)
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
    decode_documents(documents)?;
    let _lock = lock(root)?;
    if !root.exists() {
        fs::create_dir(root).map_err(error)?;
        sync_dir(root.parent().ok_or("missing project parent")?)?;
    }
    regular_directory(root)?;
    recover(root, &mut |_| Ok(()))?;
    let baseline = read_documents(root, false)?;
    if expected.map_or(!baseline.is_empty(), |expected| expected != &baseline) {
        return Err("Project folder changed on disk. Use Save As to keep your edits in a separate folder.".to_owned());
    }
    if &baseline == documents {
        return Ok(());
    }
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
    let journal_bytes = read_regular(&transaction.join("journal.json"), JOURNAL_LIMIT)?;
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
        let path = safe_path(root, &change.path, false)?;
        let mut payload = |claimed: &Option<Payload>, suffix: &str| -> Result<Option<Vec<u8>>, String> {
            claimed
                .as_ref()
                .map(|claimed| {
                    if claimed.size > MAX_DOCUMENT_BYTES {
                        return Err("Recovery payload exceeds its size limit.".to_owned());
                    }
                    let bytes = read_regular(&transaction.join(format!("{index}.{suffix}")), claimed.size)?;
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
        let current = existing(&path)?;
        if current != before && current != after {
            return Err(format!("{} changed outside the interrupted save; recovery stopped.", change.path));
        }
        replacements.push((change.path.clone(), after));
    }
    // Validate the complete proposed project before completing an interrupted transaction.
    let mut result = read_documents(root, true)?;
    for (path, after) in &replacements {
        if let Some(bytes) = after {
            result.insert(path.clone(), bytes.clone());
        } else {
            result.remove(path);
        }
    }
    decode_documents(&result)?;
    for (relative, after) in replacements {
        let path = safe_path(root, &relative, true)?;
        if existing(&path)? != after {
            if let Some(bytes) = after {
                let parent = path.parent().ok_or("missing document parent")?;
                let mut file = tempfile::NamedTempFile::new_in(&transaction).map_err(error)?;
                #[cfg(unix)]
                if let Ok(metadata) = fs::symlink_metadata(&path) {
                    file.as_file().set_permissions(metadata.permissions()).map_err(error)?;
                }
                file.write_all(&bytes).and_then(|()| file.as_file().sync_all()).map_err(error)?;
                file.persist(&path).map_err(error)?;
                sync_dir(parent)?;
            } else {
                fs::remove_file(&path).map_err(error)?;
                sync_dir(path.parent().ok_or("missing document parent")?)?;
            }
        }
        checkpoint(&format!("published:{relative}"))?;
    }
    // Rename the completed transaction out of the recovery path before deleting it. A crash
    // during cleanup must not leave a journal referring to already deleted staging files.
    let cleanup = tempfile::Builder::new().prefix(".pcbcraft-stage-").tempdir_in(root).map_err(error)?;
    let retired = cleanup.path().join("completed");
    fs::rename(&transaction, retired).map_err(error)?;
    sync_dir(root)?;
    checkpoint("completed")?;
    Ok(())
}

/// Read the managed files only. Root-level README/Git files are left alone; files under
/// sheets, library and assets must belong to the supported schema, with no links or special files.
fn read_documents(root: &Path, recovering: bool) -> Result<Documents, String> {
    // An interrupted save may temporarily contain both removed and added definitions.
    let multiplier = if recovering { 2 } else { 1 };
    let mut entries = 0;
    regular_directory(root)?;
    let mut documents = Documents::new();
    for name in [ENTRY_POINT, "board.json"] {
        if let Some(bytes) = existing(&root.join(name))? {
            documents.insert(name.to_owned(), bytes);
        }
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

/// Walk only bounded owned subdirectories and refuse any unexpected file before reading it.
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

/// Hold a nonblocking sibling lock across recovery, baseline checks and publication.
fn lock(root: &Path) -> Result<FolderLock, String> {
    let parent = root.parent().ok_or("Project folder has no parent")?;
    let name = root.file_name().ok_or("Project folder has no name")?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(".pcbcraft-lock");
    let path = parent.join(lock_name);
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && !metadata.is_file()
    {
        return Err("Save lock is not a regular file.".to_owned());
    }
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(error)?;
    file.try_lock().map_err(|e| format!("Could not lock project folder: {e}"))?;
    Ok(FolderLock(file))
}

/// Explicit unlock prevents a concurrently forked child retaining our advisory lock after
/// the save finishes; closing this process's descriptor alone need not release that lock.
struct FolderLock(File);

impl Drop for FolderLock {
    /// Release the lock on every success and error path, including interrupted staging.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Resolve an already validated relative path without traversing symlinked directories.
fn safe_path(root: &Path, relative: &str, create: bool) -> Result<PathBuf, String> {
    if !is_document_path(relative) {
        return Err("Unsafe project document path.".to_owned());
    }
    regular_directory(root)?;
    let mut current = root.to_path_buf();
    let pieces: Vec<_> = relative.split('/').collect();
    for piece in &pieces[..pieces.len() - 1] {
        let parent = current.clone();
        current.push(piece);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if !metadata.is_dir() => return Err(format!("{}: not a regular directory", current.display())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if create {
                    fs::create_dir(&current).map_err(error)?;
                    sync_dir(&parent)?;
                }
            }
            Err(e) => return Err(error(e)),
        }
    }
    Ok(root.join(relative))
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
fn sync_dir(path: &Path) -> Result<(), String> {
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
