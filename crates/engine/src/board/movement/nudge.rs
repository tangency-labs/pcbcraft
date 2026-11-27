use super::{BTreeMap, BTreeSet, CommandError, MovePlan, Point, Project, Uuid, dragged_ends, invalid};
use crate::board::track::Track;
use geo_index::rtree::{RTree, RTreeBuilder, RTreeIndex, sort::HilbertSort};
use std::sync::Arc;

/// A copper capsule fits in its centerline envelope expanded by its own radius. Including both
/// radii admits every pair that can touch, including exact tangencies and odd-nanometre widths.
fn copper_bounds(track: &Track) -> [f64; 4] {
    let (a, b) = (track.start, track.end);
    let radius = track.width_nm as f64 / 2.;
    [a.x_nm.min(b.x_nm) as f64 - radius, a.y_nm.min(b.y_nm) as f64 - radius, a.x_nm.max(b.x_nm) as f64 + radius, a.y_nm.max(b.y_nm) as f64 + radius]
}

/// Immutable source geometry: one packed index per step, never rebuilt as the candidate moves.
struct StationaryTracks<'a> {
    tracks: Vec<&'a Track>,
    tree: Option<RTree<f64>>,
}

impl<'a> StationaryTracks<'a> {
    fn new(tracks: &'a [Track], selected: &BTreeSet<Uuid>) -> Result<Self, CommandError> {
        if selected.is_empty() {
            return Ok(Self { tracks: Vec::new(), tree: None });
        }
        let tracks: Vec<_> = tracks.iter().filter(|t| !selected.contains(&t.id)).collect();
        let count = u32::try_from(tracks.len()).map_err(|_| invalid("Too many stationary tracks to nudge this selection."))?;
        let tree = (count != 0).then(|| {
            let mut builder = RTreeBuilder::<f64>::new(count);
            for track in &tracks {
                let [x_min, y_min, x_max, y_max] = copper_bounds(track);
                builder.add(x_min, y_min, x_max, y_max);
            }
            #[cfg(test)]
            tests::record_index_build(tracks.len());
            builder.finish::<HilbertSort>()
        });
        Ok(Self { tracks, tree })
    }

    fn candidates(&self, dragged: &Track) -> impl Iterator<Item = &'a Track> + '_ {
        let [x_min, y_min, x_max, y_max] = copper_bounds(dragged);
        let mut hits = self.tree.as_ref().map_or_else(Vec::new, |tree| tree.search(x_min, y_min, x_max, y_max));
        // Keep source order; dragged_ends still makes the exact layer and geometry decisions.
        hits.sort_unstable();
        #[cfg(test)]
        tests::record_query(hits.len());
        hits.into_iter().map(|i| self.tracks[i as usize])
    }
}

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
        let stationary = StationaryTracks::new(tracks, &self.items.tracks)?;
        let board = Arc::make_mut(&mut next.board);
        for dragged in tracks.iter().filter(|t| self.items.tracks.contains(&t.id)) {
            let edits = dragged_ends(dragged, stationary.candidates(dragged), (delta.x_nm, delta.y_nm));
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
