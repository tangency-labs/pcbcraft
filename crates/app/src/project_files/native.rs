//! Desktop file dialog, bounded read, and atomic publication, run by the file worker.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use pcbcraft_engine::{
    Project, UNTITLED,
    persistence::{
        Documents, ENTRY_POINT, Encoding, decode_documents, decode_file, decode_packed, encode_documents, encode_packed_documents, read_bytes,
    },
};

use super::{Outcome, SaveFormat, Saved, file_name_for};
use pcbcraft_storage as folder;

#[derive(Clone, Debug)]
pub(super) struct Target {
    path: PathBuf,
    bytes: Arc<[u8]>,
    encoding: Encoding,
    documents: Option<Arc<Documents>>,
    legacy: bool,
    needs_manifest: bool,
    added_manifest: bool,
}

impl Target {
    /// Distinguish local copies without deriving project or document identity from the path.
    pub(super) fn arrangement_location(&self) -> String {
        self.path.as_os_str().as_encoded_bytes().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The normalized folder, archive or legacy file that the worker successfully opened/saved.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Explain the next save when the open form needs migration or manifest generation.
    pub(super) fn open_notice(&self) -> String {
        if self.legacy {
            "Project opened · Save will ask for a project folder or .pc.zip destination".to_owned()
        } else if self.needs_manifest {
            "Project opened without a manifest · Save will add a verified manifest".to_owned()
        } else {
            "Project opened".to_owned()
        }
    }

    /// Whether saving needs a new destination because the project was opened from an older format.
    pub(super) fn legacy(&self) -> bool {
        self.legacy
    }

    /// Tell the editor whether this save added the archive's first manifest.
    pub(super) fn added_manifest(&self) -> bool {
        self.added_manifest
    }
}

/// Pick and fully validate a project before making it available to the editor.
pub(super) fn open(pick_folder: bool) -> Result<Outcome, String> {
    let dialog = rfd::FileDialog::new().add_filter("PCBCraft project", &["pc", "zip", "json"]);
    let Some(path) = (if pick_folder { dialog.pick_folder() } else { dialog.pick_file() }) else {
        return Ok(Outcome::Cancelled);
    };
    load(&path).map(Outcome::Opened)
}

/// Load a project from a chosen path without changing the active editor.
pub(super) fn load(path: &Path) -> Result<Saved, String> {
    let path = normalize(path)?;
    if path.file_name().is_some_and(|name| name == ENTRY_POINT)
        && let Some(root) = path.parent()
        && root.join(".pcbcraft-save").try_exists().map_err(|e| e.to_string())?
    {
        return load_folder(root);
    }
    if fs::symlink_metadata(&path).map_err(|e| e.to_string())?.is_dir() {
        return load_folder(&path);
    }
    // The extension is only a picker hint. A split-format header identifies a folder entry point.
    let bytes = read(&path).map_err(|error| format!("Could not read project: {error}"))?;
    if Encoding::detect(&bytes) == Encoding::Json {
        let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if matches!(
            header["format"].as_str(),
            Some(
                "pcbcraft-project/5"
                    | "pcbcraft-project/6"
                    | "pcbcraft-project/7"
                    | "pcbcraft-project/9"
                    | "pcbcraft-project/11"
                    | "pcbcraft-project/12"
                    | "pcbcraft-project/13"
            )
        ) {
            if path.file_name().is_none_or(|name| name != ENTRY_POINT) {
                return Err("A split project's entry point must be named project.pc.".to_owned());
            }
            return load_folder(path.parent().ok_or("Project has no parent folder")?);
        }
    }
    let encoding = Encoding::detect(&bytes);
    let (snapshot, documents, legacy, needs_manifest) = if encoding == Encoding::Zip {
        let packed = decode_packed(&bytes)?;
        (packed.project, packed.documents.map(Arc::new), packed.legacy, packed.needs_manifest)
    } else {
        (decode_file(&bytes)?.0, None, true, false)
    };
    Ok(Saved { snapshot, target: Target { path, bytes: bytes.into(), encoding, documents, legacy, needs_manifest, added_manifest: false } })
}

/// Read and cross-validate all folder documents before returning an adoptable snapshot.
fn load_folder(root: &Path) -> Result<Saved, String> {
    let documents = folder::load(root)?;
    let snapshot = decode_documents(&documents)?;
    Ok(Saved {
        snapshot,
        target: Target {
            path: root.to_owned(),
            bytes: Arc::from([]),
            encoding: Encoding::Json,
            documents: Some(Arc::new(documents)),
            legacy: false,
            needs_manifest: false,
            added_manifest: false,
        },
    })
}

/// Save a non-legacy target in place when no format is requested. Otherwise, including every
/// legacy target, ask for a new destination in the requested format (a folder by default).
/// Unchanged documents retain their bytes; dismissing the picker returns `Outcome::Cancelled`.
/// A first save suggests the project's name; one still named Untitled is saved under its chosen
/// folder or archive name (see [`name_for`]), and the outcome carries the snapshot it renamed.
pub(super) fn save(snapshot: Arc<Project>, previous: Option<(Arc<Project>, Target)>, format: Option<SaveFormat>) -> Result<Outcome, String> {
    if let Some((old, target)) = previous.as_ref().filter(|(_, target)| format.is_none() && !target.legacy) {
        return save_target(snapshot, Some(old), target.clone(), &mut |_| Ok(()));
    }
    let suggestion = match previous.as_ref().and_then(|(_, target)| target.path.file_name()).and_then(|name| name.to_str()) {
        Some(file) => stem(file).to_owned(),
        None => file_name_for(&snapshot.name),
    };
    let mut dialog = rfd::FileDialog::new();
    if let Some(parent) = previous.as_ref().and_then(|(_, target)| target.path.parent()) {
        dialog = dialog.set_directory(parent);
    }
    let format = format.unwrap_or_default();
    let path = match format {
        SaveFormat::Folder => dialog.set_title("Create project folder").set_file_name(suggestion).save_file(),
        SaveFormat::Archive => dialog.add_filter("Packed PCBCraft project", &["zip"]).set_file_name(format!("{suggestion}.pc.zip")).save_file(),
    };
    let Some(path) = path else {
        return Ok(Outcome::Cancelled);
    };
    save_new(snapshot, previous.as_ref(), format, path)
}

/// Publish to a destination the person chose, which must not exist yet, adding `.pc.zip` to an
/// archive name without it. An Untitled project saved for the first time takes the destination's
/// name (see [`name_for`]); the outcome then carries the snapshot it renamed. A failure leaves no
/// name change behind, since only a successful outcome is adopted.
pub(super) fn save_new(
    snapshot: Arc<Project>,
    previous: Option<&(Arc<Project>, Target)>,
    format: SaveFormat,
    mut path: PathBuf,
) -> Result<Outcome, String> {
    if format == SaveFormat::Archive && !path.to_string_lossy().ends_with(".pc.zip") {
        if path.extension().is_some_and(|extension| extension == "zip" || extension == "pc") {
            path.set_extension("pc.zip");
        } else {
            let mut name = path.file_name().ok_or("Missing project filename")?.to_os_string();
            name.push(".pc.zip");
            path.set_file_name(name);
        }
    }
    let path = normalize(&path)?;
    let (snapshot, renamed) = match name_for(&snapshot, previous.is_some(), &path) {
        Some(named) => (Arc::new(named), Some(snapshot)),
        None => (snapshot, None),
    };
    let mut target = Target {
        path,
        bytes: Arc::from([]),
        encoding: if format == SaveFormat::Archive { Encoding::Zip } else { Encoding::Json },
        documents: None,
        legacy: false,
        needs_manifest: false,
        added_manifest: false,
    };
    let documents = working_documents(&snapshot, previous.map(|(old, _)| old), previous.and_then(|(_, target)| target.documents.as_deref()))?;
    if target.encoding == Encoding::Zip {
        target.bytes = encode_packed_documents(documents.clone())?.into();
        publish(&target.path, None, &target.bytes, || Ok(()))?;
    } else {
        folder::save(&target.path, None, &documents, &mut |_| Ok(()))?;
    }
    target.documents = Some(Arc::new(documents));
    Ok(Outcome::Saved(Saved { snapshot, target }, renamed))
}

/// Strip a project extension from a folder or archive file name.
fn stem(file: &str) -> &str {
    file.strip_suffix(".pc.zip").or_else(|| file.strip_suffix(".pc")).or_else(|| file.strip_suffix(".zip")).unwrap_or(file)
}

/// Name an Untitled project after the destination of its first save, as SolidCraft does. A project
/// saved before, named by the person, or given a destination whose name isn't a valid project name
/// keeps its name, and `None` is returned.
fn name_for(snapshot: &Project, saved_before: bool, path: &Path) -> Option<Project> {
    if saved_before || snapshot.name != UNTITLED {
        return None;
    }
    snapshot.renamed(stem(path.file_name()?.to_str()?)).ok()
}

/// Keep exact source bytes for every logically unchanged document, including Save As conversion.
fn working_documents(snapshot: &Arc<Project>, previous: Option<&Arc<Project>>, original: Option<&Documents>) -> Result<Documents, String> {
    if previous.is_some_and(|old| Arc::ptr_eq(old, snapshot))
        && let Some(original) = original
    {
        return Ok(original.clone());
    }
    let mut documents = encode_documents(snapshot.clone())?;
    if let (Some(old), Some(original)) = (previous, original) {
        let canonical = encode_documents(old.clone())?;
        for (path, bytes) in &mut documents {
            if canonical.get(path) == Some(bytes)
                && let Some(original) = original.get(path)
            {
                bytes.clone_from(original);
            }
        }
    }
    Ok(documents)
}

/// Publish a folder or v2 archive while retaining unchanged input bytes. Comparing each old
/// canonical document preserves custom formatting in files unaffected by the current edit.
fn save_target(
    snapshot: Arc<Project>,
    previous: Option<&Arc<Project>>,
    mut target: Target,
    checkpoint: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    let unchanged = previous.is_some_and(|old| Arc::ptr_eq(old, &snapshot));
    if target.encoding == Encoding::Zip {
        let bytes = if unchanged && !target.needs_manifest && !target.legacy {
            target.bytes.clone()
        } else {
            let documents = working_documents(&snapshot, previous, target.documents.as_deref())?;
            let bytes = encode_packed_documents(documents.clone())?;
            target.documents = Some(Arc::new(documents));
            Arc::from(bytes)
        };
        publish(&target.path, previous.map(|_| target.bytes.as_ref()), &bytes, || checkpoint("staged"))?;
        target.added_manifest = target.needs_manifest;
        target.needs_manifest = false;
        target.legacy = false;
        target.bytes = bytes;
    } else {
        let documents = working_documents(&snapshot, previous, target.documents.as_deref())?;
        folder::save(&target.path, target.documents.as_deref(), &documents, checkpoint)?;
        target.documents = Some(Arc::new(documents));
    }
    Ok(Outcome::Saved(Saved { snapshot, target }, None))
}

/// Encode and publish the chosen snapshot. The staging callback runs after the
/// temporary file is flushed and before any replacement of the destination.
/// Migrated snapshots always encode the current schema, even with no user edits.
#[cfg(test)]
fn save_at(
    snapshot: Arc<Project>,
    path: PathBuf,
    expected: Option<Arc<[u8]>>,
    unchanged: bool,
    encoding: Encoding,
    after_staging: impl FnOnce() -> Result<(), String>,
) -> Result<Outcome, String> {
    let bytes = if unchanged && snapshot.upgraded_from().is_none() {
        expected.clone().ok_or("Missing saved project baseline")?
    } else {
        Arc::from(pcbcraft_engine::persistence::encode_file(Arc::clone(&snapshot), encoding)?)
    };
    publish(&path, expected.as_deref(), &bytes, after_staging)?;
    Ok(Outcome::Saved(
        Saved { snapshot, target: Target { path, bytes, encoding, documents: None, legacy: true, needs_manifest: false, added_manifest: false } },
        None,
    ))
}

/// Canonicalize the parent directory so application instances use the same lock for the destination.
/// The result is also what Recent projects show and store, so a Windows target comes back in its
/// plain `C:\…` spelling, not `\\?\C:\…`, whenever Windows reads that spelling as the same file
/// (#154). Otherwise, for example `CON.pc.zip` or a name ending in a dot, it keeps `\\?\`.
fn normalize(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("Project file has no parent directory")?;
    let name = path.file_name().ok_or("Project file has no name")?;
    // Check the whole target, not just the parent: the file name decides whether the plain
    // spelling is safe.
    fs::canonicalize(parent).map(|parent| pcbcraft_storage::plain_path_if_equivalent(parent.join(name))).map_err(|error| error.to_string())
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
/// then atomically publish. Preserve Unix permissions on replacement and leave an unchanged file untouched.
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
    let lock = pcbcraft_storage::open_lock_file(&parent.join(lock_name)).map_err(|error| format!("Could not open save lock: {error}"))?;
    lock.try_lock().map_err(|error| format!("Could not lock project for saving: {error}"))?;

    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    staged.write_all(bytes).and_then(|()| staged.as_file().sync_all()).map_err(|error| format!("Could not stage project: {error}"))?;
    after_staging()?;
    #[cfg(unix)]
    if expected.is_some() {
        let metadata = fs::symlink_metadata(path).map_err(|error| format!("Could not read project permissions: {error}"))?;
        if !metadata.is_file() {
            return Err("Project destination is no longer a regular file.".to_owned());
        }
        staged
            .as_file()
            .set_permissions(metadata.permissions())
            .and_then(|()| staged.as_file().sync_all())
            .map_err(|error| format!("Could not preserve project permissions: {error}"))?;
    }
    verify()?;
    if expected.is_some() {
        staged.persist(path).map_err(|error| format!("Could not replace project: {error}"))?;
    } else {
        staged.persist_noclobber(path).map_err(|error| format!("Could not publish project: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;
