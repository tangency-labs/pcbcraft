//! One fixed-geometry policy applies at the shared snapshot publication boundary.
use super::*;

/// Existing locked objects may change lock intent, but must retain all other fields and identity.
/// New copied objects may be translated and remain locked; replacing the whole project is separate.
pub(crate) fn protect_locked(before: &Project, after: &Project) -> Result<(), CommandError> {
    if std::sync::Arc::ptr_eq(&before.board, &after.board)
        || (!before.board.tracks.iter().any(|track| track.locked) && !before.board.vias.iter().any(|via| via.locked))
    {
        return Ok(());
    }
    let tracks: BTreeMap<_, _> = after.board.tracks.iter().map(|track| (track.id, track)).collect();
    let vias: BTreeMap<_, _> = after.board.vias.iter().map(|via| (via.id, via)).collect();
    for previous in before.board.tracks.iter().filter(|track| track.locked) {
        if !tracks.get(&previous.id).is_some_and(|next| {
            let mut geometry = (*next).clone();
            geometry.locked = previous.locked;
            geometry == *previous
        }) {
            return Err(CommandError::InvalidParams(format!("Track {} is locked; unlock it before editing or deleting it.", previous.id)));
        }
    }
    for previous in before.board.vias.iter().filter(|via| via.locked) {
        if !vias.get(&previous.id).is_some_and(|next| {
            let mut geometry = (*next).clone();
            geometry.locked = previous.locked;
            geometry == *previous
        }) {
            return Err(CommandError::InvalidParams(format!("Via {} is locked; unlock it before editing or deleting it.", previous.id)));
        }
    }
    Ok(())
}

impl Session {
    /// Resolve the complete selection before one undoable lock edit; duplicate/unknown IDs and stale targets fail atomically.
    pub(crate) fn set_copper_locked(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let tracks = ids(params, "tracks", |id| project.board.tracks.iter().any(|track| track.id == *id))?;
        let vias = ids(params, "vias", |id| project.board.vias.iter().any(|via| via.id == *id))?;
        if tracks.is_empty() && vias.is_empty() {
            return Err(CommandError::InvalidParams("Select at least one track or via to lock or unlock.".into()));
        }
        let locked: bool = param(params, "locked")?;
        if !project.board.tracks.iter().any(|track| tracks.contains(&track.id) && track.locked != locked)
            && !project.board.vias.iter().any(|via| vias.contains(&via.id) && via.locked != locked)
        {
            return Ok(());
        }
        let mut next = project.clone();
        let board = std::sync::Arc::make_mut(&mut next.board);
        for track in board.tracks.iter_mut().filter(|track| tracks.contains(&track.id)) {
            track.locked = locked;
        }
        for via in board.vias.iter_mut().filter(|via| vias.contains(&via.id)) {
            via.locked = locked;
        }
        self.change(next)
    }
}
