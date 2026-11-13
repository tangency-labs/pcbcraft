//! Filesystem-aware selection of a project's registered physical-data namespace.

use pcbcraft_engine::{
    Project,
    persistence::{Documents, ENTRY_POINT, MAX_DOCUMENTS, layout, project_board_path, project_layout_root},
};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::Path, sync::Arc};

const AREAS: [&str; 4] = ["boards", "builds", "documents", "outputs"];

/// Plan first-save folder and occurrence migration without writing files. Registered namespaces stay
/// fixed; new registrations avoid all unowned reserved areas and occupied board folders.
/// Archives pass no filesystem root. The returned Arc is unchanged when metadata already agrees.
pub fn prepare(snapshot: Arc<Project>, root: Option<&Path>, original: Option<&Documents>) -> Result<Arc<Project>, String> {
    let stored_root = original.and_then(|documents| documents.get(ENTRY_POINT)).map(|entry| project_layout_root(entry)).transpose()?.flatten();
    let mut next = if snapshot.layout_root().is_none() { snapshot.with_board_folders(".")? } else { Project::clone(&snapshot) };
    let mut chosen = stored_root.clone().or_else(|| next.layout_root().map(str::to_owned)).ok_or("missing project layout root")?;
    if stored_root.is_none()
        && let Some(root) = root
    {
        let occupied = names(root)?;
        let conflict = if chosen == "." { AREAS.iter().any(|area| contains(&occupied, area)) } else { contains(&occupied, &chosen) };
        if conflict {
            let id = snapshot.id().simple().to_string();
            chosen = (12..=32)
                .step_by(4)
                .map(|len| format!("pcbcraft-data-{}", &id[..len]))
                .find(|candidate| !contains(&occupied, candidate))
                .ok_or("No unused project data root is available.")?;
        }
    }
    next = next.with_layout_root(&chosen)?;
    next = next.with_sheet_occurrences()?;
    let previous_board = original.and_then(|documents| documents.get(ENTRY_POINT)).map(|entry| project_board_path(entry)).transpose()?.flatten();
    // An existing board keeps its stored segment. New board identities get their own segment,
    // even when Undo/Create replaced a previously saved board in the same project history.
    let registered_id = original
        .and_then(|documents| documents.get(ENTRY_POINT))
        .map(|entry| pcbcraft_engine::persistence::project_registered_board_id(entry))
        .transpose()?
        .flatten();
    let current_board = next.boards().next().map(|board| (board.name().to_owned(), board.target.board));
    if registered_id == next.board_target().map(|target| target.board) && stored_root.is_some() {
        if let Some(folder) = previous_board.as_deref().and_then(|path| path.rsplit('/').nth(1)) {
            next = next.with_board_folder(folder)?;
        }
    } else if let (Some(root), Some(board)) = (root, current_board) {
        let area = if chosen == "." { root.join("boards") } else { root.join(&chosen).join("boards") };
        let occupied = names(&area)?;
        if next.board_folder().is_some_and(|folder| contains(&occupied, folder)) {
            let folder = layout::folder_name(&board.0, board.1, &occupied)?;
            next = next.with_board_folder(&folder)?;
        }
    }
    if snapshot.has_sheet_occurrences() && next.layout_root() == snapshot.layout_root() && next.board_folder() == snapshot.board_folder() {
        Ok(snapshot)
    } else {
        Ok(Arc::new(next))
    }
}

/// Recheck a new namespace before staging and immediately before journal publication. Existing
/// registered storage stays authoritative; foreign folders and instance files cannot become owned.
pub(super) fn preflight_ownership(root: &Path, before: &Documents, after: &Documents) -> Result<(), String> {
    let prior = before.get(ENTRY_POINT).map(|entry| project_layout_root(entry)).transpose()?.flatten();
    let next = after.get(ENTRY_POINT).map(|entry| project_layout_root(entry)).transpose()?.flatten();
    let Some(next) = next else { return Ok(()) };
    if prior.as_deref() != Some(next.as_str()) {
        let occupied = names(root)?;
        let conflict = if next == "." { AREAS.iter().any(|area| contains(&occupied, area)) } else { contains(&occupied, &next) };
        if conflict {
            return Err("The selected layout root is occupied by unregistered files or directories. Save again to choose an unused root.".into());
        }
    }
    let old_path = before.get(ENTRY_POINT).map(|entry| project_board_path(entry)).transpose()?.flatten();
    let new_path = after.get(ENTRY_POINT).map(|entry| project_board_path(entry)).transpose()?.flatten();
    if new_path != old_path
        && let Some(path) = new_path
    {
        let folder = Path::new(&path).parent().ok_or("missing board folder")?;
        let area = folder.parent().ok_or("missing board area")?;
        let name = folder.file_name().and_then(|name| name.to_str()).ok_or("invalid board folder")?;
        if contains(&names(&root.join(area))?, name) {
            return Err("The selected board folder is occupied by unregistered data. Save again to choose an unused folder.".into());
        }
    }
    let old_instances = before.get(ENTRY_POINT).map(|entry| pcbcraft_engine::persistence::project_instances_path(entry)).transpose()?.flatten();
    let new_instances = after.get(ENTRY_POINT).map(|entry| pcbcraft_engine::persistence::project_instances_path(entry)).transpose()?.flatten();
    if old_instances != new_instances
        && let Some(path) = new_instances
    {
        let path = Path::new(&path);
        let parent = path.parent().ok_or("missing instance parent")?;
        if contains(&names(&root.join(parent))?, "instances.json") {
            return Err("The new instances.json path contains unregistered user data; move that file before saving.".into());
        }
    }
    Ok(())
}

/// Case-insensitive comparison covers portable names even on a case-sensitive host.
fn contains(names: &BTreeSet<String>, name: &str) -> bool {
    names.iter().any(|item| item.eq_ignore_ascii_case(name))
}

/// Enumerate only one bounded directory level, refusing symlinks and special directory roots.
fn names(path: &Path) -> Result<BTreeSet<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(format!("{}: not a regular layout directory", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(error.to_string()),
    }
    let mut names = BTreeSet::new();
    for (index, entry) in fs::read_dir(path).map_err(|error| error.to_string())?.enumerate() {
        if index >= MAX_DOCUMENTS * 2 {
            return Err("Too many directory entries to select project storage safely.".into());
        }
        names.insert(entry.map_err(|error| error.to_string())?.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod net_registry_tests;
#[cfg(test)]
mod no_connect_tests;
#[cfg(test)]
mod occurrence_tests;
