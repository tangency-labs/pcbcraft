//! Immutable native revision publication through verified directory handles.
use crate::bound::Directory;
#[cfg(test)]
mod tests;
use pcbcraft_engine::{
    authoring::publishing::Publication,
    library::sources::{MAX_FILE_BYTES, NativeSource},
};
use std::{fs::File, io::Write, path::Path};

/// Disk publication is distinct from subsequent catalog refresh. Once a final record is visible,
/// a sync failure is reported as uncertain and callers must refresh rather than publish again.
#[derive(Debug)]
pub enum Failure {
    /// No final revision was exposed; the draft may be prepared again after fixing the error.
    BeforeWrite(String),
    /// A complete revision is visible but durability is uncertain; verify that same revision.
    Published(String),
}
impl std::fmt::Display for Failure {
    /// Preserve whether a record may already exist when explaining retry behavior to a host.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeWrite(error) => f.write_str(error),
            Self::Published(error) => {
                write!(f, "Revision is published but durable confirmation failed: {error}. Refresh the library before retrying.")
            }
        }
    }
}

/// A process lock keeps simultaneous cooperating publishers from allocating different snapshots
/// of the same next revision. Crash/shutdown releases the OS lock; its empty file is reusable.
struct Lock(File);
impl Drop for Lock {
    /// Explicitly release the advisory lock on every staging and publication failure path.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Read an exact bounded manifest through the same no-follow directory binding used for writes.
pub fn read_manifest(root: &Path) -> Result<String, String> {
    let directory = Directory::open(root).map_err(|e| e.to_string())?;
    let bytes = directory.read("library.json", MAX_FILE_BYTES).map_err(|e| e.to_string())?.ok_or("Missing library manifest.")?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Recover durable confirmation of an already visible exact revision without writing another.
pub fn confirm(root: &Path, prepared: &impl Publication) -> Result<(), String> {
    let directory = Directory::open(root).map_err(|e| e.to_string())?;
    if read_manifest(root)? != prepared.manifest() {
        return Err("Destination manifest changed after publication.".into());
    }
    for (path, text) in prepared.documents() {
        let (parent, name) = path.split_once('/').ok_or("Invalid prepared document path.")?;
        let target = directory.child(parent, false).map_err(|e| e.to_string())?.ok_or("Published record directory is missing.")?;
        target
            .confirm_file(name, text.as_bytes())
            .map_err(|error| format!("Published revision could not be confirmed: {error}; inspect the destination before retrying."))?;
        target.sync().map_err(|e| e.to_string())?;
    }
    directory.sync().map_err(|e| e.to_string())
}

/// Resume the same captured revision after an uncertain worker outcome. An absent final record
/// can be published under its original name; an existing record must match exactly and is never replaced.
pub fn recover(root: &Path, prepared: &impl Publication) -> Result<(), Failure> {
    let directory = Directory::open(root).map_err(|e| Failure::BeforeWrite(e.to_string()))?;
    let (path, _) = prepared.documents().iter().next().ok_or_else(|| Failure::BeforeWrite("Missing prepared record.".into()))?;
    let (parent, name) = path.split_once('/').ok_or_else(|| Failure::BeforeWrite("Invalid prepared path.".into()))?;
    let present = directory
        .child(parent, false)
        .map_err(|e| Failure::Published(e.to_string()))?
        .map(|target| target.read(name, MAX_FILE_BYTES))
        .transpose()
        .map_err(|e| Failure::Published(e.to_string()))?
        .flatten()
        .is_some();
    if present { confirm(root, prepared).map_err(Failure::Published) } else { publish(root, prepared) }
}

/// Validate the destination again, then stage, sync and expose exactly one complete native record.
/// Existing revisions and unrelated files are never replaced, including raced-in destinations.
pub fn publish(root: &Path, prepared: &impl Publication) -> Result<(), Failure> {
    let before = |e: String| Failure::BeforeWrite(e);
    if !super::writable(root) {
        return Err(before("Choose a writable native library folder.".into()));
    }
    let directory = Directory::open(root).map_err(|e| before(e.to_string()))?;
    let lock = directory.lock_file(".library-publish.pcbcraft-lock").map_err(|e| before(e.to_string()))?;
    lock.try_lock().map_err(|e| before(format!("Another publisher is using this library: {e}")))?;
    let _lock = Lock(lock);
    let manifest = directory
        .read("library.json", MAX_FILE_BYTES)
        .map_err(|e| before(e.to_string()))?
        .ok_or_else(|| before("Destination manifest disappeared.".into()))?;
    if manifest != prepared.manifest().as_bytes() {
        return Err(before("Destination library changed; refresh before publishing.".into()));
    }
    let (mut documents, problems) = super::read_documents(root).map_err(before)?;
    let disk = NativeSource::load(&documents).map_err(before)?;
    let reference = prepared.reference();
    if disk.library() != reference.library || !disk.problems().is_empty() || !problems.is_empty() {
        return Err(before("Resolve the destination library's diagnostics before publishing.".into()));
    }
    let kind = prepared.kind();
    let latest =
        disk.items().iter().filter(|item| item.key.kind == kind && item.key.id == reference.id).map(|item| item.key.revision).max().unwrap_or(0);
    if latest > prepared.latest() {
        return Err(before("A later revision appeared on disk; refresh and review the publication warning before retrying.".into()));
    }
    let (path, text) = prepared.documents().iter().next().ok_or_else(|| before("No prepared document.".into()))?;
    if documents.contains_key(path) {
        return Err(before("Revision destination already exists; refresh before retrying.".into()));
    }
    documents.extend(prepared.documents().clone());
    pcbcraft_engine::library::sources::check_documents(&documents).map_err(before)?;
    let (parent, name) = path.split_once('/').ok_or_else(|| before("Invalid prepared document path.".into()))?;
    let target = directory.child(parent, true).map_err(|e| before(e.to_string()))?.ok_or_else(|| before("Missing item directory.".into()))?;
    // Root staging files are outside native item directories. A crash leaves inspectable bytes
    // without making the source invalid or preventing a retry from choosing another stage.
    let mut stage = None;
    for slot in 0..64 {
        let name = format!(".pcbcraft-{name}-{slot}.pending");
        match directory.create(&name) {
            Ok(file) => {
                stage = Some((name, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(before(error.to_string())),
        }
    }
    let (staged, mut file) = stage.ok_or_else(|| before("Too many interrupted publication stages; inspect the library folder.".into()))?;
    let write = file.write_all(text.as_bytes()).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write {
        let cleanup = directory.remove_file(&staged);
        return Err(before(format!("Could not stage revision: {error}; cleanup: {cleanup:?}")));
    }
    if let Err(error) = directory.publish_new(&staged, &target, name) {
        let cleanup = directory.remove_file(&staged);
        return Err(before(format!("Could not publish unused revision: {error}; cleanup: {cleanup:?}")));
    }
    let cleanup = directory.remove_file(&staged);
    target.sync().and_then(|_| directory.sync()).map_err(|e| Failure::Published(e.to_string()))?;
    if let Err(error) = cleanup {
        // The complete final file and directory are durable, so report this separately from save failure.
        log_cleanup(root, error);
    }
    Ok(())
}

/// Creation uses the same validated manifest and bounded installation as imported libraries.
pub fn create(root: &Path, manifest: &str) -> Result<NativeSource, String> {
    pcbcraft_engine::authoring::publishing::destination(manifest)?;
    let bundle = serde_json::json!({"format":"pcbcraft-library-bundle/1","documents":{"library.json":manifest}});
    super::install_bundle(&serde_json::to_vec(&bundle).map_err(|e| e.to_string())?, root)
}

/// A leftover staging hard link does not invalidate a confirmed immutable final record.
fn log_cleanup(root: &Path, error: std::io::Error) {
    eprintln!("Published library revision in {}, but could not remove staging link: {error}", root.display());
}
