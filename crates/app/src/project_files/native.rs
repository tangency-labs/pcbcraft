//! Desktop file dialog, bounded read, and atomic publication, run by the file worker.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use pcbcraft_engine::{
    Project,
    persistence::{Encoding, decode_file, encode_file, read_bytes},
};

use super::{Outcome, Saved};

#[derive(Clone, Debug)]
pub(super) struct Target {
    path: PathBuf,
    bytes: Arc<[u8]>,
    encoding: Encoding,
}

/// Pick and fully validate a project before making it available to the editor.
pub(super) fn open() -> Result<Outcome, String> {
    let Some(path) = rfd::FileDialog::new().add_filter("PCBCraft project", &["pc"]).pick_file() else {
        return Ok(Outcome::Cancelled);
    };
    load(&path).map(Outcome::Opened)
}

/// Load a project from a chosen path without changing the active editor.
fn load(path: &Path) -> Result<Saved, String> {
    let path = normalize(path)?;
    let bytes = read(&path).map_err(|error| format!("Could not read project: {error}"))?;
    let (snapshot, encoding) = decode_file(&bytes)?;
    Ok(Saved { snapshot, target: Target { path, bytes: bytes.into(), encoding } })
}

/// Save in the existing file’s encoding, or pick a destination for the requested encoding.
pub(super) fn save(snapshot: Arc<Project>, previous: Option<(Arc<Project>, Target)>, encoding: Encoding) -> Result<Outcome, String> {
    let (path, expected, unchanged, encoding) = if let Some((old, target)) = previous {
        (target.path, Some(target.bytes), Arc::ptr_eq(&old, &snapshot), target.encoding)
    } else {
        let Some(mut path) = rfd::FileDialog::new().add_filter("PCBCraft project", &["pc"]).set_file_name("project.pc").save_file() else {
            return Ok(Outcome::Cancelled);
        };
        if path.extension().is_none() {
            path.set_extension("pc");
        }
        let path = normalize(&path)?;
        (path, None, false, encoding)
    };
    save_at(snapshot, path, expected, unchanged, encoding, || Ok(()))
}

/// Encode and publish the chosen snapshot. The staging callback runs after the
/// temporary file is flushed and before any replacement of the destination.
fn save_at(
    snapshot: Arc<Project>,
    path: PathBuf,
    expected: Option<Arc<[u8]>>,
    unchanged: bool,
    encoding: Encoding,
    after_staging: impl FnOnce() -> Result<(), String>,
) -> Result<Outcome, String> {
    let bytes =
        if unchanged { expected.clone().ok_or("Missing saved project baseline")? } else { Arc::from(encode_file(Arc::clone(&snapshot), encoding)?) };
    publish(&path, expected.as_deref(), &bytes, after_staging)?;
    Ok(Outcome::Saved(Saved { snapshot, target: Target { path, bytes, encoding } }))
}

/// Canonicalize the parent directory so application instances use the same lock for the destination.
fn normalize(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("Project file has no parent directory")?;
    let name = path.file_name().ok_or("Project file has no name")?;
    fs::canonicalize(parent).map(|parent| parent.join(name)).map_err(|error| error.to_string())
}

/// Read only regular files and enforce the size bound even if the file grows after opening.
fn read(path: &Path) -> io::Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "project path is not a regular file"));
    }
    read_bytes(File::open(path)?)
}

/// Treat a missing destination as an empty baseline and propagate other failures.
fn existing(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not check destination: {error}")),
    }
}

/// Serialize writers using a persistent sibling lock. Stage, flush, check the destination again,
/// then atomically publish. An unchanged file retains its bytes and metadata.
fn publish(path: &Path, expected: Option<&[u8]>, bytes: &[u8], after_staging: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let verify = || {
        if existing(path)?.as_deref() != expected {
            let message = if expected.is_some() {
                "Project file changed on disk. Use Save As to keep your edits in a separate file."
            } else {
                "A file already exists at that destination. Choose a new filename."
            };
            return Err(message.to_owned());
        }
        Ok(())
    };
    if expected == Some(bytes) {
        return verify();
    }
    let parent = path.parent().ok_or("Project file has no parent directory")?;
    let name = path.file_name().ok_or("Project file has no name")?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(".pcbcraft-lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(lock_name))
        .map_err(|error| format!("Could not open save lock: {error}"))?;
    lock.try_lock().map_err(|error| format!("Could not lock project for saving: {error}"))?;

    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    staged.write_all(bytes).and_then(|()| staged.as_file().sync_all()).map_err(|error| format!("Could not stage project: {error}"))?;
    after_staging()?;
    verify()?;
    if expected.is_some() {
        staged.persist(path).map_err(|error| format!("Could not replace project: {error}"))?;
    } else {
        staged.persist_noclobber(path).map_err(|error| format!("Could not publish project: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
