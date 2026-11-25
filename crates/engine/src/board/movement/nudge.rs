use super::{BTreeMap, BTreeSet, CommandError, MovePlan, Point, Project, Uuid, dragged_ends, invalid};
use std::sync::Arc;

/// Merge constraints from different moving objects without choosing an arbitrary winner at a junction.
fn merge(merged: &mut Point, original: Point, proposal: Point) -> Result<(), CommandError> {
    if proposal != original {
        if *merged != original && *merged != proposal {
            return Err(invalid("This move asks a copper junction to follow in two different ways."));
        }
        *merged = proposal;
    }
    Ok(())
}

impl MovePlan {
    /// Adapt the normal move candidate using the same straight-track drag as D. Selected neighbors
    /// move together rather than following one another. Unselected endpoints merge proposals from
    /// tracks, carried pads, and vias with an explicit coincident endpoint. A selected leg held at
    /// both ends refuses the entire step; one held end pivots, as with D. The caller validates
    /// locks and all initial direct copper contacts on the resulting candidate.
    pub(super) fn reshape_tracks(&self, next: &mut Project, delta: Point) -> Result<BTreeSet<Uuid>, CommandError> {
        if self.items.tracks.is_empty() && self.items.vias.is_empty() {
            return Ok(BTreeSet::new());
        }
        let view = self.source.board_view(self.target).ok_or_else(|| invalid("Board is unavailable."))?;
        let tracks = view.tracks();
        let ends: BTreeMap<_, _> = tracks.iter().enumerate().map(|(i, track)| (track.id, i)).collect();
        let stationary: Vec<_> = tracks.iter().filter(|t| !self.items.tracks.contains(&t.id)).collect();
        let board = Arc::make_mut(&mut next.board);
        for dragged in tracks.iter().filter(|t| self.items.tracks.contains(&t.id)) {
            let edits = dragged_ends(dragged, stationary.iter().copied(), (delta.x_nm, delta.y_nm));
            if edits.is_empty() {
                // Both ends meet the middle of unselected copper. Refuse the whole step rather than
                // leave this leg behind while the rest of the selection moves.
                return Err(invalid("A selected track is held at both ends by other copper and cannot move. Select the copper holding it too."));
            }
            for (id, start, end) in edits {
                let Some(&index) = ends.get(&id) else { return Err(invalid("Unknown copper neighbor.")) };
                let (original, moved) = (&tracks[index], &mut board.tracks[index]);
                if id == dragged.id {
                    if let Some((carry_start, carry_end)) = self.carried.get(&id)
                        && (*carry_start && start != moved.start || *carry_end && end != moved.end)
                    {
                        return Err(invalid("A held track end cannot follow its moving pad."));
                    }
                    (moved.start, moved.end) = (start, end);
                } else {
                    merge(&mut moved.start, original.start, start)?;
                    merge(&mut moved.end, original.end, end)?;
                }
            }
        }
        let moved_vias: BTreeMap<_, _> = view
            .vias()
            .iter()
            .zip(&board.vias)
            .filter(|(old, new)| old.at != new.at)
            .map(|(old, new)| ((old.at.x_nm, old.at.y_nm), new.at))
            .collect();
        for (original, moved) in tracks.iter().zip(&mut board.tracks) {
            for (old, at) in [(original.start, &mut moved.start), (original.end, &mut moved.end)] {
                if let Some(&new) = moved_vias.get(&(old.x_nm, old.y_nm)) {
                    if self.items.tracks.contains(&original.id) && *at != new {
                        return Err(invalid("A held track end cannot follow its moving via."));
                    }
                    merge(at, old, new)?;
                }
            }
        }
        Ok(tracks.iter().zip(&board.tracks).filter_map(|(old, new)| (old != new).then_some(old.id)).collect())
    }
}

#[cfg(test)]
mod tests;
