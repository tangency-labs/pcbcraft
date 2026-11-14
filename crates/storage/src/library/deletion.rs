//! Confirmed per-item removal stages exact revisions reversibly before invoking system trash.
use crate::bound::Directory;
use pcbcraft_engine::library::sources::{Documents, ItemRemoval, Key, MAX_FILE_BYTES, NativeSource};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// A bounded immutable disk capture, held with the root's verified directory handle.
#[derive(Clone, Debug)]
pub struct Plan {
    removal: ItemRemoval,
    documents: Documents,
    directory: Directory,
    token: String,
}
impl Plan {
    /// Inspect the captured target without allowing callers to change the reviewed file set.
    pub fn removal(&self) -> &ItemRemoval {
        &self.removal
    }

    /// Confirmation names this exact disk snapshot rather than a mutable browser selection.
    pub fn token(&self) -> &str {
        &self.token
    }
}
/// Plan only a writable, non-reserved, diagnostic-free native folder; no records are moved.
pub fn prepare(root: &Path, key: &Key) -> Result<Plan, String> {
    if !super::writable(root) {
        return Err("Choose a writable library folder.".into());
    }
    let directory = Directory::open(root).map_err(|e| e.to_string())?;
    if !directory.matches_path(root).map_err(|e| e.to_string())? {
        return Err("Library folder changed.".into());
    }
    let (documents, problems) = super::read_documents(root)?;
    let mut source = NativeSource::load(&documents)?;
    source.add_adapter_problems(problems);
    if pcbcraft_engine::authoring::publishing::reserved_destination(source.library()) {
        return Err("Built-in library records cannot be deleted.".into());
    }
    let removal = source.item_removal(key)?;
    let token = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(
                &serde_json::json!({"root":directory.identity().map_err(|e|e.to_string())?,"kind":removal.kind,"id":removal.id,"documents":documents})
            )
            .map_err(|e| e.to_string())?
        )
    );
    let plan = Plan { removal, documents, directory, token };
    verify(root, &plan)?;
    Ok(plan)
}
/// Revalidate all reviewed native bytes through captured parents. New records also invalidate
/// the review, so a raced-in dependent cannot silently bypass the source's relationship check.
fn verify(root: &Path, plan: &Plan) -> Result<(), String> {
    if !plan.directory.matches_path(root).map_err(|e| e.to_string())? {
        return Err("Library folder moved or was replaced; refresh before deleting.".into());
    }
    let (current, problems) = super::read_documents(root)?;
    if !problems.is_empty() || current != plan.documents {
        return Err("Library folder changed; review deletion again.".into());
    }
    for (path, text) in &plan.documents {
        let bytes = if let Some((parent, name)) = path.split_once('/') {
            plan.directory.child(parent, false).map_err(|e| e.to_string())?.ok_or("Native item directory disappeared.")?.read(name, MAX_FILE_BYTES)
        } else {
            plan.directory.read(path, MAX_FILE_BYTES)
        }
        .map_err(|e| e.to_string())?;
        if bytes.as_deref() != Some(text.as_bytes()) {
            return Err("Library contents changed through the captured directory.".into());
        }
    }
    Ok(())
}
/// Move every reviewed revision of one identity to system trash. Recovery bytes are never
/// permanently deleted: errors restore unused source paths or retain a named recovery folder.
pub fn execute(root: &Path, plan: &Plan) -> Result<NativeSource, String> {
    execute_with(root, plan, super::trash_directory)
}
/// A testable boundary around the platform trash service retains the same lock and rollback
/// behavior for real operations, including a partial/uncertain platform outcome.
fn execute_with(root: &Path, plan: &Plan, trash: impl FnOnce(&Path) -> Result<(), String>) -> Result<NativeSource, String> {
    let lock = plan.directory.lock_file(".library-publish.pcbcraft-lock").map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|e| format!("Another publisher is using this library: {e}"))?;
    let _lock = super::publishing::Lock(lock);
    verify(root, plan)?;
    let recovery = tempfile::Builder::new().prefix("pcbcraft-deleted-part-").tempdir_in(root).map_err(|e| e.to_string())?.keep();
    let archive = Directory::open(&recovery).map_err(|e| e.to_string())?;
    let recovery_name = recovery.file_name().and_then(|name| name.to_str()).ok_or("Invalid recovery directory name.")?;
    let captured_archive = plan.directory.child(recovery_name, false).map_err(|e| e.to_string())?.ok_or("Recovery directory moved.")?;
    if !plan.directory.matches_path(root).map_err(|e| e.to_string())?
        || archive.identity().map_err(|e| e.to_string())? != captured_archive.identity().map_err(|e| e.to_string())?
    {
        return Err("Library changed while preparing recovery; no records were moved.".into());
    }
    let mut manifest = archive.create("library.json").map_err(|e| e.to_string())?;
    manifest.write_all(plan.documents["library.json"].as_bytes()).and_then(|_| manifest.sync_all()).map_err(|e| e.to_string())?;
    drop(manifest);
    let mut moved = Vec::new();
    let staged = (|| {
        for path in &plan.removal.files {
            let (parent, name) = path.split_once('/').ok_or("Invalid native record path.")?;
            let source = plan.directory.child(parent, false).map_err(|e| e.to_string())?.ok_or("Native item directory disappeared.")?;
            let target = archive.child(parent, true).map_err(|e| e.to_string())?.ok_or("Could not create the recovery record directory.")?;
            source.rename(name, &target, name).map_err(|e| e.to_string())?;
            moved.push(Moved { source, archive: target.clone(), parent: parent.into(), name: name.into() });
            let expected = plan.documents.get(path).ok_or("Missing reviewed native bytes.")?;
            target.confirm_file(name, expected.as_bytes()).map_err(|e| e.to_string())?;
        }
        for moved in &moved {
            moved.source.sync().and_then(|_| moved.archive.sync()).map_err(|e| e.to_string())?;
        }
        archive.sync().and_then(|_| plan.directory.sync()).map_err(|e| e.to_string())?;
        if !plan.directory.matches_path(root).map_err(|e| e.to_string())? {
            return Err("Library moved during deletion.".into());
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = staged {
        return Err(restore(&archive, &moved, &recovery, error));
    }
    // Windows handles prohibit deleting their captured directories; Unix is the qualified path.
    drop(archive);
    if let Err(error) = trash(&recovery) {
        return match Directory::open(&recovery) {
            Ok(archive) => Err(restore(&archive, &moved, &recovery, error)),
            Err(_) => Err(format!("Trash completion is uncertain: {error}. Refresh the library before retrying.")),
        };
    }
    if recovery.exists() {
        let archive = Directory::open(&recovery).map_err(|e| e.to_string())?;
        return Err(restore(&archive, &moved, &recovery, "System trash did not remove the recovery folder".into()));
    }
    super::load(root).map_err(|e| format!("Part records were moved to trash, but refreshing the library failed: {e}"))
}
/// Never overwrite a new source file during rollback. A failed restore preserves the private
/// recovery directory for inspection instead of allowing a TempDir destructor to remove data.
struct Moved {
    source: Directory,
    archive: Directory,
    parent: String,
    name: String,
}
/// Restore only unused source names, keeping raced-in replacements and durable recovery copies.
fn restore(archive: &Directory, moved: &[Moved], recovery: &PathBuf, error: String) -> String {
    let mut retained = false;
    for moved in moved.iter().rev() {
        if moved.archive.publish_new(&moved.name, &moved.source, &moved.name).is_err()
            || moved.source.sync().is_err()
            || moved.archive.remove_file(&moved.name).is_err()
        {
            retained = true;
        }
    }
    if !retained {
        let parents: std::collections::BTreeSet<_> = moved.iter().map(|m| m.parent.as_str()).collect();
        for parent in parents {
            if archive.remove_dir(parent).is_err() {
                retained = true;
            }
        }
        if !retained {
            let _ = archive.remove_file("library.json");
            let _ = std::fs::remove_dir(recovery);
        }
    }
    if retained {
        format!("{error}. Recovery records are retained at {}. Refresh before retrying.", recovery.display())
    } else {
        format!("{error}. Reviewed records were restored; refresh before retrying.")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pcbcraft_engine::{
        authoring::publishing::Publication,
        library::{parts::Reference, sources::Catalog},
        part_authoring::{Record, ReferenceField, Session},
    };
    use std::sync::Arc;
    /// Build original records through the production authoring and immutable publisher, with
    /// two revisions of the target and a separate item that deletion must leave untouched.
    #[allow(clippy::unwrap_used, clippy::panic)]
    fn fixture() -> (tempfile::TempDir, PathBuf, Key) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let manifest = pcbcraft_engine::authoring::publishing::new_library("delete-test", "Delete test", "MIT OR Apache-2.0").unwrap();
        super::super::publishing::create(&root, &manifest).unwrap();
        let catalog = Catalog::bundled();
        let record = Record::new_part(None)
            .unwrap()
            .select_reference(
                &ReferenceField::NewPackage { package: "physical".into() },
                Some(Reference { library: "basic-components".into(), id: "header-1x02-254".into(), revision: 3 }),
                &catalog,
            )
            .unwrap();
        let mut session = Session::new(record, false).unwrap();
        let first = session.prepare(&catalog, &manifest, 0, false, false).unwrap();
        super::super::publishing::publish(&root, &first).unwrap();
        session.confirm(&first);
        let mut record = session.record().clone();
        let Record::Part(part) = &mut record else { panic!() };
        part.name = "Reviewed physical part".into();
        session.replace(session.revision(), record).unwrap();
        let second = session.prepare(&catalog, &manifest, 1, false, false).unwrap();
        super::super::publishing::publish(&root, &second).unwrap();
        let unrelated = Session::new(Record::new_part(None).unwrap(), false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
        super::super::publishing::publish(&root, &unrelated).unwrap();
        let r = second.reference();
        (temp, root, Key { library: r.library, id: r.id, revision: r.revision, kind: pcbcraft_engine::library::sources::Kind::Part })
    }
    /// A successful removal includes all target revisions but no shared dependency or other item.
    #[test]
    fn deletion_is_source_local_and_keeps_unrelated_records() {
        let (_temp, root, key) = fixture();
        let before = super::super::read_documents(&root).unwrap().0;
        let plan = prepare(&root, &key).unwrap();
        assert_eq!(plan.removal.files.len(), 2);
        let after = execute_with(&root, &plan, |archive| {
            assert_eq!(std::fs::read_dir(archive).unwrap().count(), 2);
            std::fs::remove_dir_all(archive).map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(after.items().len(), 1);
        assert!(after.items().iter().all(|item| item.key.id != key.id));
        let actual = super::super::read_documents(&root).unwrap().0;
        for (path, bytes) in before.iter().filter(|(path, _)| !plan.removal.files.contains(path)) {
            assert_eq!(&actual[path], bytes);
        }
    }
    /// Trash failure restores exact native bytes, while source edits after review reject before
    /// the trash callback runs. The shared publisher lock also refuses overlap rather than waiting.
    #[test]
    fn failed_trash_changed_files_and_publication_lock_preserve_data() {
        let (_temp, root, key) = fixture();
        let before = super::super::read_documents(&root).unwrap().0;
        let plan = prepare(&root, &key).unwrap();
        assert!(execute_with(&root, &plan, |_| Err("Simulated unavailable trash".into())).unwrap_err().contains("restored"));
        assert_eq!(super::super::read_documents(&root).unwrap().0, before);
        let lock = plan.directory.lock_file(".library-publish.pcbcraft-lock").unwrap();
        lock.try_lock().unwrap();
        assert!(execute_with(&root, &plan, |_| panic!("must not trash")).unwrap_err().contains("publisher"));
        drop(lock);
        let path = &plan.removal.files[0];
        std::fs::write(root.join(path), format!("{}\n", before[path])).unwrap();
        assert!(execute_with(&root, &plan, |_| panic!("must not trash")).unwrap_err().contains("changed"));
    }
    /// Rollback never overwrites a raced-in replacement. Original bytes remain in the private
    /// recovery directory when restoring that source name would destroy newer content.
    #[test]
    fn rollback_collision_retains_recovery_and_never_overwrites() {
        let (_temp, root, key) = fixture();
        let plan = prepare(&root, &key).unwrap();
        let collision = root.join(&plan.removal.files[0]);
        let error = execute_with(&root, &plan, |_| {
            std::fs::write(&collision, "external replacement").unwrap();
            Err("Simulated trash failure".into())
        })
        .unwrap_err();
        assert_eq!(std::fs::read_to_string(collision).unwrap(), "external replacement");
        assert!(error.contains("Recovery records are retained"));
        let recovery = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.file_name().unwrap().to_str().unwrap().starts_with("pcbcraft-deleted-part-"))
            .unwrap();
        assert_eq!(super::super::load(&recovery).unwrap().items().len(), 1);
    }
    /// Root replacement and record symlinks cannot redirect a reviewed operation to another folder.
    #[cfg(unix)]
    #[test]
    fn replaced_root_and_symlinked_record_are_rejected() {
        let (temp, root, key) = fixture();
        let plan = prepare(&root, &key).unwrap();
        let moved = temp.path().join("moved");
        std::fs::rename(&root, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &root).unwrap();
        assert!(execute_with(&root, &plan, |_| panic!("must not trash")).is_err());
        std::fs::remove_file(&root).unwrap();
        std::fs::rename(&moved, &root).unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, "preserved").unwrap();
        let file = root.join(&plan.removal.files[0]);
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        assert!(execute_with(&root, &plan, |_| panic!("must not trash")).is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "preserved");
    }
    /// Native dependency checks also run against disk, independent of the UI catalog cache.
    #[test]
    fn orderable_reference_blocks_generic_deletion() {
        let (_temp, root, key) = fixture();
        let mut catalog = Catalog::bundled();
        catalog.install("local", Arc::new(super::super::load(&root).unwrap())).unwrap();
        let reference = Reference { library: key.library.clone(), id: key.id.clone(), revision: key.revision };
        let mut record = Record::new_orderable(&catalog, reference).unwrap();
        let Record::Orderable(m) = &mut record else { panic!() };
        m.manufacturer = "Test maker".into();
        m.mpn = "REFERENCES-TARGET".into();
        let manifest = super::super::publishing::read_manifest(&root).unwrap();
        let prepared = Session::new(record, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
        super::super::publishing::publish(&root, &prepared).unwrap();
        assert!(prepare(&root, &key).unwrap_err().contains("referenced"));
        let mut other = Catalog::bundled();
        other.install("parts", Arc::new(super::super::load(&root).unwrap())).unwrap();
        assert!(other.item_removal("parts", &key).unwrap_err().contains("referenced"));
        let mpn = Key {
            library: prepared.reference().library,
            id: prepared.reference().id,
            revision: 1,
            kind: pcbcraft_engine::library::sources::Kind::ManufacturerPart,
        };
        let mpn_plan = prepare(&root, &mpn).unwrap();
        assert_eq!(mpn_plan.removal.files.len(), 1);
    }
    /// Review tokens bind the actual directory and item, even when folder contents are identical.
    #[test]
    fn tokens_bind_root_and_target() {
        let (_temp, root, key) = fixture();
        let first = prepare(&root, &key).unwrap();
        let other = super::super::load(&root).unwrap().items().iter().find(|item| item.key.id != key.id).unwrap().key.clone();
        assert_ne!(first.token(), prepare(&root, &other).unwrap().token());
        let copy = root.parent().unwrap().join("copy");
        std::fs::create_dir(&copy).unwrap();
        for (path, text) in &first.documents {
            let destination = copy.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, text).unwrap();
        }
        assert_ne!(first.token(), prepare(&copy, &key).unwrap().token());
    }
    /// A dependent in another loaded library blocks deletion too, while built-ins and shared
    /// assets remain outside the removal contract.
    #[test]
    fn catalog_blocks_cross_source_references_and_builtin_assets() {
        let (temp, root, key) = fixture();
        let mut catalog = Catalog::bundled();
        catalog.install("local", Arc::new(super::super::load(&root).unwrap())).unwrap();
        let removal = catalog.item_removal("local", &key).unwrap();
        let reference = Reference { library: key.library.clone(), id: key.id.clone(), revision: key.revision };
        let mut record = Record::new_orderable(&catalog, reference).unwrap();
        let Record::Orderable(m) = &mut record else { panic!() };
        m.manufacturer = "Other maker".into();
        m.mpn = "DEPENDENT".into();
        let manifest = pcbcraft_engine::authoring::publishing::new_library("other-delete-test", "Other", "MIT OR Apache-2.0").unwrap();
        let other = temp.path().join("other");
        super::super::publishing::create(&other, &manifest).unwrap();
        let publication = Session::new(record, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
        super::super::publishing::publish(&other, &publication).unwrap();
        catalog.install("other", Arc::new(super::super::load(&other).unwrap())).unwrap();
        assert!(catalog.check_item_removal(&removal).unwrap_err().contains("referenced"));
        assert!(catalog.item_removal("local", &key).unwrap_err().contains("referenced"));
        let asset = catalog
            .source_items("bundled-basic-components")
            .iter()
            .find(|item| item.key.kind == pcbcraft_engine::library::sources::Kind::Footprint)
            .unwrap()
            .key
            .clone();
        assert!(catalog.item_removal("bundled-basic-components", &asset).is_err());
        assert!(super::super::load(&root).unwrap().item_removal(&asset).is_err());
    }
    /// Exercise the real Linux trash adapter using only a test-owned isolated XDG trash folder.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the host gio trash service"]
    fn isolated_gio_trash_preserves_native_recovery_records() {
        let (temp, root, key) = fixture();
        let plan = prepare(&root, &key).unwrap();
        let xdg = temp.path().join("xdg");
        std::fs::create_dir(&xdg).unwrap();
        let after = execute_with(&root, &plan, |archive| {
            let mut command = std::process::Command::new("gio");
            command.arg("trash").arg("--").arg(archive).env("XDG_DATA_HOME", &xdg);
            let status = super::super::wait_for_trash(&mut command, std::time::Duration::from_secs(10))?;
            if status.success() { Ok(()) } else { Err("gio failed".into()) }
        })
        .unwrap();
        assert_eq!(after.items().len(), 1);
        let trashed = std::fs::read_dir(xdg.join("Trash/files")).unwrap().next().unwrap().unwrap().path();
        let restored = super::super::load(&trashed).unwrap();
        assert_eq!(restored.items().len(), 2);
        for path in &plan.removal.files {
            assert_eq!(std::fs::read_to_string(trashed.join(path)).unwrap(), plan.documents[path]);
        }
    }
}
