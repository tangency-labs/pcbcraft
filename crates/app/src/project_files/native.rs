//! Desktop file dialog, bounded read, and atomic publication, run by the file worker.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use pcbcraft_engine::{
    Project,
    persistence::{decode, encode, read_bytes},
};

use super::{Outcome, Saved};

#[derive(Clone, Debug)]
pub(super) struct Target {
    path: PathBuf,
    bytes: Arc<[u8]>,
}

/// Pick and fully validate a project before making it available to the editor.
pub(super) fn open() -> Result<Outcome, String> {
    let Some(path) = rfd::FileDialog::new().add_filter("PCBCraft project", &["pc"]).pick_file() else {
        return Ok(Outcome::Cancelled);
    };
    let path = normalize(&path)?;
    let bytes = read(&path).map_err(|error| format!("Could not read project: {error}"))?;
    let snapshot = decode(&bytes)?;
    Ok(Outcome::Opened(Saved { snapshot, target: Target { path, bytes: bytes.into() } }))
}

/// Publish a snapshot after checking the destination against the last known bytes.
pub(super) fn save(snapshot: Arc<Project>, previous: Option<(Arc<Project>, Target)>) -> Result<Outcome, String> {
    let (path, expected, unchanged) = if let Some((old, target)) = previous {
        (target.path, Some(target.bytes), Arc::ptr_eq(&old, &snapshot))
    } else {
        let Some(mut path) = rfd::FileDialog::new().add_filter("PCBCraft project", &["pc"]).set_file_name("project.pc").save_file() else {
            return Ok(Outcome::Cancelled);
        };
        if path.extension().is_none() {
            path.set_extension("pc");
        }
        let path = normalize(&path)?;
        (path, None, false)
    };
    let bytes = if unchanged { expected.clone().ok_or("Missing saved project baseline")? } else { Arc::from(encode(Arc::clone(&snapshot))?) };
    publish(&path, expected.as_deref(), &bytes)?;
    Ok(Outcome::Saved(Saved { snapshot, target: Target { path, bytes } }))
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
fn publish(path: &Path, expected: Option<&[u8]>, bytes: &[u8]) -> Result<(), String> {
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
    verify()?;
    if expected.is_some() {
        staged.persist(path).map_err(|error| format!("Could not replace project: {error}"))?;
    } else {
        staged.persist_noclobber(path).map_err(|error| format!("Could not publish project: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Competing writes from the same baseline cannot both replace the project.
    #[test]
    fn one_writer_publishes_against_a_shared_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pc");
        publish(&path, None, b"base").unwrap();
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                publish(&path, Some(b"base"), b"first")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                publish(&path, Some(b"base"), b"second")
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.into_iter().filter(Result::is_ok).count(), 1);
        let winner = fs::read(&path).unwrap();
        assert!(winner == b"first" || winner == b"second");
        assert!(publish(&path, Some(b"base"), b"third").is_err());
        assert_eq!(fs::read(&path).unwrap(), winner);
    }

    /// A worker completion marks only the retained snapshot saved, even when editing continued.
    #[test]
    fn a_late_edit_is_not_marked_saved() {
        use crate::project_files::{Job, ProjectFiles, Update};
        use pcbcraft_engine::Session;
        use serde_json::json;

        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Original"})).unwrap();
        let saved = session.snapshot().unwrap();
        let revision = session.revision();
        let bytes: Arc<[u8]> = encode(saved.clone()).unwrap().into();
        session.apply("project.rename", &json!({"name":"Late edit"})).unwrap();
        let (send, receiver) = std::sync::mpsc::channel();
        send.send(Ok(Outcome::Saved(Saved { snapshot: saved, target: Target { path: PathBuf::from("test.pc"), bytes: bytes.clone() } }))).unwrap();
        let mut files = ProjectFiles { saved: None, job: Some(Job { receiver, revision }) };
        assert!(matches!(files.poll(&mut session, &egui::Context::default()), Some(Update::Message(_))));
        assert_eq!(decode(&bytes).unwrap().name, "Original");
        assert_eq!(session.project().unwrap().name, "Late edit");
        assert!(files.dirty(&session));
        session.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert!(!files.dirty(&session));
    }

    /// A no-op preserves the original bytes and a new save cannot clobber an existing file.
    #[test]
    fn unchanged_and_first_saves_do_not_overwrite_unexpected_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pc");
        let initial = b"{  \"format\": \"illustrative\"  }\n";
        publish(&path, None, initial).unwrap();
        publish(&path, Some(initial), initial).unwrap();
        assert!(publish(&path, None, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), initial);
    }
}
