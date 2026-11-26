//! Exact authored circular copper arcs. Conductive consumers refuse these records until they
//! support the same circular contour; storing an arc never substitutes its endpoint chord.
use super::{setup::LayerPurpose, view::BoardView};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

pub mod geometry;

/// Replace the addressed board's exact arc records without running conductive validation.
pub const SET_ARCS: &str = "pcb.set_arc_tracks";
/// Storage bound independent of the straight-segment budget.
pub const MAX_ARCS: usize = 4096;
/// Shared refusal text for consumers without a circular copper contour.
pub const UNSUPPORTED: &str = "circular copper arcs are stored but unsupported by this geometry operation";

/// Direction from start to end in board coordinates (X right, Y down).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Clockwise,
    Counterclockwise,
}

impl<'de> Deserialize<'de> for Direction {
    /// Accept only the published string variants, not externally tagged maps.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "clockwise" => Ok(Self::Clockwise),
            "counterclockwise" => Ok(Self::Counterclockwise),
            value => Err(serde::de::Error::unknown_variant(value, &["clockwise", "counterclockwise"])),
        }
    }
}

/// A circular centerline with round ends and constant width. The circle's squared radius is
/// derived from `center` and `start`, never a second authored length. Equal endpoints are
/// refused: neither zero sweep nor full circles have an implicit interpretation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcTrack {
    pub id: Uuid,
    pub layer: Uuid,
    pub width_nm: i64,
    pub center: Point,
    pub start: Point,
    pub end: Point,
    pub direction: Direction,
}
impl ArcTrack {
    /// Exact squared radius in nm²; out-of-range center/start coordinates return an error.
    pub fn radius_squared_nm2(&self) -> Result<i128, String> {
        if !crate::wire::valid_point(&self.center) || !crate::wire::valid_point(&self.start) {
            return Err("arc coordinates are out of range".into());
        }
        Ok(squared_distance(self.center, self.start))
    }
    /// Reject invalid identities, dimensions, coordinates and sweeps. The end must lie within
    /// one nanometre of the start's circle. An integer inequality compares the exact radicals,
    /// including the inclusive boundary, without rounded radii or platform trigonometry.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.layer.is_nil()
            || !(super::track::MIN_WIDTH_NM..=super::track::MAX_WIDTH_NM).contains(&self.width_nm)
            || [self.center, self.start, self.end].iter().any(|p| !crate::wire::valid_point(p))
        {
            return Err("an arc needs non-nil identities, bounded points and a supported track width".into());
        }
        if self.center == self.start || self.center == self.end || self.start == self.end {
            return Err("an arc needs a nonzero radius and distinct ends; full circles are unsupported".into());
        }
        let (a, b) = (self.radius_squared_nm2()?, squared_distance(self.center, self.end));
        let (large, small) = (a.max(b), a.min(b));
        let delta = large - small;
        // Bounded coordinates are below ±1 m: the squared inequality fits i128.
        if delta > 1 && (delta - 1) * (delta - 1) > 4 * small {
            return Err("arc endpoint is more than 1 nm off its circle".into());
        }
        // Distinct ends on the same radial ray still have zero angular sweep.
        let (sx, sy) = (self.start.x_nm as i128 - self.center.x_nm as i128, self.start.y_nm as i128 - self.center.y_nm as i128);
        let (ex, ey) = (self.end.x_nm as i128 - self.center.x_nm as i128, self.end.y_nm as i128 - self.center.y_nm as i128);
        if sx * ey == sy * ex && sx * ex + sy * ey > 0 {
            return Err("an arc needs a nonzero angular sweep".into());
        }
        Ok(())
    }
}
/// Squared distance after coordinate validation; the ±1 m bounds fit i128.
fn squared_distance(a: Point, b: Point) -> i128 {
    let (x, y) = (a.x_nm as i128 - b.x_nm as i128, a.y_nm as i128 - b.y_nm as i128);
    x * x + y * y
}
/// Validate the complete arc list against the addressed board, including cross-copper IDs.
pub(crate) fn validate(project: &Project, arcs: &[ArcTrack]) -> Result<(), String> {
    if arcs.len() > MAX_ARCS {
        return Err(format!("a board can hold at most {MAX_ARCS} circular arcs"));
    }
    if arcs.is_empty() {
        return Ok(());
    }
    let mut ids: BTreeSet<_> = project.board.tracks.iter().map(|t| t.id).chain(project.board.vias.iter().map(|v| v.id)).collect();
    for arc in arcs {
        arc.validate()?;
        if !ids.insert(arc.id) {
            return Err("duplicate arc, track or via identity".into());
        }
        if !project.board.setup.as_ref().is_some_and(|s| s.layers().iter().any(|l| l.id == arc.layer && l.purpose == LayerPurpose::Copper)) {
            return Err("arcs need a copper layer of their board setup".into());
        }
    }
    Ok(())
}
impl<'a> BoardView<'a> {
    /// Exact authored arc records in UUID order, with no tessellation or inferred net membership.
    pub fn arc_tracks(self) -> &'a [ArcTrack] {
        &self.project().board.arcs
    }
    /// Refuse incomplete conductive results rather than silently omitting curved copper.
    pub fn require_linear_copper(self) -> Result<(), String> {
        self.require_no_arcs()?;
        self.require_no_teardrops()
    }
    /// Refuse circular copper when building a projection of the underlying straight objects.
    pub(super) fn require_no_arcs(self) -> Result<(), String> {
        if self.arc_tracks().is_empty() { Ok(()) } else { Err(UNSUPPORTED.into()) }
    }
    /// Refuse incomplete conductive results while teardrop intents are present.
    pub(super) fn require_no_teardrops(self) -> Result<(), String> {
        if self.teardrops().is_empty() { Ok(()) } else { Err(super::teardrop::UNSUPPORTED.into()) }
    }
}
impl Session {
    /// Replace exact records atomically after catalog, board and revision checks. A dry run
    /// returns the full before/after list without editing history; equal lists are a no-op.
    /// This storage operation makes no connectivity, clearance or manufacturing claim.
    pub(crate) fn set_arc_tracks(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.validate_project_command(SET_ARCS, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let mut arcs: Vec<ArcTrack> = serde_json::from_value(params["arcs"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        validate(project, &arcs).map_err(CommandError::InvalidParams)?;
        if !arcs.is_empty() && !project.board.teardrops.is_empty() {
            return Err(CommandError::Unavailable(super::teardrop::UNSUPPORTED.into()));
        }
        arcs.sort_by_key(|a| a.id);
        let before = &project.board.arcs;
        let changed = before != &arcs;
        let dry_run = params["dry_run"].as_bool().unwrap_or(false);
        let diff =
            json!({"board":target.board,"before":before,"after":arcs,"changed":changed,"dry_run":dry_run,"geometry_supported":arcs.is_empty()});
        if changed && !dry_run {
            let mut next = project.clone();
            Arc::make_mut(&mut next.board).arcs = arcs;
            self.change(next)?;
        }
        Ok(json!({"revision":self.revision(),"arc_tracks":diff}))
    }
}

/// Catalog vocabulary for full-list record replacement; an empty list removes all arcs.
pub(crate) fn schema() -> Value {
    json!({"type":"array","maxItems":MAX_ARCS,
        "description":"Integer nanometre fields require JSON integer tokens; fractional, exponent and negative-zero spellings are refused even where JSON Schema type:integer permits their mathematical value.",
        "items":{"type":"object","additionalProperties":false,
        "required":["id","layer","width_nm","center","start","end","direction"],
        "properties":{"id":{"type":"string","format":"uuid"},"layer":{"type":"string","format":"uuid"},
            "width_nm":{"type":"integer","minimum":super::track::MIN_WIDTH_NM,"maximum":super::track::MAX_WIDTH_NM},
            "center":crate::point_schema(),"start":crate::point_schema(),"end":crate::point_schema(),
            "direction":{"type":"string","enum":["clockwise","counterclockwise"]}}}})
}

#[cfg(test)]
mod tests;
