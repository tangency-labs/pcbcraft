//! Per-user crash recovery for unsaved work.
//!
//! A host can give each editing session an entry in a caller-selected recovery folder:
//! `<id>.pc` (a versioned recovery envelope around the portable archive), `<id>.journal`
//! (successfully recorded changes since that snapshot) and `<id>.lock`, which the writer holds
//! locked while it runs. Recovery reads the snapshot and replays the journal in order.
//! The default folder is in per-user storage; neither an override nor a caller's folder is
//! checked for separation from projects. Hosts should keep recovery outside their project,
//! save transaction and save lock.
//!
//! The journal records the documents each edit produced, not just its command, because commands
//! that create objects draw fresh random identities and would not reproduce the same project
//! when run again. Each record still names its command, version and parameters, and carries the
//! hash of the documents before and after, so replay stops at the first record that does not
//! continue the state before it.
//!
//! The folder rules (`default_dir`, `data_dir`) and the `<id>` entry layout with its lock follow
//! SolidCraft `crates/engine/src/recovery.rs` at 73bdb6c (MIT OR Apache-2.0).
//!
//! A journal file is the line `pcbcraft-recovery/2`, then frames of a little-endian `u32` payload
//! length, the payload and the payload's SHA-256. The first frame is a JSON header naming the
//! snapshot it continues; every later frame is one edit. A frame cut short by a crash fails its
//! length or hash check and ends the replay before it.

use crate::open_lock_file;
use pcbcraft_engine::{
    Project,
    edits::{CommittedEdit, EditObserver},
    persistence::{Documents, MAX_ARCHIVE_BYTES, MAX_DOCUMENTS_BYTES, decode_documents, decode_packed, encode_documents, encode_packed_documents},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufReader, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

/// First line of every journal. A reader refuses any other version rather than guess at it.
pub const FORMAT: &str = "pcbcraft-recovery/2";
const MAGIC: &[u8] = b"pcbcraft-recovery/2\n";
/// Room for the version and framed snapshot header, in addition to the bounded project archive.
const SNAPSHOT_OVERHEAD_BYTES: usize = 4096;
const REMOVED_ENTRY: &str = "The recovery journal was removed; start a new recovery entry to record further edits.";
/// Overrides the recovery folder, for tests, portable installs and headless runs.
pub const DIR_VARIABLE: &str = "PCBCRAFT_RECOVERY_DIR";
/// Version recorded for every command until the catalog versions commands individually.
const COMMAND_VERSION: u32 = 1;
/// A frame holds at most a whole project's documents plus the command and its parameters.
const MAX_FRAME_BYTES: usize = MAX_DOCUMENTS_BYTES + 16 * 1024 * 1024;
const MAX_ID_LEN: usize = 64;
/// A journal is replaced by a new snapshot once it holds more than this many times the bytes of
/// its snapshot, or [`REBASE_FLOOR_BYTES`] if that is larger. While the project stays about the
/// same size, rewriting snapshots adds at most about half again to the bytes the journal writes;
/// one large edit can make the new snapshot as large as the old one and the journal together.
/// While new snapshots succeed, recovery reads at most the snapshot, records up to the larger of
/// twice its size and the floor, and the one record that crossed the limit. While they keep
/// failing with the old journal still open and record writes succeeding, recorded changes keep
/// appending, so the journal can grow until a snapshot succeeds, an append fails or the session
/// ends. With no journal open (after a failed append, a failed journal publish or a root that
/// could not be snapshotted), edits are not recorded until a snapshot succeeds. A detected removed
/// entry stays disabled until a new recorder is started. Recovery returns the last recorded state.
const REBASE_RATIO: u64 = 2;
/// The least journal size that starts a new snapshot, so a small project is not snapshotted
/// and flushed every few edits.
const REBASE_FLOOR_BYTES: u64 = 1024 * 1024;

/// The recovery folder: `PCBCRAFT_RECOVERY_DIR`, else `recovery` in the per-user data folder
/// ([`data_dir`]). Tests of this crate use a folder of their own under the temporary folder.
/// Returns `None` when the platform names no per-user folder, as in the browser.
pub fn default_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("pcbcraft-test-recovery-{}", std::process::id())));
    }
    if let Some(dir) = std::env::var_os(DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    Some(data_dir()?.join("recovery"))
}

/// PCBCraft's per-user data folder: `%LOCALAPPDATA%\PCBcraft`,
/// `~/Library/Application Support/PCBcraft`, or `$XDG_DATA_HOME/pcbcraft`
/// (`~/.local/share/pcbcraft`). Returns `None` when the variables it needs are unset or empty.
pub fn data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        Some(env("LOCALAPPDATA")?.join("PCBcraft"))
    } else if cfg!(target_os = "macos") {
        Some(env("HOME")?.join("Library/Application Support/PCBcraft"))
    } else {
        Some(env("XDG_DATA_HOME").or_else(|| env("HOME").map(|home| home.join(".local/share")))?.join("pcbcraft"))
    }
}

/// A fresh entry id for one editing session of `project`: its UUID followed by this process and
/// a time stamp, so two sessions of one project, or two untitled projects, never share an entry.
pub fn new_entry_id(project: &Project) -> String {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| u64::try_from(time.as_nanos()).unwrap_or(u64::MAX));
    // Strictly increasing within the process even when the clock stalls or steps back.
    let mut last = LAST.load(Ordering::Relaxed);
    let stamp = loop {
        let stamp = last.saturating_add(1).max(now);
        match LAST.compare_exchange_weak(last, stamp, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break stamp,
            Err(seen) => last = seen,
        }
    };
    format!("{}-{:x}-{stamp:x}", project.id(), std::process::id())
}

/// Entry ids are file stems: lower-case ASCII letters, digits and hyphens only, so no id can name
/// a path outside the recovery folder, and two ids never name the same files on a file system
/// that ignores case. Windows device names (`con`, `nul`, `com1`, ...) are refused, since Windows
/// opens the device for `nul.pc` and the like.
fn valid_id(id: &str) -> bool {
    const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];
    let device = DEVICES.contains(&id) || (id.len() == 4 && (id.starts_with("com") || id.starts_with("lpt")) && id.as_bytes()[3].is_ascii_digit());
    !id.is_empty() && id.len() <= MAX_ID_LEN && !device && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The entry's file with extension `ext`.
fn entry_file(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}"))
}

/// Order-sensitive fingerprint of a complete document set, independent of any archive layout.
pub fn documents_sha256(documents: &Documents) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in documents {
        hasher.update(u64::try_from(path.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// The snapshot's and journal's first frame. Unknown fields are ignored within format 2.
#[derive(Deserialize, Serialize)]
struct Header {
    /// The snapshot publication, distinct even when its documents have not changed.
    generation: String,
    /// Fingerprint of the snapshot documents the journal continues.
    base_sha256: String,
}

/// One committed edit. Unknown fields are ignored so later writers can add fields within
/// format 2; anything a reader must understand needs a new format line.
#[derive(Deserialize, Serialize)]
struct Record {
    command: String,
    version: u32,
    params: Value,
    revision: u64,
    before_sha256: String,
    after_sha256: String,
    /// Documents added or changed by the edit, in the order of their bytes after the metadata.
    written: Vec<Written>,
    removed: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct Written {
    path: String,
    size: usize,
}

/// Wrap `payload` in a length and hash so a torn or damaged frame is detected on reading.
fn frame(payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err("Recovery record exceeds its size limit.".to_owned());
    }
    let length = u32::try_from(payload.len()).map_err(|_| "Recovery record exceeds its size limit.".to_owned())?;
    let mut bytes = Vec::with_capacity(payload.len() + 36);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&Sha256::digest(payload));
    Ok(bytes)
}

/// Read the next frame. `Ok(None)` is a clean end of file; `Err` is a short, oversized or
/// damaged frame, which is what a crash during an append leaves behind.
fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<u8>>, String> {
    let mut length = [0; 4];
    let mut filled = 0;
    while filled < length.len() {
        match reader.read(&mut length[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err("record is cut short".to_owned()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let length = usize::try_from(u32::from_le_bytes(length)).map_err(|_| "record exceeds its size limit".to_owned())?;
    if length > MAX_FRAME_BYTES {
        return Err("record exceeds its size limit".to_owned());
    }
    let mut payload = vec![0; length];
    let mut hash = [0; 32];
    reader.read_exact(&mut payload).and_then(|()| reader.read_exact(&mut hash)).map_err(|_| "record is cut short".to_owned())?;
    if Sha256::digest(&payload).as_slice() != hash {
        return Err("record is damaged".to_owned());
    }
    Ok(Some(payload))
}

/// Encode an edit as metadata followed by the bytes of each written document. Documents equal
/// in `before` and `after` are left out.
fn encode_record(edit: &CommittedEdit, before: &Documents, before_sha256: &str, after: &Documents, after_sha256: &str) -> Result<Vec<u8>, String> {
    let changed: Vec<(&String, &Vec<u8>)> = after.iter().filter(|(path, bytes)| before.get(*path) != Some(*bytes)).collect();
    let mut params = edit.params.clone();
    // Review authority is session-local and already spent. Recovery replays documents, not commands.
    let spent_token = match edit.command.as_str() {
        pcbcraft_engine::SET_BOARD_SETUP => Some("plan_token"),
        pcbcraft_engine::annotation::APPLY => Some("token"),
        _ => None,
    };
    if let (Some(field), Some(params)) = (spent_token, params.as_object_mut()) {
        params.remove(field);
    }
    let record = Record {
        command: edit.command.clone(),
        version: COMMAND_VERSION,
        params,
        revision: edit.revision,
        before_sha256: before_sha256.to_owned(),
        after_sha256: after_sha256.to_owned(),
        written: changed.iter().map(|(path, bytes)| Written { path: (*path).clone(), size: bytes.len() }).collect(),
        removed: before.keys().filter(|path| !after.contains_key(*path)).cloned().collect(),
    };
    let meta = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    let meta_length = u32::try_from(meta.len()).map_err(|_| "Recovery record exceeds its size limit.".to_owned())?;
    let mut payload = Vec::with_capacity(4 + meta.len() + changed.iter().map(|(_, bytes)| bytes.len()).sum::<usize>());
    payload.extend_from_slice(&meta_length.to_le_bytes());
    payload.extend_from_slice(&meta);
    for (_, bytes) in changed {
        payload.extend_from_slice(bytes);
    }
    Ok(payload)
}

/// Paths and bytes of the documents a record writes, in record order.
type WrittenDocuments = Vec<(String, Vec<u8>)>;

/// Split a record payload into its metadata and the written documents' bytes.
fn decode_record(payload: &[u8]) -> Result<(Record, WrittenDocuments), String> {
    let (length, rest) = payload.split_first_chunk::<4>().ok_or("record has no metadata")?;
    let length = usize::try_from(u32::from_le_bytes(*length)).map_err(|_| "record metadata exceeds its size limit")?;
    let (meta, mut blobs) = rest.split_at_checked(length).ok_or("record metadata is cut short")?;
    let record: Record = serde_json::from_slice(meta).map_err(|e| format!("record metadata: {e}"))?;
    let mut written = Vec::with_capacity(record.written.len());
    for document in &record.written {
        if !pcbcraft_engine::persistence::is_document_path(&document.path) {
            return Err(format!("{}: not a project document", document.path));
        }
        let (bytes, rest) = blobs.split_at_checked(document.size).ok_or("record documents are cut short")?;
        written.push((document.path.clone(), bytes.to_vec()));
        blobs = rest;
    }
    if !blobs.is_empty() {
        return Err("record has bytes no document claims".to_owned());
    }
    if let Some(path) = record.removed.iter().find(|path| !pcbcraft_engine::persistence::is_document_path(path)) {
        return Err(format!("{path}: not a project document"));
    }
    Ok((record, written))
}

/// Write `bytes` to `path` through a flushed sibling temporary file and an atomic rename, then
/// flush the folder where the platform allows it, so a reader sees the old or the new file.
///
/// A failure says whether the rename had already happened: then `path` holds `bytes`, though
/// the folder flush that makes the rename durable did not complete.
fn publish(dir: &Path, path: &Path, bytes: &[u8]) -> Result<File, PublishError> {
    let before = |error: String| PublishError { replaced: false, message: error };
    let mut staged = tempfile::Builder::new().prefix(".recovery-").tempfile_in(dir).map_err(|e| before(e.to_string()))?;
    staged.write_all(bytes).map_err(|e| before(e.to_string()))?;
    publish_staged(dir, path, staged)
}

/// Flush and rename a staged file. Before the rename the destination is unchanged; after it,
/// even a failed folder flush means the destination holds the staged bytes.
fn publish_staged(dir: &Path, path: &Path, staged: tempfile::NamedTempFile) -> Result<File, PublishError> {
    let before = |error: String| PublishError { replaced: false, message: error };
    #[cfg(test)]
    if faults::fails(dir, faults::Fault::Publish) {
        return Err(before("The recovery file could not be staged.".to_owned()));
    }
    staged.as_file().sync_all().map_err(|e| before(e.to_string()))?;
    let file = staged.persist(path).map_err(|e| before(e.error.to_string()))?;
    sync_after_rename(dir).map_err(|message| PublishError { replaced: true, message })?;
    Ok(file)
}

/// Why [`publish`] failed, and whether the new file was already in place.
#[derive(Debug)]
struct PublishError {
    replaced: bool,
    message: String,
}

impl From<PublishError> for String {
    fn from(error: PublishError) -> Self {
        error.message
    }
}

/// Failures that tests inject into one recovery folder. Each switch is keyed by the folder, so
/// tests running in parallel, and the recorder's worker thread, see only their own.
#[cfg(test)]
mod faults {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    /// What a switch makes fail.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Fault {
        /// A publish, before its rename.
        Publish,
        /// The folder flush after a rename.
        Flush,
        /// A record write, after half its frame.
        Tear,
    }

    /// Armed switches: the folder, the fault, and how many such calls still pass before one fails.
    static ARMED: Mutex<Vec<(PathBuf, Fault, usize)>> = Mutex::new(Vec::new());

    /// Make the `nth` call of kind `fault` in `dir` fail, counting from 1, once.
    pub fn arm(dir: &Path, fault: Fault, nth: usize) {
        assert!(nth > 0, "calls are counted from 1");
        ARMED.lock().unwrap().push((dir.to_path_buf(), fault, nth));
    }

    /// Count one call of kind `fault` in `dir`; true if it is the one to fail.
    pub fn fails(dir: &Path, fault: Fault) -> bool {
        let mut armed = ARMED.lock().unwrap();
        let Some(index) = armed.iter().position(|(folder, kind, _)| folder == dir && *kind == fault) else {
            return false;
        };
        armed[index].2 -= 1;
        if armed[index].2 > 0 {
            return false;
        }
        armed.swap_remove(index);
        true
    }
}

/// Flush `dir` after a rename into it.
fn sync_after_rename(dir: &Path) -> Result<(), String> {
    #[cfg(test)]
    if faults::fails(dir, faults::Fault::Flush) {
        return Err("The recovery folder could not be flushed.".to_owned());
    }
    crate::folder::sync_dir(dir)
}

/// Append one framed record to the journal in `dir` and flush it to disk.
#[cfg_attr(not(test), allow(unused_variables))]
fn write_record(dir: &Path, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    if faults::fails(dir, faults::Fault::Tear) {
        file.write_all(&bytes[..bytes.len() / 2])?;
        file.sync_data()?;
        return Err(std::io::Error::other("The record write was cut short."));
    }
    file.write_all(bytes).and_then(|()| file.sync_data())
}

/// Stage a private recovery snapshot: its version and framed header, then the ordinary packed
/// project. The staging name's 24 random characters identify this publication, not its content.
/// Nothing at the entry's destination changes until the caller publishes the staged file.
fn stage_snapshot(dir: &Path, base: &Documents) -> Result<(tempfile::NamedTempFile, Header, u64), String> {
    let packed = encode_packed_documents(base.clone())?;
    let mut staged = tempfile::Builder::new().prefix(".recovery-").rand_bytes(24).tempfile_in(dir).map_err(|e| e.to_string())?;
    let generation = staged.path().file_name().ok_or("The recovery staging file has no name.")?.to_string_lossy().into_owned();
    let header = Header { generation, base_sha256: documents_sha256(base) };
    let metadata = frame(&serde_json::to_vec(&header).map_err(|e| e.to_string())?)?;
    staged.write_all(MAGIC).and_then(|()| staged.write_all(&metadata)).and_then(|()| staged.write_all(&packed)).map_err(|e| e.to_string())?;
    let size = MAGIC.len().saturating_add(metadata.len()).saturating_add(packed.len());
    Ok((staged, header, u64::try_from(size).unwrap_or(u64::MAX)))
}

/// Publish `<id>.journal` holding only `header`, which names the snapshot's generation and
/// fingerprint, returning the open file and its size in bytes.
fn publish_journal(dir: &Path, id: &str, header: &Header) -> Result<(File, u64), PublishError> {
    let before = |message: String| PublishError { replaced: false, message };
    let header = serde_json::to_vec(header).map_err(|e| before(e.to_string()))?;
    let mut journal = MAGIC.to_vec();
    journal.extend(frame(&header).map_err(before)?);
    let file = publish(dir, &entry_file(dir, id, "journal"), &journal)?;
    Ok((file, u64::try_from(journal.len()).unwrap_or(u64::MAX)))
}

/// The journal size above which a snapshot of `snapshot_bytes` is replaced, at least `floor`.
fn rebase_limit(snapshot_bytes: u64, floor: u64) -> u64 {
    snapshot_bytes.saturating_mul(REBASE_RATIO).max(floor)
}

/// The writing side of one recovery entry. It holds the entry's lock for its whole life, so a
/// scan for entries left by crashed sessions can tell this one is still in use.
#[derive(Debug)]
pub struct EditJournal {
    dir: PathBuf,
    id: String,
    /// Kept open and locked; released when the journal is dropped or the process ends.
    _lock: File,
    /// The journal, open for appending. `None` before the first snapshot, after an append
    /// failed part-way or the journal was removed, and after a rebase failed once something was
    /// renamed into place; the next edit then starts a new snapshot instead of appending.
    file: Option<File>,
    /// A removed entry may have lost its lock path too; never reuse it after detecting removal.
    removed: bool,
    documents: Documents,
    sha256: String,
    /// Bytes in the journal file, header included.
    journal_bytes: u64,
    /// Journal size above which the next recorded edit starts a new snapshot.
    rebase_after: u64,
    /// The least `rebase_after`; [`REBASE_FLOOR_BYTES`] outside tests.
    floor: u64,
}

impl EditJournal {
    /// Create entry `id` in `dir` (creating the folder) with `base` as its recovery snapshot and
    /// an empty journal. Fails if the id is not a plain file stem, another writer holds the
    /// entry, or the folder cannot be written; nothing outside `dir` is touched.
    pub fn create(dir: &Path, id: &str, base: Documents) -> Result<Self, String> {
        let mut journal = Self::lock(dir, id)?;
        journal.rebase(base)?;
        Ok(journal)
    }

    /// Take the lock of entry `id` in `dir` (creating the folder) without writing a snapshot: no
    /// journal is open, so the first recorded state becomes the snapshot. The lock is held even
    /// while snapshots fail, so the entry is never taken for one left by a crashed session.
    fn lock(dir: &Path, id: &str) -> Result<Self, String> {
        if !valid_id(id) {
            return Err(format!("`{id}` is not a recovery entry id."));
        }
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let lock = open_lock_file(&entry_file(dir, id, "lock")).map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|_| format!("Recovery entry `{id}` is in use by another session."))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            id: id.to_owned(),
            _lock: lock,
            file: None,
            removed: false,
            documents: Documents::new(),
            sha256: String::new(),
            journal_bytes: 0,
            rebase_after: REBASE_FLOOR_BYTES,
            floor: REBASE_FLOOR_BYTES,
        })
    }

    /// The entry id, the stem of each of its files.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Fingerprint of the documents after the last recorded edit.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Replace the snapshot with `base` and start an empty journal. A failure before the
    /// snapshot's rename preserves the existing pair and the open journal. After that rename,
    /// `base` becomes the recorded state and the old journal is closed, even if the folder flush
    /// fails; the next edit retries with a new snapshot. A journal belongs to one publication,
    /// so an old journal is ignored even when the new snapshot's documents have not changed.
    /// The old journal closes before replacement because Windows refuses to replace an open file.
    /// Once removal was detected, the entry stays disabled even if its folder is recreated: its
    /// original lock handle cannot protect a recreated lock path.
    pub fn rebase(&mut self, base: Documents) -> Result<(), String> {
        if self.removed {
            return Err(REMOVED_ENTRY.to_owned());
        }
        let (staged, header, snapshot_bytes) = stage_snapshot(&self.dir, &base)?;
        let published = publish_staged(&self.dir, &entry_file(&self.dir, &self.id, "pc"), staged);
        if let Err(PublishError { replaced: false, message }) = published {
            return Err(message);
        }
        drop(self.file.take());
        self.documents = base;
        self.sha256 = header.base_sha256.clone();
        drop(published?);
        let (file, journal_bytes) = publish_journal(&self.dir, &self.id, &header)?;
        self.file = Some(file);
        self.journal_bytes = journal_bytes;
        self.rebase_after = rebase_limit(snapshot_bytes, self.floor);
        Ok(())
    }

    /// Record that `edit` produced `documents`, and flush the record to disk before returning.
    /// After a failed append the journal may end in a torn record, so the next call starts a new
    /// snapshot rather than appending past it. Detected removal disables the entry, since its
    /// lock path may have been removed too. An edit that changed no document is not recorded.
    /// Once the journal passes its size limit, the recorded state becomes a new snapshot. If
    /// rebasing fails after a successful append, that edit remains in either the old journal or
    /// the new snapshot, depending on the publication stage, and the error is returned.
    pub fn append(&mut self, edit: &CommittedEdit, documents: Documents) -> Result<(), String> {
        let Some(file) = self.file.as_mut() else {
            return self.rebase(documents);
        };
        let sha256 = documents_sha256(&documents);
        if sha256 == self.sha256 {
            return Ok(());
        }
        let bytes = frame(&encode_record(edit, &self.documents, &self.sha256, &documents, &sha256)?)?;
        if let Err(error) = write_record(&self.dir, file, &bytes) {
            // The journal may now end in a torn record; close it so nothing is appended past it.
            self.file = None;
            return Err(error.to_string());
        }
        // Writes to a journal whose folder was removed still succeed on Unix, into a file
        // recovery can no longer find; report that instead of a record that cannot be recovered.
        if let Err(error) = fs::metadata(entry_file(&self.dir, &self.id, "journal"))
            && error.kind() == ErrorKind::NotFound
        {
            self.file = None;
            self.removed = true;
            return Err(REMOVED_ENTRY.to_owned());
        }
        self.journal_bytes = self.journal_bytes.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        self.documents = documents;
        self.sha256 = sha256;
        if self.journal_bytes > self.rebase_after {
            return self.rebase(self.documents.clone()).map_err(|e| format!("The recovery snapshot could not be renewed: {e}"));
        }
        Ok(())
    }
}

enum Message {
    Opened(Arc<Project>),
    Committed(CommittedEdit),
    Sync(mpsc::SyncSender<Result<(), String>>),
}

/// An [`EditObserver`] that journals a session's edits on a worker thread, so neither editing
/// nor saving waits for encoding or disk. Attach it with
/// [`pcbcraft_engine::Session::observe_edits`]. The entry is created from the first state it
/// is told about; a later open starts a new snapshot in the same entry.
///
/// Dropping the recorder lets the worker finish what is queued in the background. A process kill
/// can lose queued or in-flight edits. Earlier notifications need not have been recorded either:
/// encoding or disk writes can fail, and a detected removed entry stays disabled. Recovery returns
/// the last complete state that was successfully recorded and remains readable.
#[derive(Debug)]
pub struct JournalRecorder {
    sender: mpsc::Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl JournalRecorder {
    /// Start the worker for entry `id` in `dir`. Nothing is written until the session reports
    /// its first state. Fails only if the id is invalid or the thread cannot start.
    pub fn start(dir: PathBuf, id: String) -> Result<Self, String> {
        if !valid_id(&id) {
            return Err(format!("`{id}` is not a recovery entry id."));
        }
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("pcbcraft-recovery".to_owned())
            .spawn(move || record(&dir, &id, &receiver))
            .map_err(|e| e.to_string())?;
        Ok(Self { sender, worker: Some(worker) })
    }

    /// Wait until the worker has processed notifications reported so far. Returns the first error
    /// since the last call; this is a queue barrier, not a guarantee that every edit is on disk.
    /// Recording failures can leave edits unrecorded until a snapshot succeeds; a detected removed
    /// entry stays disabled. A later successful snapshot can include earlier unrecorded changes.
    pub fn sync(&self) -> Result<(), String> {
        let (reply, done) = mpsc::sync_channel(1);
        self.sender.send(Message::Sync(reply)).map_err(|_| "The recovery journal has stopped.".to_owned())?;
        done.recv().map_err(|_| "The recovery journal has stopped.".to_owned())?
    }

    /// Write what is queued, stop the worker and release the entry's lock. The entry's files
    /// stay for the caller to keep or discard.
    pub fn finish(self) -> Result<(), String> {
        let result = self.sync();
        let Self { sender, worker } = self;
        drop(sender);
        if let Some(worker) = worker {
            worker.join().map_err(|_| "The recovery journal stopped unexpectedly.".to_owned())?;
        }
        result
    }
}

impl EditObserver for JournalRecorder {
    /// Queue a new history root; never blocks.
    fn opened(&self, project: Arc<Project>) {
        // A stopped worker has already kept its error for `sync`.
        let _ = self.sender.send(Message::Opened(project));
    }

    /// Queue a committed edit; never blocks.
    fn committed(&self, edit: CommittedEdit) {
        let _ = self.sender.send(Message::Committed(edit));
    }
}

/// The worker: encode each reported state and write it to the entry, until the recorder is gone.
/// Errors are kept for the next `sync`. A state that cannot be encoded is skipped; the next
/// recorded edit carries its changes, or a new snapshot follows a root that failed.
fn record(dir: &Path, id: &str, receiver: &mpsc::Receiver<Message>) {
    let mut journal: Option<EditJournal> = None;
    let mut error: Option<String> = None;
    while let Ok(message) = receiver.recv() {
        let result = match message {
            Message::Sync(reply) => {
                let _ = reply.send(error.take().map_or(Ok(()), Err));
                continue;
            }
            Message::Opened(project) => {
                let result = encode_documents(project).and_then(|documents| write(&mut journal, dir, id, None, documents));
                // A root that could not be snapshotted must not be journaled as an edit.
                if result.is_err()
                    && let Some(journal) = &mut journal
                {
                    journal.file = None;
                }
                result
            }
            // An edit that cannot be encoded is folded into the next record, which starts from
            // the last state written; the journal closes itself after a failed write.
            Message::Committed(edit) => {
                encode_documents(edit.project.clone()).and_then(|documents| write(&mut journal, dir, id, Some(&edit), documents))
            }
        };
        if let Err(failure) = result {
            error.get_or_insert(failure);
        }
    }
}

/// Record `documents` as the result of `edit`, or as a new snapshot when there is no edit. The
/// first state creates the entry, so it becomes the snapshot whatever produced it.
fn write(journal: &mut Option<EditJournal>, dir: &Path, id: &str, edit: Option<&CommittedEdit>, documents: Documents) -> Result<(), String> {
    match (journal.as_mut(), edit) {
        (Some(journal), Some(edit)) => journal.append(edit, documents),
        (Some(journal), None) => journal.rebase(documents),
        // The lock is kept even if this first snapshot fails; the next state tries again.
        (None, _) => journal.insert(EditJournal::lock(dir, id)?).rebase(documents),
    }
}

/// Why replay ended before the end of the journal. The recovered state is the last one that
/// was complete and consistent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The journal header is missing, torn or damaged; the snapshot alone was recovered.
    Header(String),
    /// The journal continues a different snapshot, so a newer snapshot replaced it.
    OtherSnapshot,
    /// Record `index` (from 0) is torn or unreadable.
    Unreadable { index: usize, reason: String },
    /// Record `index` does not start from, or does not produce, the state its hashes name.
    Mismatch { index: usize, command: String },
}

/// What [`recover`] found for an entry.
#[derive(Debug)]
pub struct Recovered {
    /// The recovered documents, as the project codec writes them.
    pub documents: Documents,
    /// The recovered project, decoded from `documents`.
    pub project: Arc<Project>,
    /// The commands replayed on top of the snapshot, in order.
    pub commands: Vec<String>,
    /// Why replay ended early, if it did.
    pub stop: Option<Stop>,
}

/// Read entry `id` in `dir`: its snapshot, then each journal record that continues it.
///
/// Replay stops at the first torn, damaged or inconsistent record and returns the state before
/// it; a journal written for another snapshot is ignored. Fails, writing nothing, when the
/// snapshot is missing or unreadable or the journal names a format this build does not read,
/// which callers offer for discard.
pub fn recover(dir: &Path, id: &str) -> Result<Recovered, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a recovery entry id."));
    }
    let snapshot = read_bounded(&entry_file(dir, id, "pc"), MAX_ARCHIVE_BYTES + SNAPSHOT_OVERHEAD_BYTES)?;
    let (header, mut documents) = decode_snapshot(&snapshot)?;
    let mut sha256 = documents_sha256(&documents);
    let mut commands = Vec::new();
    let stop = match File::open(entry_file(dir, id, "journal")) {
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => Some(Stop::Header(error.to_string())),
        Ok(file) => replay(&mut BufReader::new(file), &header.generation, &mut documents, &mut sha256, &mut commands)?,
    };
    let project = decode_documents(&documents)?;
    Ok(Recovered { documents, project, commands, stop })
}

/// Decode a versioned recovery snapshot without treating it as a user project archive. Earlier
/// unwrapped snapshots are explicitly refused, not paired with a generation-less journal.
fn decode_snapshot(bytes: &[u8]) -> Result<(Header, Documents), String> {
    if bytes.starts_with(b"PK") {
        return Err("This recovery entry uses pcbcraft-recovery/1, which this version can't read.".to_owned());
    }
    let (magic, mut rest) = bytes.split_at_checked(MAGIC.len()).ok_or("Recovery snapshot is cut short.")?;
    if magic != MAGIC {
        let line = magic.split(|byte| *byte == b'\n').next().unwrap_or_default();
        return Err(format!("Unsupported recovery snapshot format: {}.", String::from_utf8_lossy(line)));
    }
    if let Some(length) = rest.first_chunk::<4>()
        && u64::from(u32::from_le_bytes(*length)) + 36 + MAGIC.len() as u64 > SNAPSHOT_OVERHEAD_BYTES as u64
    {
        return Err("Recovery snapshot header exceeds its size limit.".to_owned());
    }
    let metadata = read_frame(&mut rest)?.ok_or("Recovery snapshot has no header.")?;
    let header: Header = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    let documents = decode_packed(rest)?.documents;
    if header.base_sha256 != documents_sha256(&documents) {
        return Err("Recovery snapshot documents do not match its header.".to_owned());
    }
    Ok((header, documents))
}

/// Apply records only if the journal names `generation` and the documents' hash. Only an
/// unsupported format is an error; every other problem ends replay with its reason.
fn replay(
    reader: &mut impl Read,
    generation: &str,
    documents: &mut Documents,
    sha256: &mut String,
    commands: &mut Vec<String>,
) -> Result<Option<Stop>, String> {
    let mut magic = [0; MAGIC.len()];
    if reader.read_exact(&mut magic).is_err() {
        return Ok(Some(Stop::Header("journal is cut short".to_owned())));
    }
    if magic != MAGIC {
        let line = magic.split(|byte| *byte == b'\n').next().unwrap_or_default();
        if line.starts_with(b"pcbcraft-recovery/") {
            return Err(format!("This recovery entry uses {}, which this version can't read.", String::from_utf8_lossy(line)));
        }
        return Ok(Some(Stop::Header("not a recovery journal".to_owned())));
    }
    let header: Header = match read_frame(reader) {
        Ok(Some(payload)) => match serde_json::from_slice(&payload) {
            Ok(header) => header,
            Err(error) => return Ok(Some(Stop::Header(error.to_string()))),
        },
        Ok(None) => return Ok(Some(Stop::Header("journal has no header".to_owned()))),
        Err(reason) => return Ok(Some(Stop::Header(reason))),
    };
    if header.generation != generation || header.base_sha256 != *sha256 {
        return Ok(Some(Stop::OtherSnapshot));
    }
    let mut index = 0;
    loop {
        let (record, written) = match read_frame(reader).and_then(|frame| frame.map(|payload| decode_record(&payload)).transpose()) {
            Ok(Some(record)) => record,
            Ok(None) => return Ok(None),
            Err(reason) => return Ok(Some(Stop::Unreadable { index, reason })),
        };
        if record.before_sha256 != *sha256 {
            return Ok(Some(Stop::Mismatch { index, command: record.command }));
        }
        let mut next = documents.clone();
        for path in &record.removed {
            next.remove(path);
        }
        next.extend(written);
        let next_sha256 = documents_sha256(&next);
        if next_sha256 != record.after_sha256 {
            return Ok(Some(Stop::Mismatch { index, command: record.command }));
        }
        *documents = next;
        *sha256 = next_sha256;
        commands.push(record.command);
        index += 1;
    }
}

/// Read a regular file of at most `limit` bytes.
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("{}: exceeds size limit", path.display()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
