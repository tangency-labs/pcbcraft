//! One immutable source and atomic translation for an explicit mixed board selection.
use super::{clipboard::BoardSelection, drawing::Drawings, view::BoardTarget};
use crate::{CommandError, Project, Session, schematic::Point};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Translate selected footprints, graphics, zones, tracks and vias in one revision-checked edit.
pub const MOVE_ITEMS: &str = "pcb.move_items";

/// Immutable geometry authority for a translation preview. Pad contacts are captured once;
/// previews are disposable and do not certify copper validity or publish history.
#[derive(Debug)]
pub struct MovePlan {
    source: Arc<Project>,
    target: BoardTarget,
    items: BoardSelection,
    carried: BTreeMap<Uuid, (bool, bool)>,
}
/// Report invalid group geometry without changing source authority.
fn invalid(message: impl ToString) -> CommandError {
    CommandError::InvalidParams(message.to_string())
}
impl MovePlan {
    /// Capture existing IDs on the explicit board and refuse empty, foreign or locked targets.
    /// Circular copper is refused before preparing a partial move.
    pub fn new(source: Arc<Project>, target: BoardTarget, items: BoardSelection) -> Result<Self, CommandError> {
        let view = source.board_view(target).ok_or_else(|| invalid("Unknown board move target."))?;
        view.require_linear_copper().map_err(CommandError::Unavailable)?;
        if items.is_empty()
            || view.drawings().iter().filter(|d| items.drawings.contains(&d.id)).count() != items.drawings.len()
            || view.zones().iter().filter(|z| items.zones.contains(&z.id)).count() != items.zones.len()
            || view.tracks().iter().filter(|t| items.tracks.contains(&t.id)).count() != items.tracks.len()
            || view.vias().iter().filter(|v| items.vias.contains(&v.id)).count() != items.vias.len()
        {
            return Err(invalid("Select existing objects on this board to move."));
        }
        super::transform::require_unlocked_placements(&source, items.footprints.iter().map(|id| view.placement(id))).map_err(
            |error| match error {
                // A missing footprint uses the same invalid-selection error as other missing move targets.
                CommandError::NotFound(_) => invalid("Select existing objects on this board to move."),
                error => error,
            },
        )?;
        let carried = super::track::carried_ends(view, &items.footprints);
        if view.tracks().iter().any(|t| t.locked && (items.tracks.contains(&t.id) || carried.contains_key(&t.id)))
            || view.vias().iter().any(|v| v.locked && items.vias.contains(&v.id))
        {
            return Err(invalid("Unlock the affected copper before moving this selection."));
        }
        Ok(Self { source, target, items, carried })
    }
    /// Translate from the original coordinates, never from a previous preview. Selected tracks
    /// move rigidly once; unselected track ends on selected pads follow the existing carry policy.
    /// Out-of-range coordinates refuse the whole disposable candidate.
    pub fn at(&self, delta: Point) -> Result<Arc<Project>, CommandError> {
        if delta == (Point { x_nm: 0, y_nm: 0 }) {
            return Ok(self.source.clone());
        }
        let shift = |p: Point| -> Result<Point, CommandError> {
            let x_nm = p.x_nm.checked_add(delta.x_nm).ok_or_else(|| invalid("Move is outside the coordinate range."))?;
            let y_nm = p.y_nm.checked_add(delta.y_nm).ok_or_else(|| invalid("Move is outside the coordinate range."))?;
            let p = Point { x_nm, y_nm };
            if !crate::wire::valid_point(&p) {
                return Err(invalid("Move is outside the coordinate range."));
            }
            Ok(p)
        };
        let mut next = self.source.as_ref().clone();
        let drawings = if self.items.drawings.is_empty() {
            None
        } else {
            let view = self.source.board_view(self.target).ok_or_else(|| invalid("Board is unavailable."))?;
            let mut data = Drawings::capture(view);
            for d in data.items.iter_mut().filter(|d| self.items.drawings.contains(&d.id)) {
                d.points = d.points.iter().copied().map(shift).collect::<Result<_, _>>()?;
            }
            Some(data)
        };
        let board = Arc::make_mut(&mut next.board);
        for f in board.footprints.iter_mut().filter(|f| self.items.footprints.contains(&format!("footprint-{}", f.id))) {
            let at = shift(Point { x_nm: f.x_nm, y_nm: f.y_nm })?;
            (f.x_nm, f.y_nm) = (at.x_nm, at.y_nm);
        }
        if let Some(drawings) = drawings {
            board.drawings = Some(drawings);
            board.outline = None;
        }
        for z in board.zones.iter_mut().filter(|z| self.items.zones.contains(&z.id)) {
            z.points = z.points.iter().copied().map(shift).collect::<Result<_, _>>()?;
        }
        for t in &mut board.tracks {
            let selected = self.items.tracks.contains(&t.id);
            let (start, end) = self.carried.get(&t.id).copied().unwrap_or_default();
            if selected || start {
                t.start = shift(t.start)?;
            }
            if selected || end {
                t.end = shift(t.end)?;
            }
        }
        for v in board.vias.iter_mut().filter(|v| self.items.vias.contains(&v.id)) {
            v.at = shift(v.at)?;
        }
        Ok(Arc::new(next))
    }
}
impl Session {
    /// Rebuild against the requested revision and validate the complete translated candidate,
    /// including locked geometry, references and shorts, before one shared history publication.
    pub(crate) fn move_board_items(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let source = self.snapshot().ok_or_else(|| invalid("No project is open."))?;
        let target = source.resolve_board(params["board"].as_str())?;
        let items = BoardSelection::parse(&params["items"]).ok_or_else(|| invalid("Invalid board selection."))?;
        let delta = Point {
            x_nm: params["dx_nm"].as_i64().ok_or_else(|| invalid("Invalid X displacement."))?,
            y_nm: params["dy_nm"].as_i64().ok_or_else(|| invalid("Invalid Y displacement."))?,
        };
        let plan = MovePlan::new(source.clone(), target, items)?;
        let candidate = plan.at(delta)?;
        if Arc::ptr_eq(&candidate, &source) {
            return Ok(());
        }
        let mut next = candidate.as_ref().clone();
        let (mut tracks, mut vias) = (next.board.tracks.clone(), next.board.vias.clone());
        super::track::validate(&next, &mut tracks, &mut vias).map_err(invalid)?;
        super::track::protect_locked(&source, &next)?;
        let affected: BTreeSet<_> = plan.items.tracks.union(&plan.items.vias).copied().chain(plan.carried.keys().copied()).collect();
        if (!affected.is_empty() || !plan.items.footprints.is_empty())
            && let Some((a, b)) = super::track::short_for_copper_edit(&next, &affected, &plan.items.footprints).map_err(invalid)?
        {
            return Err(invalid(format!("This move would connect nets {a} and {b}.")));
        }
        {
            let board = Arc::make_mut(&mut next.board);
            board.tracks = tracks;
            board.vias = vias;
        }
        let next = crate::clipboard::validate_candidate(next)?;
        self.change_with_native_poses(next, plan.items.footprints.iter().filter_map(|id| crate::schematic::wiring::parse_id(id, "footprint-")))
    }
}

#[cfg(test)]
mod tests;
