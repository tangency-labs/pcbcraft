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
//!
//! A [`Schedule`] decides when the snapshot is replaced by the current state, which empties the
//! journal. The same timer paces the opt-in save to the project, which goes through the
//! ordinary save path and never through this module.

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
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// First line of every journal. A reader refuses any other version rather than guess at it.
pub const FORMAT: &str = "pcbcraft-recovery/2";
const MAGIC: &[u8] = b"pcbcraft-recovery/2\n";
/// Room for the version and framed snapshot header, in addition to the bounded project archive.
const SNAPSHOT_OVERHEAD_BYTES: usize = 4096;
/// Maximum payload bytes of a framed recovery header within [`SNAPSHOT_OVERHEAD_BYTES`].
const MAX_HEADER_BYTES: usize = SNAPSHOT_OVERHEAD_BYTES - MAGIC.len() - 36;
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

/// How long editing must pause before a timed snapshot or save runs, so it does not land in
/// the middle of a burst of edits. A burst never delays it beyond a second interval.
pub const QUIET: Duration = Duration::from_secs(2);
/// An edit that ran at least this long is followed by a snapshot whatever the interval, as
/// SolidCraft does after a big operation.
pub const LONG_OPERATION: Duration = Duration::from_millis(1500);

/// When to take the next timed snapshot or save. Times are offsets on any steady clock the
/// caller keeps, such as a frame clock, so tests can drive it without waiting.
///
/// Nothing is due until something changes. A change is due one interval after the first change
/// since the last snapshot, once editing has been quiet for [`QUIET`], or after two intervals
/// however busy editing is. An interval of Off takes snapshots only after a long operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    interval: Option<Duration>,
    /// The first and the latest change not yet taken.
    pending: Option<(Duration, Duration)>,
    /// A long operation asked for the next snapshot without waiting for the interval.
    urgent: bool,
}

impl Schedule {
    /// A schedule with an interval of `minutes`; 0 is Off. Nothing is pending.
    pub fn new(minutes: u8) -> Self {
        let mut schedule = Self::default();
        schedule.set_minutes(minutes);
        schedule
    }

    /// Change the interval. Pending changes stay pending and are measured against it.
    pub fn set_minutes(&mut self, minutes: u8) {
        self.interval = (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60));
    }

    /// Whether a change is waiting to be taken.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Note a change at `now`.
    pub fn changed(&mut self, now: Duration) {
        let first = self.pending.map_or(now, |(first, _)| first.min(now));
        self.pending = Some((first, now));
    }

    /// Note a change made by an edit that took `took`; at least [`LONG_OPERATION`] makes the
    /// next snapshot due at once.
    pub fn long_operation(&mut self, took: Duration, now: Duration) {
        self.changed(now);
        self.urgent |= took >= LONG_OPERATION;
    }

    /// Whether a snapshot or save should run at `now`.
    pub fn due(&self, now: Duration) -> bool {
        self.wait(now) == Some(Duration::ZERO)
    }

    /// How long from `now` until the next one is due, zero when due, or `None` when nothing
    /// is pending or the interval is Off without an urgent long-operation request. Callers
    /// use it to wake in time.
    pub fn wait(&self, now: Duration) -> Option<Duration> {
        let (first, latest) = self.pending?;
        if self.urgent {
            return Some(Duration::ZERO);
        }
        let interval = self.interval?;
        let quiet = latest.saturating_add(QUIET).min(first.saturating_add(interval.saturating_mul(2)));
        Some(first.saturating_add(interval).max(quiet).saturating_sub(now))
    }

    /// The pending changes were taken, or the state is again the saved one: nothing is pending.
    pub fn taken(&mut self) {
        self.pending = None;
        self.urgent = false;
    }
}

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
    read_frame_bounded(reader, MAX_FRAME_BYTES)
}

/// Read the next frame, rejecting any declared payload length above `max_bytes` before allocating.
fn read_frame_bounded(reader: &mut impl Read, max_bytes: usize) -> Result<Option<Vec<u8>>, String> {
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
    if length > max_bytes {
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
    // Setup review authority is session-local and already spent. Recovery replays the documents, not this capability.
    if edit.command == pcbcraft_engine::SET_BOARD_SETUP
        && let Some(params) = params.as_object_mut()
    {
        params.remove("plan_token");
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
        /// A post-write journal identity check.
        Identity,
        /// Opening a candidate entry's `.lock` file during retention.
        LockPermission,
        /// Reading a candidate entry's modification times during retention.
        EntryMetadata,
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
fn stage_snapshot(dir: &Path, base: &Documents, packed: &[u8]) -> Result<(tempfile::NamedTempFile, Header, u64), String> {
    let mut staged = tempfile::Builder::new().prefix(".recovery-").rand_bytes(24).tempfile_in(dir).map_err(|e| e.to_string())?;
    let generation = staged.path().file_name().ok_or("The recovery staging file has no name.")?.to_string_lossy().into_owned();
    let header = Header { generation, base_sha256: documents_sha256(base) };
    let metadata = frame(&serde_json::to_vec(&header).map_err(|e| e.to_string())?)?;
    staged.write_all(MAGIC).and_then(|()| staged.write_all(&metadata)).and_then(|()| staged.write_all(packed)).map_err(|e| e.to_string())?;
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
    /// Fingerprint of the snapshot documents.
    base_sha256: String,
    /// Publication token written into the current snapshot and journal headers.
    generation: String,
    /// Byte length of the published `.pc` snapshot file.
    snapshot_bytes: u64,
    /// Unix `(dev, ino)` of the published `.pc` snapshot file.
    #[cfg(unix)]
    snapshot_file_id: Option<(u64, u64)>,
    /// Bytes in the published header-only journal file.
    journal_header_bytes: u64,
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
        let mut journal = Self::lock(dir, id, false)?;
        journal.rebase(base)?;
        Ok(journal)
    }

    /// Take the lock of entry `id` in `dir` (creating the folder) without writing a snapshot: no
    /// journal is open, so the first recorded state becomes the snapshot. The lock is held even
    /// while snapshots fail, so the entry is never taken for one left by a crashed session.
    /// The folder gate and entry lock are tried without waiting. A new recorder reserves a
    /// fresh name instead of adopting an entry saved by another run.
    fn lock(dir: &Path, id: &str, fresh: bool) -> Result<Self, String> {
        if !valid_id(id) {
            return Err(format!("`{id}` is not a recovery entry id."));
        }
        fs::create_dir_all(dir).map_err(|e| format!("Could not create the recovery folder: {e}"))?;
        let _gate = entry_gate(dir).map_err(|e| e.to_string())?;
        if fresh {
            for ext in ["pc", "journal"] {
                if entry_file(dir, id, ext).try_exists().map_err(|e| e.to_string())? {
                    return Err(format!("Recovery entry `{id}` already exists."));
                }
            }
        }
        let lock = open_entry_lock(&entry_file(dir, id, "lock"), fresh).map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|_| format!("Recovery entry `{id}` is in use by another session."))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            id: id.to_owned(),
            _lock: lock,
            file: None,
            removed: false,
            documents: Documents::new(),
            sha256: String::new(),
            base_sha256: String::new(),
            generation: String::new(),
            snapshot_bytes: 0,
            #[cfg(unix)]
            snapshot_file_id: None,
            journal_header_bytes: 0,
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

    /// Verify the entry's lock path still identifies the lock handle held by this journal.
    /// If the lock file or its directory was removed or replaced, disable the entry so a
    /// timed snapshot or rebase cannot publish unprotected files into a recreated directory.
    fn ensure_lock_owned(&mut self) -> Result<(), String> {
        if self.removed {
            return Err(REMOVED_ENTRY.to_owned());
        }
        let lock_path = entry_file(&self.dir, &self.id, "lock");
        let path_meta = match fs::symlink_metadata(&lock_path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.file = None;
                self.removed = true;
                return Err(REMOVED_ENTRY.to_owned());
            }
            Err(error) => return Err(error.to_string()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let held_meta = self._lock.metadata().map_err(|e| e.to_string())?;
            if (path_meta.dev(), path_meta.ino()) != (held_meta.dev(), held_meta.ino()) {
                self.file = None;
                self.removed = true;
                return Err(REMOVED_ENTRY.to_owned());
            }
        }
        #[cfg(not(unix))]
        let _ = path_meta;
        Ok(())
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
        self.ensure_lock_owned()?;
        let packed = encode_packed_documents(base.clone())?;
        self.rebase_with_packed(base, &packed)
    }

    fn rebase_packed(&mut self, base: Documents, packed: &[u8]) -> Result<(), String> {
        self.ensure_lock_owned()?;
        self.rebase_with_packed(base, packed)
    }

    fn rebase_with_packed(&mut self, base: Documents, packed: &[u8]) -> Result<(), String> {
        let (staged, header, snapshot_bytes) = stage_snapshot(&self.dir, &base, packed)?;
        let published = publish_staged(&self.dir, &entry_file(&self.dir, &self.id, "pc"), staged);
        if let Err(PublishError { replaced: false, message }) = published {
            return Err(message);
        }
        drop(self.file.take());
        self.documents = base;
        self.sha256 = header.base_sha256.clone();
        self.base_sha256.clone_from(&self.sha256);
        self.generation.clone_from(&header.generation);
        self.snapshot_bytes = snapshot_bytes;
        #[cfg(unix)]
        {
            self.snapshot_file_id = None;
        }
        let published_file = published?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = published_file.metadata().map_err(|e| e.to_string())?;
            self.snapshot_file_id = Some((meta.dev(), meta.ino()));
        }
        drop(published_file);
        let (file, journal_bytes) = publish_journal(&self.dir, &self.id, &header)?;
        self.file = Some(file);
        self.journal_header_bytes = journal_bytes;
        self.journal_bytes = journal_bytes;
        self.rebase_after = rebase_limit(snapshot_bytes, self.floor);
        Ok(())
    }

    /// Take a recovery snapshot of `documents`: [`Self::rebase`] onto them, unless the journal
    /// is open and header-only (no appended records), both its recorded result and its base
    /// already equal the requested state, the current `.pc` snapshot file still matches this
    /// recorder's publication, and the `.journal` file still names the open header-only journal
    /// with its expected byte length and publication header. Rebasing whenever the journal holds
    /// records prevents skipping publication over unverified or in-place damaged records (for
    /// example after edits that return to the base state). Checking the recorded result matters
    /// if a notification failed to return the journal to its base, for example when an undo could
    /// not be encoded. Returns whether it wrote.
    pub fn snapshot(&mut self, documents: Documents) -> Result<bool, String> {
        self.ensure_lock_owned()?;
        if self.file.is_some()
            && self.journal_bytes == self.journal_header_bytes
            && self.base_sha256 == self.sha256
            && documents_sha256(&documents) == self.base_sha256
            && self.snapshot_matches_publication()?
            && self.journal_matches_publication()?
        {
            return Ok(false);
        }
        self.rebase(documents).map(|()| true)
    }

    /// Return whether `<id>.journal` still names the open header-only regular journal file this
    /// recorder published (same open file, expected byte length, and exact matching header
    /// generation and base hash with no trailing bytes). Missing, non-regular, replaced,
    /// truncated, or corrupted journals return `Ok(false)` so [`Self::snapshot`] rebuilds the
    /// pair under the held entry lock; other metadata, identity, or read errors propagate as
    /// faults.
    fn journal_matches_publication(&mut self) -> Result<bool, String> {
        let Some(journal_file) = self.file.as_mut() else {
            return Ok(false);
        };
        let path = entry_file(&self.dir, &self.id, "journal");
        if !same_open_file(&path, journal_file)? {
            return Ok(false);
        }
        let open_meta = journal_file.metadata().map_err(|e| e.to_string())?;
        if !open_meta.file_type().is_file() || open_meta.len() != self.journal_bytes {
            return Ok(false);
        }
        let mut buf = [0u8; SNAPSHOT_OVERHEAD_BYTES];
        let read = read_header_prefix(journal_file, &mut buf).map_err(|e| e.to_string())?;
        if u64::try_from(read).ok() != Some(self.journal_bytes) {
            return Ok(false);
        }
        let mut cursor = &buf[..read];
        let Ok(header) = read_recovery_header_from(&mut cursor) else {
            return Ok(false);
        };
        Ok(cursor.is_empty() && header.generation == self.generation && header.base_sha256 == self.base_sha256)
    }

    /// Return whether `<id>.pc` still names the regular recovery snapshot publication this
    /// recorder wrote (same file identity on Unix, same byte length, and matching header
    /// generation and base hash). Missing, non-regular, or replaced files return `Ok(false)` so
    /// [`Self::snapshot`] repairs the pair under the held entry lock; other metadata or open
    /// errors propagate as faults.
    fn snapshot_matches_publication(&self) -> Result<bool, String> {
        let path = entry_file(&self.dir, &self.id, "pc");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.file_type().is_file() || metadata.len() != self.snapshot_bytes {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if self.snapshot_file_id != Some((metadata.dev(), metadata.ino())) {
                return Ok(false);
            }
        }
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        };
        let Ok(header) = read_recovery_header_from(file) else {
            return Ok(false);
        };
        Ok(header.generation == self.generation && header.base_sha256 == self.base_sha256)
    }

    /// Record that `edit` produced `documents`, and flush the record to disk before returning.
    /// After a failed append or post-write verification error, the journal is closed so the next
    /// call starts a new snapshot rather than appending past it. Detected removal disables the
    /// entry, since its lock path may have been removed too. An edit that changed no document is
    /// not recorded. Once the journal passes its size limit, the recorded state becomes a new
    /// snapshot. If rebasing fails after a successful append, that edit remains in either the old
    /// journal or the new snapshot, depending on the publication stage, and the error is returned.
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
        // Writes to a journal whose file or folder was removed or replaced still succeed on Unix,
        // into an unlinked file recovery can no longer find; report that instead of a record that
        // cannot be recovered.
        match same_open_file(&entry_file(&self.dir, &self.id, "journal"), file) {
            Ok(true) => {}
            Ok(false) => {
                self.file = None;
                self.removed = true;
                return Err(REMOVED_ENTRY.to_owned());
            }
            Err(error) => {
                // The record was already flushed while `self.sha256` still names the prior state;
                // close the journal so the next edit or snapshot rebases instead of appending from
                // the stale hash.
                self.file = None;
                return Err(error);
            }
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

/// Read up to `buf.len()` bytes from the start of `file` without disturbing its write cursor.
fn read_header_prefix(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        let mut offset = 0;
        while offset < buf.len() {
            match file.read_at(&mut buf[offset..], offset as u64) {
                Ok(0) => break,
                Ok(n) => offset += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(offset)
    }
    #[cfg(not(unix))]
    {
        use std::io::{Seek, SeekFrom};
        let saved = file.stream_position()?;
        let result = file.seek(SeekFrom::Start(0)).and_then(|_| {
            let mut offset = 0;
            while offset < buf.len() {
                match file.read(&mut buf[offset..]) {
                    Ok(0) => break,
                    Ok(n) => offset += n,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(offset)
        });
        let restored = file.seek(SeekFrom::Start(saved));
        let offset = result?;
        restored?;
        Ok(offset)
    }
}

/// Return whether `path` currently names the same regular file as the open handle `open`.
fn same_open_file(path: &Path, open: &File) -> Result<bool, String> {
    #[cfg(test)]
    if path.parent().is_some_and(|dir| faults::fails(dir, faults::Fault::Identity)) {
        return Err("The recovery journal could not be verified.".to_owned());
    }
    let path_meta = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => meta,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held_meta = open.metadata().map_err(|e| e.to_string())?;
        Ok((path_meta.dev(), path_meta.ino()) == (held_meta.dev(), held_meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path_meta, open);
        Ok(true)
    }
}

enum Message {
    Opened(Arc<Project>),
    /// A compacted pending burst replacing the queued tail with its newest state while keeping
    /// a bounded list of earlier pending states (oldest-to-newest) until encoding succeeds.
    Compacted {
        project: Arc<Project>,
        fallbacks: Vec<Message>,
    },
    Committed(CommittedEdit),
    Snapshot(Arc<Project>),
    Sync(mpsc::SyncSender<Result<(), String>>),
}

/// Max queued state notifications between barriers before compacting to a new history root.
/// Committed edits are compacted only when `coalesce` is enabled; non-coalescing recorders
/// retain every queued edit so individual command records are not dropped.
const PENDING_LIMIT: usize = 32;

/// Bound compacted fallbacks to `PENDING_LIMIT`, keeping the oldest and newest entries.
fn bound_fallbacks(fallbacks: &mut Vec<Message>) {
    if fallbacks.len() > PENDING_LIMIT {
        let excess = fallbacks.len() - PENDING_LIMIT;
        fallbacks.drain(1..=excess);
    }
}

#[derive(Default)]
struct Pending {
    queue: std::collections::VecDeque<Message>,
    /// `Some(None)` drains and preserves; `Some(Some(..))` may also discard before the deadline.
    stop: Option<Option<Instant>>,
    exited: bool,
    fault: Option<RecordingFault>,
}

#[derive(Default)]
struct Mailbox {
    state: Mutex<Pending>,
    ready: Condvar,
    /// Hand the worker only the newest of consecutive queued edits (see [`RecorderOptions`]).
    coalesce: bool,
}

/// The current recovery failure, retained until a successful snapshot repairs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingFault {
    pub message: String,
    /// Detected removal invalidates the recorder's entry lock and disables it until replaced.
    pub removed: bool,
}

const WORKER_STOPPED_UNEXPECTEDLY: &str = "The recovery worker stopped unexpectedly.";

/// Mark a panicked or stopped worker unavailable, release queued barriers, publish the worker's
/// completion outcome, and wake the host afterwards.
struct WorkerExit<F: Fn()> {
    mailbox: Arc<Mailbox>,
    done: Option<mpsc::SyncSender<Result<(), String>>>,
    wake: F,
}

impl<F: Fn()> WorkerExit<F> {
    fn mark_exited(&self, panicked: bool) {
        let mut state = self.mailbox.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.exited = true;
        state.queue.clear();
        if panicked {
            state.fault = Some(RecordingFault { message: WORKER_STOPPED_UNEXPECTEDLY.into(), removed: true });
        }
    }

    fn finish(mut self, result: Result<(), String>, panicked: bool) {
        self.mark_exited(panicked);
        if let Some(done) = self.done.take() {
            let _ = done.send(result);
        }
        (self.wake)();
    }
}

impl<F: Fn()> Drop for WorkerExit<F> {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            self.mark_exited(std::thread::panicking());
            drop(done);
        }
    }
}

/// Run the recorder worker body so both normal completion and unexpected panics mark the mailbox
/// exited and publish the outcome to [`RecorderExit`] before waking the host outside unwinding.
fn run_worker<F: Fn()>(
    mailbox: Arc<Mailbox>,
    done: mpsc::SyncSender<Result<(), String>>,
    wake: F,
    work: impl FnOnce(&Arc<Mailbox>, &F) -> Result<(), String>,
) {
    let exit = WorkerExit { mailbox, done: Some(done), wake };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&exit.mailbox, &exit.wake))) {
        Ok(result) => exit.finish(result, false),
        Err(payload) => {
            exit.finish(Err(WORKER_STOPPED_UNEXPECTEDLY.into()), true);
            std::panic::resume_unwind(payload);
        }
    }
}

impl Mailbox {
    /// Enqueue a state without waiting for disk. Only the worker performs encoding and writes.
    fn push(&self, message: Message) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.exited {
            return;
        }
        if matches!(message, Message::Sync(_)) || (!self.coalesce && matches!(message, Message::Committed(_))) {
            state.queue.push_back(message);
            self.ready.notify_one();
            return;
        }
        let tail = state.queue.iter().rev().take_while(|m| !matches!(m, Message::Sync(_))).count();
        let compactable =
            tail >= PENDING_LIMIT && (self.coalesce || state.queue.iter().rev().take(tail).all(|m| !matches!(m, Message::Committed(_))));
        if compactable {
            let retained = state.queue.len() - tail;
            let drained: Vec<Message> = state.queue.drain(retained..).collect();
            let mut fallbacks = Vec::new();
            for item in drained {
                match item {
                    Message::Compacted { project, fallbacks: saved } => {
                        fallbacks.extend(saved);
                        fallbacks.push(Message::Opened(project));
                    }
                    Message::Opened(project) => {
                        // Never fall back across an explicit project open.
                        fallbacks.clear();
                        fallbacks.push(Message::Opened(project));
                    }
                    Message::Committed(_) | Message::Snapshot(_) => fallbacks.push(item),
                    Message::Sync(_) => {}
                }
            }
            match message {
                Message::Opened(project) => {
                    // An explicit open starts a new project root; do not fall back to the previous one.
                    state.queue.push_back(Message::Opened(project));
                }
                Message::Snapshot(project) => {
                    bound_fallbacks(&mut fallbacks);
                    state.queue.push_back(Message::Compacted { project, fallbacks });
                }
                Message::Committed(edit) => {
                    bound_fallbacks(&mut fallbacks);
                    state.queue.push_back(Message::Compacted { project: edit.project, fallbacks });
                }
                Message::Compacted { project, fallbacks: mut extra } => {
                    fallbacks.append(&mut extra);
                    bound_fallbacks(&mut fallbacks);
                    state.queue.push_back(Message::Compacted { project, fallbacks });
                }
                Message::Sync(reply) => state.queue.push_back(Message::Sync(reply)),
            }
        } else {
            state.queue.push_back(message);
        }
        self.ready.notify_one();
    }

    /// Drain queued notifications first and then return the shutdown decision.
    #[cfg(test)]
    fn next(&self) -> Result<Message, Option<Instant>> {
        self.next_with_fallbacks().map(|(message, _)| message)
    }

    /// Drain the next notification along with any consecutive committed edits it coalesced,
    /// ordered oldest-to-newest.
    fn next_with_fallbacks(&self) -> Result<(Message, Vec<Message>), Option<Instant>> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state =
            self.ready.wait_while(state, |state| state.queue.is_empty() && state.stop.is_none()).unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut message = state.queue.pop_front();
        let mut fallbacks = Vec::new();
        // Each record holds the documents it produces, so the newest of a run of edits replays
        // to the same state as the whole run, and only that state is encoded unless it fails.
        while self.coalesce && matches!(message, Some(Message::Committed(_))) && matches!(state.queue.front(), Some(Message::Committed(_))) {
            if let Some(next) = state.queue.pop_front()
                && let Some(previous) = message.replace(next)
            {
                fallbacks.push(previous);
            }
        }
        message.map(|message| (message, fallbacks)).ok_or(state.stop.flatten())
    }

    /// Stop accepting notifications after the owner releases its last handle.
    fn close(&self, deadline: Option<Instant>) {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).stop.get_or_insert(deadline);
        self.ready.notify_one();
    }
}

/// Completion of a retiring recorder. Waiting here is optional; dropping it does not cancel writes.
#[derive(Debug)]
pub struct RecorderExit(Mutex<mpsc::Receiver<Result<(), String>>>);

impl RecorderExit {
    /// Wait no longer than `timeout`; an expired discard deadline prevents cleanup from starting.
    pub fn wait(self, timeout: Duration) -> Result<(), String> {
        self.0.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner).recv_timeout(timeout).map_err(|e| e.to_string())?
    }

    /// Report a completed worker without waiting for it.
    pub fn poll(&self) -> Option<Result<(), String>> {
        match self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("The recovery worker stopped unexpectedly.".into())),
        }
    }
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
pub struct JournalRecorder {
    dir: PathBuf,
    id: String,
    mailbox: Arc<Mailbox>,
    /// Only one barrier is outstanding at a time. Editor notifications never take this lock.
    sync_guard: Mutex<()>,
    exit: Option<RecorderExit>,
}

impl std::fmt::Debug for JournalRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalRecorder").field("dir", &self.dir).field("id", &self.id).finish_non_exhaustive()
    }
}

impl Drop for JournalRecorder {
    fn drop(&mut self) {
        self.mailbox.close(None);
    }
}

impl JournalRecorder {
    /// Start the worker for entry `id` in `dir`. Nothing is written until the session reports
    /// its first state. Fails only if the id is invalid or the thread cannot start.
    pub fn start(dir: PathBuf, id: String) -> Result<Self, String> {
        Self::start_notifying(dir, id, || {})
    }

    /// Start recording, waking the host when recovery health changes or the worker stops.
    /// The callback runs off the editor thread and must not wait for it.
    pub fn start_notifying(dir: PathBuf, id: String, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        Self::start_with(dir, id, RecorderOptions::default(), wake)
    }

    /// Start recording with `options`. With a retention, the worker prunes entries left by
    /// earlier runs before it records anything; a prune failure is reported by the next
    /// [`sync`](Self::sync) and does not stop recording.
    pub fn start_with(dir: PathBuf, id: String, options: RecorderOptions, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        if !valid_id(&id) {
            return Err(format!("`{id}` is not a recovery entry id."));
        }
        let mailbox = Arc::new(Mailbox { coalesce: options.coalesce_edits, ..Mailbox::default() });
        let (worker_dir, worker_id, worker_mailbox) = (dir.clone(), id.clone(), Arc::clone(&mailbox));
        let (done, exit) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("pcbcraft-recovery".to_owned())
            .spawn(move || {
                run_worker(worker_mailbox, done, wake, |mailbox, wake| {
                    let pruned = options.retention.map_or(Ok(0), |retention| prune(&worker_dir, retention));
                    record(&worker_dir, &worker_id, mailbox, wake, pruned.err())
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { dir, id, mailbox, sync_guard: Mutex::new(()), exit: Some(RecorderExit(Mutex::new(exit))) })
    }

    /// The recovery folder this recorder writes in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The entry id, the stem of each of its files.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Queue a recovery snapshot of `project`, the session's current state; never blocks. The
    /// worker skips a snapshot only when its open journal already recovers the requested base
    /// (see [`EditJournal::snapshot`]); a closed journal requires a fresh snapshot attempt,
    /// including after a publication failure. It creates the entry if nothing has been recorded yet.
    pub fn snapshot(&self, project: Arc<Project>) {
        self.mailbox.push(Message::Snapshot(project));
    }

    /// Read the worker's current health without waiting for any I/O.
    pub fn fault(&self) -> Option<RecordingFault> {
        self.mailbox.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).fault.clone()
    }

    /// Wait until the worker has processed notifications reported so far. Returns the first error
    /// since the last call; this is a queue barrier, not a guarantee that every edit is on disk.
    /// Recording failures can leave edits unrecorded until a snapshot succeeds; a detected removed
    /// entry stays disabled. A later successful snapshot can include earlier unrecorded changes.
    pub fn sync(&self) -> Result<(), String> {
        let _guard = self.sync_guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (reply, done) = mpsc::sync_channel(1);
        self.mailbox.push(Message::Sync(reply));
        done.recv().map_err(|_| "The recovery journal has stopped.".to_owned())?
    }

    /// Write what is queued, stop the worker and release the entry's lock. The entry's files
    /// stay for the caller to keep or discard.
    pub fn finish(self) -> Result<(), String> {
        let result = self.sync();
        self.retire(None).0.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner).recv().map_err(|e| e.to_string())??;
        result
    }

    /// Drain the recorder and try to delete only its owned entry. Unowned entries are untouched;
    /// folder-gate contention or an expired deadline leaves the entry for later recovery.
    pub fn discard(self) -> Result<(), String> {
        let timeout = Duration::from_secs(2);
        self.retire(Some(Instant::now() + timeout)).wait(timeout)
    }

    /// Request a drain without blocking. `None` always preserves the entry. With a deadline,
    /// the worker may discard its entry only if drain and gate acquisition finish in time.
    pub fn retire(mut self, discard_before: Option<Instant>) -> RecorderExit {
        self.mailbox.close(discard_before);
        self.exit.take().unwrap_or_else(|| RecorderExit(Mutex::new(mpsc::channel().1)))
    }
}

/// Options for [`JournalRecorder::start_with`]. The default records every edit and prunes
/// nothing, as [`JournalRecorder::start`] does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecorderOptions {
    /// When edits arrive faster than the worker writes them, encode and record only the newest
    /// of each queued run. Recovery reaches the same documents; [`Recovered::commands`] then
    /// names only the last command of each coalesced run. If the newest state of a compacted
    /// burst fails to encode, fallback tries only the bounded retained candidates (the oldest
    /// and newest pending states, up to the queue compaction limit), so intermediate states
    /// dropped across multiple compactions are not retried.
    pub coalesce_edits: bool,
    /// Prune entries left by earlier runs when the recorder starts.
    pub retention: Option<Retention>,
}

/// How long, and how many, recovery entries no running session holds are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    /// An unheld entry whose newest file is older than this is removed.
    pub max_age: Duration,
    /// At most this many unheld entries are kept, the most recently written first.
    pub max_entries: usize,
}

/// Most folder entries a prune looks at, so a crowded folder cannot stall the worker.
const SCAN_LIMIT: usize = 16_384;

/// Remove entries in `dir` that no running session holds and that are older than
/// `retention.max_age` or beyond the newest `retention.max_entries`. Entries held by a running
/// session are never touched. Returns how many entries were removed; while another entry is
/// being opened or removed it removes nothing and does not wait.
pub fn prune(dir: &Path, retention: Retention) -> Result<usize, String> {
    prune_with_discovery(dir, retention, SCAN_LIMIT, |dir| {
        Ok(fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.map(|entry| entry.ok().map(|entry| entry.file_name())))
    })
}

/// Read and verify the versioned recovery header from `reader` within the bounded snapshot
/// header budget (`SNAPSHOT_OVERHEAD_BYTES` / `MAX_HEADER_BYTES`).
fn read_recovery_header_from(reader: impl Read) -> Result<Header, String> {
    let mut reader = reader.take(SNAPSHOT_OVERHEAD_BYTES as u64);
    let mut magic = [0; MAGIC.len()];
    reader.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if magic != MAGIC {
        return Err("not a recovery file".to_owned());
    }
    let payload = read_frame_bounded(&mut reader, MAX_HEADER_BYTES)?.ok_or_else(|| "header is missing".to_owned())?;
    serde_json::from_slice(&payload).map_err(|e| e.to_string())
}

/// Read and verify the versioned recovery header from a regular `.pc` or `.journal` file.
/// Returns `Ok(None)` when the path does not exist, `Ok(Some(header))` when the file is a regular
/// recovery envelope with a valid framed [`Header`], and `Err(())` for ordinary project archives,
/// symlinks, directories, unreadable files or any other non-recovery file.
fn read_recovery_header(path: &Path) -> Result<Option<Header>, ()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !metadata.is_file() {
        return Err(());
    }
    let file = File::open(path).map_err(|_| ())?;
    read_recovery_header_from(file).map(Some).map_err(|_| ())
}

/// Whether `id` in `dir` is provably a recovery entry created by this format.
/// Any present `.pc` or `.journal` file must carry a valid recovery header, and at least one of
/// them must exist; ordinary saved `.pc` project archives and unrelated files never match.
fn is_recovery_entry(dir: &Path, id: &str) -> bool {
    let Ok(pc) = read_recovery_header(&entry_file(dir, id, "pc")) else {
        return false;
    };
    let Ok(journal) = read_recovery_header(&entry_file(dir, id, "journal")) else {
        return false;
    };
    pc.is_some() || journal.is_some()
}

/// Observed modification times of a recovery entry's three known files (`.pc`, `.journal`,
/// `.lock`), together with their maximum for ranking and age checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryTimes {
    pc: Option<SystemTime>,
    journal: Option<SystemTime>,
    lock: Option<SystemTime>,
    newest: SystemTime,
}

#[cfg_attr(not(test), allow(unused_variables))]
fn entry_file_metadata(dir: &Path, path: &Path) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    if faults::fails(dir, faults::Fault::EntryMetadata) {
        return Err(std::io::Error::new(ErrorKind::PermissionDenied, "permission denied"));
    }
    fs::metadata(path)
}

/// Modification times of all three known files of recovery entry `id`, plus their maximum.
/// Returns `None` if a metadata error prevents determining any existing file's time, or if none
/// of the files exist, so retention leaves the entry in place rather than treating it as expired.
fn entry_times(dir: &Path, id: &str) -> Option<EntryTimes> {
    let mut times = [None; 3];
    let mut newest: Option<SystemTime> = None;
    for (slot, ext) in times.iter_mut().zip(["pc", "journal", "lock"]) {
        let path = entry_file(dir, id, ext);
        match entry_file_metadata(dir, &path) {
            Ok(metadata) => {
                let modified = metadata.modified().ok()?;
                *slot = Some(modified);
                newest = Some(newest.map_or(modified, |prev| prev.max(modified)));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(EntryTimes { pc: times[0], journal: times[1], lock: times[2], newest: newest? })
}

#[cfg(test)]
fn entry_modified(dir: &Path, id: &str) -> Option<SystemTime> {
    entry_times(dir, id).map(|times| times.newest)
}

#[cfg_attr(not(test), allow(unused_variables))]
fn open_candidate_lock(dir: &Path, path: &Path) -> std::io::Result<File> {
    #[cfg(test)]
    if faults::fails(dir, faults::Fault::LockPermission) {
        return Err(std::io::Error::new(ErrorKind::PermissionDenied, "permission denied"));
    }
    open_entry_lock(path, false)
}

/// Try to acquire candidate `id`'s `.lock` file without following symlinks.
/// Returns `Some(Some(file))` when locked, `Some(None)` when no `.lock` file exists, or `None`
/// when the entry is busy, non-regular (symlink/directory), or unreadable and should be skipped.
fn try_lock_candidate(dir: &Path, id: &str) -> Option<Option<File>> {
    let lock_path = entry_file(dir, id, "lock");
    match fs::symlink_metadata(&lock_path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Some(None),
        Ok(metadata) if metadata.file_type().is_file() => match open_candidate_lock(dir, &lock_path) {
            Ok(lock) if lock.try_lock().is_ok() => Some(Some(lock)),
            Ok(_) => None,
            Err(error) if error.kind() == ErrorKind::NotFound => Some(None),
            Err(_) => None,
        },
        Ok(_) | Err(_) => None,
    }
}

fn prune_with_discovery<I>(
    dir: &Path,
    retention: Retention,
    scan_limit: usize,
    discover: impl FnOnce(&Path) -> Result<I, String>,
) -> Result<usize, String>
where
    I: IntoIterator<Item = Option<std::ffi::OsString>>,
{
    prune_with_hooks(dir, retention, scan_limit, discover, |_| {}, |_| {})
}

fn prune_with_hooks<I>(
    dir: &Path,
    retention: Retention,
    scan_limit: usize,
    discover: impl FnOnce(&Path) -> Result<I, String>,
    mut on_scan: impl FnMut(&str),
    mut before_delete: impl FnMut(&str),
) -> Result<usize, String>
where
    I: IntoIterator<Item = Option<std::ffi::OsString>>,
{
    // A folder not created yet holds nothing; failing to create one is the recorder's to report.
    if !dir.is_dir() {
        return Ok(0);
    }
    let gate = match open_lock_file(&dir.join(".entries.lock")) {
        Ok(gate) => gate,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    if gate.try_lock().is_err() {
        return Ok(0);
    }
    let mut ids = std::collections::BTreeSet::new();
    for name in discover(dir)?.into_iter().take(scan_limit) {
        let Some(name) = name else { continue };
        let Some((id, ext)) = name.to_str().and_then(|name| name.rsplit_once('.')) else { continue };
        if !matches!(ext, "pc" | "journal" | "lock") || !valid_id(id) {
            continue;
        }
        ids.insert(id.to_owned());
    }
    // Pass 1 (scan): acquire each candidate's entry lock before reading its headers or timestamps,
    // then drop the lock immediately so retention holds at most one entry lock handle at a time.
    let mut entries: Vec<(EntryTimes, String)> = Vec::new();
    for id in ids {
        let Some(lock) = try_lock_candidate(dir, &id) else {
            continue;
        };
        on_scan(&id);
        if !is_recovery_entry(dir, &id) {
            continue;
        }
        let Some(times) = entry_times(dir, &id) else {
            continue;
        };
        drop(lock);
        entries.push((times, id));
    }
    // Pass 2 (rank): newest first by maximum modification time, tie-broken deterministically by id.
    entries.sort_by(|a, b| b.0.newest.cmp(&a.0.newest).then_with(|| b.1.cmp(&a.1)));
    let now = SystemTime::now();
    let (mut kept, mut removed) = (0, 0);
    // Pass 3 (delete): re-acquire each selected entry's lock under the folder gate, re-verify
    // its recovery header and each file's modification time, and skip it if it became busy or changed.
    for (recorded_times, id) in entries {
        let old = now.duration_since(recorded_times.newest).unwrap_or_default() > retention.max_age;
        if !old && kept < retention.max_entries {
            kept += 1;
            continue;
        }
        before_delete(&id);
        let Some(lock) = try_lock_candidate(dir, &id) else {
            continue;
        };
        if !is_recovery_entry(dir, &id) {
            continue;
        }
        let Some(current_times) = entry_times(dir, &id) else {
            continue;
        };
        if current_times != recorded_times {
            continue;
        }
        remove_unlocked(dir, &id, lock)?;
        removed += 1;
    }
    drop(gate);
    Ok(removed)
}

/// Delete entry `id`, whose lock (if it has one) the caller holds under the folder gate. The lock
/// is closed before its path is removed, as Windows requires.
fn remove_unlocked(dir: &Path, id: &str, lock: Option<File>) -> Result<(), String> {
    for ext in ["pc", "journal"] {
        remove_if_present(&entry_file(dir, id, ext))?;
    }
    match lock {
        Some(lock) => {
            drop(lock);
            remove_if_present(&entry_file(dir, id, "lock"))
        }
        None => Ok(()),
    }
}

/// Reserve lock-path creation and removal within this folder. The gate file stays in place:
/// closing an entry's lock before deleting it on Windows must not let another writer acquire
/// that old lock and then lose its path. Contention is refused without waiting.
fn entry_gate(dir: &Path) -> std::io::Result<File> {
    let gate = open_lock_file(&dir.join(".entries.lock"))?;
    gate.try_lock().map_err(|_| std::io::Error::other("Recovery entries are being opened or removed."))?;
    Ok(gate)
}

/// On Windows an owned entry lock cannot be unlinked underneath its recorder; when deleting,
/// close that handle first while still holding the folder gate.
fn open_entry_lock(path: &Path, fresh: bool) -> std::io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false).create_new(fresh);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x2).attributes(crate::FILE_ATTRIBUTE_HIDDEN);
    }
    options.open(path)
}

/// Delete only a lock path still owned by this recorder. Unix allows an outside process to
/// unlink an open file, so check identity before deleting anything. An overdue drain only exits.
fn discard_owned(mut journal: EditJournal, deadline: Instant) -> Result<(), String> {
    if journal.removed || Instant::now() >= deadline {
        return Ok(());
    }
    let dir = journal.dir.clone();
    let id = journal.id.clone();
    let gate = entry_gate(&dir).map_err(|e| e.to_string())?;
    let lock_path = entry_file(&dir, &id, "lock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let actual = fs::metadata(&lock_path).map_err(|e| e.to_string())?;
        let held = journal._lock.metadata().map_err(|e| e.to_string())?;
        if (actual.dev(), actual.ino()) != (held.dev(), held.ino()) {
            return Err("The recovery entry lock was replaced; leaving the entry alone.".into());
        }
    }
    if Instant::now() >= deadline {
        return Ok(());
    }
    // Unpublish the snapshot first. Keeping the gate across close/unlink prevents another
    // cooperating writer from opening the old entry lock path during Windows' handoff.
    drop(journal.file.take());
    remove_if_present(&entry_file(&dir, &id, "pc"))?;
    remove_if_present(&entry_file(&dir, &id, "journal"))?;
    drop(journal);
    let result = remove_if_present(&lock_path);
    drop(gate);
    result
}

/// Delete entry `id` in `dir`: its snapshot, journal and lock. Missing files are not an error.
/// The folder's `.entries.lock` gate stays. Refused, deleting nothing, when the id is invalid,
/// a running session holds the entry, or another entry is being opened or removed.
pub fn remove_entry(dir: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a recovery entry id."));
    }
    let _gate = match entry_gate(dir) {
        Ok(gate) => gate,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let lock_path = entry_file(dir, id, "lock");
    match open_entry_lock(&lock_path, false) {
        Ok(lock) => {
            lock.try_lock().map_err(|_| format!("Recovery entry `{id}` is in use by another session."))?;
            remove_unlocked(dir, id, Some(lock))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Remove a file, treating one that is already gone as removed.
fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(format!("{}: {error}", path.display())),
        _ => Ok(()),
    }
}

impl EditObserver for JournalRecorder {
    /// Queue a new history root; never blocks.
    fn opened(&self, project: Arc<Project>) {
        self.mailbox.push(Message::Opened(project));
    }

    /// Queue a committed edit; never blocks.
    fn committed(&self, edit: CommittedEdit) {
        self.mailbox.push(Message::Committed(edit));
    }
}

/// The worker: encode each reported state and write it to the entry, until the recorder is gone.
/// Errors are kept for the next `sync`. A state that cannot be encoded is skipped; the next
/// recorded edit carries its changes, or a new snapshot follows a root that failed.
fn record(dir: &Path, id: &str, mailbox: &Mailbox, wake: &impl Fn(), mut error: Option<String>) -> Result<(), String> {
    let mut journal: Option<EditJournal> = None;
    let discard_before = loop {
        let (message, fallbacks) = match mailbox.next_with_fallbacks() {
            Ok(item) => item,
            Err(stop) => break stop,
        };
        let full = matches!(&message, Message::Opened(_) | Message::Snapshot(_) | Message::Compacted { .. });
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
            Message::Compacted { project, fallbacks } => match encode_snapshot(project) {
                Ok((documents, packed)) => {
                    let result = write_snapshot(&mut journal, dir, id, documents, &packed);
                    if result.is_err()
                        && let Some(journal) = &mut journal
                    {
                        journal.file = None;
                    }
                    result
                }
                Err(encode_error) => {
                    if !record_fallback(&mut journal, dir, id, fallbacks, true)
                        && let Some(journal) = &mut journal
                    {
                        journal.file = None;
                    }
                    Err(encode_error)
                }
            },
            // An edit that cannot be encoded is folded into the next record, which starts from
            // the last state written; the journal closes itself after a failed write.
            Message::Committed(edit) if journal.as_ref().is_none_or(|journal| journal.file.is_none()) => {
                match encode_snapshot(edit.project.clone()) {
                    Ok((documents, packed)) => write_snapshot(&mut journal, dir, id, documents, &packed),
                    Err(encode_error) => {
                        let _ = record_fallback(&mut journal, dir, id, fallbacks, false);
                        Err(encode_error)
                    }
                }
            }
            Message::Committed(edit) => match encode_documents(edit.project.clone()) {
                Ok(documents) => write(&mut journal, dir, id, Some(&edit), documents),
                Err(encode_error) => {
                    let _ = record_fallback(&mut journal, dir, id, fallbacks, false);
                    Err(encode_error)
                }
            },
            Message::Snapshot(project) => encode_documents(project).and_then(|documents| match journal.as_mut() {
                Some(journal) => journal.snapshot(documents).map(drop),
                None => write(&mut journal, dir, id, None, documents),
            }),
        };
        let mut pending = mailbox.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = pending.fault.clone();
        match result {
            Err(message) => {
                error.get_or_insert_with(|| message.clone());
                pending.fault = Some(RecordingFault { removed: journal.as_ref().is_some_and(|journal| journal.removed), message });
            }
            Ok(()) if full => pending.fault = None,
            Ok(()) => {}
        }
        let changed = pending.fault != before;
        drop(pending);
        if changed {
            wake();
        }
    };
    if let (Some(journal), Some(deadline)) = (journal, discard_before) {
        discard_owned(journal, deadline)?;
    }
    Ok(())
}

/// After the newest coalesced or compacted state fails to encode, record the newest retained
/// earlier pending candidate that still encodes, leaving the caller to report the newest state's
/// error. Compacted bursts retain at most `PENDING_LIMIT` candidates (the oldest and newest
/// entries via [`bound_fallbacks`]), so intermediate states dropped across multiple compactions
/// are not retried. Compacted bursts always write a snapshot (`as_snapshot == true`) so an edit
/// after a compacted `Opened` is never appended to a pre-open journal.
fn record_fallback(journal: &mut Option<EditJournal>, dir: &Path, id: &str, fallbacks: Vec<Message>, as_snapshot: bool) -> bool {
    let needs_snapshot = as_snapshot || journal.as_ref().is_none_or(|journal| journal.file.is_none());
    for fallback in fallbacks.into_iter().rev() {
        let result = match fallback {
            Message::Sync(_) => continue,
            Message::Opened(project) | Message::Compacted { project, .. } => match encode_snapshot(project) {
                Ok((documents, packed)) => write_snapshot(journal, dir, id, documents, &packed),
                Err(_) => continue,
            },
            Message::Committed(edit) if needs_snapshot => match encode_snapshot(edit.project.clone()) {
                Ok((documents, packed)) => write_snapshot(journal, dir, id, documents, &packed),
                Err(_) => continue,
            },
            Message::Committed(edit) => match encode_documents(edit.project.clone()) {
                Ok(documents) => write(journal, dir, id, Some(&edit), documents),
                Err(_) => continue,
            },
            Message::Snapshot(project) => match journal.as_mut() {
                Some(journal) if !needs_snapshot => match encode_documents(project) {
                    Ok(documents) => journal.snapshot(documents).map(drop),
                    Err(_) => continue,
                },
                _ => match encode_snapshot(project) {
                    Ok((documents, packed)) => write_snapshot(journal, dir, id, documents, &packed),
                    Err(_) => continue,
                },
            },
        };
        return result.is_ok();
    }
    false
}

/// Encode and pack a project snapshot before staging it, so a manifest-limit packing failure is
/// classified like an encoding failure instead of an I/O write error.
fn encode_snapshot(project: Arc<Project>) -> Result<(Documents, Vec<u8>), String> {
    let documents = encode_documents(project)?;
    let packed = encode_packed_documents(documents.clone())?;
    Ok((documents, packed))
}

/// Write a pre-packed snapshot to `journal`, locking the entry first when none is open yet.
fn write_snapshot(journal: &mut Option<EditJournal>, dir: &Path, id: &str, documents: Documents, packed: &[u8]) -> Result<(), String> {
    match journal.as_mut() {
        Some(journal) => journal.rebase_packed(documents, packed),
        // The lock is kept even if this first snapshot fails; the next state tries again.
        None => journal.insert(EditJournal::lock(dir, id, true)?).rebase_packed(documents, packed),
    }
}

/// Record `documents` as the result of `edit`, or as a new snapshot when there is no edit. The
/// first state creates the entry, so it becomes the snapshot whatever produced it.
fn write(journal: &mut Option<EditJournal>, dir: &Path, id: &str, edit: Option<&CommittedEdit>, documents: Documents) -> Result<(), String> {
    match (journal.as_mut(), edit) {
        (Some(journal), Some(edit)) => journal.append(edit, documents),
        (Some(journal), None) => journal.rebase(documents),
        // The lock is kept even if this first snapshot fails; the next state tries again.
        (None, _) => journal.insert(EditJournal::lock(dir, id, true)?).rebase(documents),
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
