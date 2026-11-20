//! Clearance-aware route planning around fixed copper.
//!
//! [`walkaround`] plans one single-layer route from a head point toward a target over a captured
//! [`ClearanceQuery`]: horizontal, vertical and 45° legs (or horizontal and vertical only) that
//! go round existing pads, tracks and vias, locked or not, with the effective width, pair
//! clearances and board-edge clearance. It never edits the board. Its points feed the existing
//! `pcb.add_tracks` command or copper preparation job, which revalidate and publish them in one
//! undo step. Every kept leg is confirmed by the same exact probe and outline predicates that
//! DRC uses. The search is deterministic and bounded by a probe budget and a cancellation flag.
//! Vias, arcs and zones are not planned here. [`shove`] moves existing tracks and vias out of
//! the way of a new route or dragged tracks instead of going round them.
use super::{clearance::ClearanceQuery, view::BoardTarget, work::Source};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

mod hull;
mod posture;
mod preparation;
mod search;
pub mod shove;
#[cfg(test)]
mod shove_tests;
#[cfg(test)]
mod tests;
mod walkaround;
pub use preparation::{PlanningContext, PlanningJob};

pub use posture::Posture;

/// Plan a route around existing copper without editing the board.
pub const PLAN_ROUTE: &str = "pcb.plan_route";
/// Plan a shove of existing copper out of the way of a new route or dragged tracks, without editing.
pub const PLAN_SHOVE: &str = "pcb.plan_shove";
/// Add a route or drag tracks, shoving existing copper out of the way, in one undo step.
pub const SHOVE_TRACKS: &str = "pcb.shove_tracks";
/// Exact probes one plan may charge when the caller gives no budget.
pub const DEFAULT_BUDGET: usize = 4096;
/// The largest accepted probe budget.
pub const MAX_BUDGET: usize = 65_536;

/// One planning request on one copper layer. `previous` is the caller's last result for the
/// same gesture, if any: an unchanged, still-legal result is kept as is, and otherwise the walk
/// prefers going round on the same side, so nearly equivalent routes do not alternate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub head: Point,
    pub target: Point,
    pub layer: Uuid,
    /// Original gesture contact and copper layer when `head` has advanced to a fixed vertex.
    /// None derives membership from `head`; contact unions never replace this starting intent.
    pub origin: Option<(Point, Uuid)>,
    pub width_nm: i64,
    pub posture: Posture,
    /// Start with the elbow's other leg first.
    pub flipped: bool,
    pub previous: Vec<Point>,
}

/// How a plan ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// The points run from the head to the target.
    Reached,
    /// No legal way round was found; the points end at the last legal point. When the head
    /// itself is not legal, the points are the head alone and nothing can be added.
    Blocked { last_legal: Point },
    /// The probe budget or a search bound ran out first; the points are the head alone.
    Incomplete,
    /// The caller canceled; the points are the head alone.
    Canceled,
    /// The query no longer describes the session's current board; nothing was planned.
    Stale,
}

/// A planned route. Consecutive points are distinct; fewer than two points mean nothing to add.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Plan {
    pub points: Vec<Point>,
    pub outcome: Outcome,
    /// Query address of the copper that stopped a blocked route, absent for the board edge.
    pub blocker: Option<String>,
    /// Exact probes charged, never more than the budget.
    pub probes: usize,
}

/// Plan a route over a captured board. Validates the request first: bounded endpoints and previous hints, a supported
/// width and a copper layer of the query's board; an invalid request charges nothing. Copper
/// the head touches lends the route its membership and island, so the route may join it; all
/// other copper, including copper of unknown membership, is an obstacle at its pair clearance.
/// The result is a pure function of the query, request and budget unless `cancel` returns true.
pub fn walkaround(query: &ClearanceQuery, request: &Request, budget: usize, cancel: &dyn Fn() -> bool) -> Result<Plan, CommandError> {
    validate(query, request, budget)?;
    search::complete(query, request, budget, cancel)
}

/// Reject malformed requests before any search work or asynchronous capture.
fn validate(query: &ClearanceQuery, request: &Request, budget: usize) -> Result<(), CommandError> {
    if !crate::wire::valid_point(&request.head)
        || !crate::wire::valid_point(&request.target)
        || request.origin.is_some_and(|(point, _)| !crate::wire::valid_point(&point))
    {
        return Err(CommandError::InvalidParams("Route points must be bounded board coordinates.".into()));
    }
    if request.previous.len() > super::track::MAX_PATH_POINTS || request.previous.iter().any(|point| !crate::wire::valid_point(point)) {
        return Err(CommandError::InvalidParams("Previous routes must contain at most 256 bounded board points.".into()));
    }
    super::geometry::Contour::segment(request.head, request.target, request.width_nm)?;
    if request.width_nm < query.authority().rules().values.min_track_width_nm {
        return Err(CommandError::InvalidParams("Track width cannot weaken the board minimum.".into()));
    }
    if !(1..=MAX_BUDGET).contains(&budget) {
        return Err(CommandError::InvalidParams(format!("A route budget must be from 1 to {MAX_BUDGET} probes.")));
    }
    Ok(())
}

impl Session {
    /// Plan over `query` only while it still describes this session's current board; an edit,
    /// Undo/Redo, reopen or another session's query gives [`Outcome::Stale`] without probing.
    /// Callers adopting the result later must check the query's authority again.
    pub fn plan_route(&self, query: &ClearanceQuery, request: &Request, budget: usize, cancel: &dyn Fn() -> bool) -> Result<Plan, CommandError> {
        if !query.authority().is_current(self) {
            return Ok(Plan { points: Vec::new(), outcome: Outcome::Stale, blocker: None, probes: 0 });
        }
        walkaround(query, request, budget, cancel)
    }

    /// Answer `pcb.plan_route` without changing revision, history or saved bytes. An omitted
    /// width uses the head contact's net-class preference or the board's preferred width; an
    /// explicit width must satisfy board and applicable class minimums. A given `expected_revision` that
    /// is not current gives a stale outcome instead of a plan.
    pub(crate) fn route_plan_query(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let (head, goal, layer): (Point, Point, Uuid) = (field(params, "from")?, field(params, "to")?, field(params, "layer")?);
        let width_nm = route_width(view, head, layer, params)?;
        let posture: Posture = params.get("posture").map(|_| field(params, "posture")).transpose()?.unwrap_or_default();
        let flipped = params.get("flipped").and_then(Value::as_bool).unwrap_or(false);
        let previous: Vec<Point> = params.get("previous").map(|_| field(params, "previous")).transpose()?.unwrap_or_default();
        let budget = params.get("budget").map(|_| field(params, "budget")).transpose()?.unwrap_or(DEFAULT_BUDGET);
        let request = Request { head, target: goal, layer, origin: None, width_nm, posture, flipped, previous };
        let stale = params.get("expected_revision").is_some_and(|revision| revision.as_u64() != Some(self.revision()));
        let plan = if stale {
            Plan { points: Vec::new(), outcome: Outcome::Stale, blocker: None, probes: 0 }
        } else {
            let query = self.clearance_query_with_rules(target, &json!({}))?;
            self.plan_route(&query, &request, budget, &|| false)?
        };
        Ok(json!({"revision":self.revision(),"route_plan":{
            "board":target.board,"layer":layer,"width_nm":width_nm,"posture":posture,"flipped":flipped,"budget":budget,
            "points":plan.points,"outcome":plan.outcome,"blocker":plan.blocker,"probes":plan.probes
        }}))
    }
}

/// The width of a new route from `head`: an explicit `width_nm` must satisfy the head contact's
/// net-class limits; an omitted one uses that class's preference or the board's preferred width.
fn route_width(view: super::view::BoardView<'_>, head: Point, layer: Uuid, params: &Value) -> Result<i64, CommandError> {
    let explicit_width: Option<i64> = params.get("width_nm").map(|_| field(params, "width_nm")).transpose()?;
    Ok(match view.route_rules_at(head, Some(layer))?.filter(|policy| policy.has_dimensions()) {
        Some(policy) => policy.track_width(explicit_width)?,
        None => explicit_width.unwrap_or(view.effective_rules().values.preferred_track_width_nm),
    })
}

/// Read a shove from command parameters: the board, the request, the route width (zero for a
/// drag) and the probe budget. Exactly one of `points` (a new route) or `tracks` (dragged
/// tracks with new ends) is required.
fn shove_request(project: &Project, params: &Value) -> Result<(BoardTarget, shove::Request, i64, usize), CommandError> {
    let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
    let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let layer: Uuid = field(params, "layer")?;
    let budget = params.get("budget").map(|_| field(params, "budget")).transpose()?.unwrap_or(shove::DEFAULT_BUDGET);
    let (shover, width_nm) = match (params.get("points"), params.get("tracks")) {
        (Some(_), None) => {
            let points: Vec<Point> = field(params, "points")?;
            let head = *points.first().ok_or_else(|| CommandError::InvalidParams("A shoved route needs at least two points.".into()))?;
            let width_nm = route_width(view, head, layer, params)?;
            (shove::Shover::Route { points, width_nm }, width_nm)
        }
        (None, Some(_)) if params.get("width_nm").is_none() => (shove::Shover::Drag { tracks: field(params, "tracks")? }, 0),
        (None, Some(_)) => return Err(CommandError::InvalidParams("Dragged tracks keep their width; omit width_nm.".into())),
        _ => return Err(CommandError::InvalidParams("Give either points for a new route or tracks to drag.".into())),
    };
    Ok((target, shove::Request { layer, shover }, width_nm, budget))
}

/// Explain why a shove cannot be committed, naming what stopped it.
fn refusal(plan: &shove::Shove) -> CommandError {
    let at = |p: &Point| format!("({}, {}) nm", p.x_nm, p.y_nm);
    CommandError::InvalidParams(match (&plan.outcome, &plan.blocker) {
        (Outcome::Blocked { last_legal }, Some(blocker)) => format!("The shove is blocked by {blocker}; the last legal point is {}.", at(last_legal)),
        (Outcome::Blocked { last_legal }, None) => {
            format!("The shove is blocked by the board edge or a rule; the last legal point is {}.", at(last_legal))
        }
        (Outcome::Incomplete, Some(blocker)) => format!("The shove reached its depth, push or moved-copper bound while pushing {blocker}."),
        (Outcome::Incomplete, None) => "The shove ran out of its probe budget.".into(),
        _ => "The shove did not complete.".into(),
    })
}

/// The copper candidate of `pcb.shove_tracks`, built in bounded steps from a captured source:
/// obstacle preparation, then the shove planned against that exact board, then all of it
/// applied. Anything but a reached shove is refused, so either every route segment, dragged
/// track, pushed track and via lands or nothing changes. Dropping it cancels the work.
pub(crate) struct ShoveCandidate {
    source: Source,
    request: shove::Request,
    width_nm: i64,
    budget: usize,
    preparation: Option<super::clearance::preparation::Preparation>,
    search: Option<shove::ShoveSearch>,
}

/// A built copper candidate (absent for a no-op) and the identities of the copper it changes.
type Built = (Option<Project>, BTreeSet<Uuid>);

/// Search suspensions (each at most one exact probe or one bounded walk) per host step, as for
/// walkaround planning.
const SHOVE_UNITS_PER_STEP: usize = 8;

impl ShoveCandidate {
    /// Read the request and capture the source; no geometry is prepared yet. A request for
    /// another board than the source's is refused.
    pub(crate) fn new(source: &Source, params: &Value) -> Result<Self, CommandError> {
        let (target, request, width_nm, budget) = shove_request(&source.snapshot, params)?;
        if target != source.target {
            return Err(CommandError::InvalidParams("The shove addresses another board than its preparation.".into()));
        }
        Ok(Self {
            source: source.clone(),
            request,
            width_nm,
            budget,
            preparation: Some(super::clearance::preparation::Preparation::new(source.clone())),
            search: None,
        })
    }

    /// Whether obstacle preparation is done and the shove is being planned.
    pub(crate) fn planning(&self) -> bool {
        self.search.is_some()
    }

    /// Exact probes the shove has charged so far, and its probe budget.
    pub(crate) fn probes(&self) -> (usize, usize) {
        (self.search.as_ref().map_or(0, shove::ShoveSearch::probes), self.budget)
    }

    /// Advance one bounded unit of work: a preparation batch of at most `budget` units, or at
    /// most [`SHOVE_UNITS_PER_STEP`] search suspensions. Returns the candidate (absent for a
    /// no-op) and the identities of changed copper once the shove is planned and applied; a
    /// zero budget does nothing. Applying and validating the reached plan is the final unit.
    pub(crate) fn step(&mut self, budget: usize) -> Result<Option<Built>, CommandError> {
        if budget == 0 {
            return Ok(None);
        }
        if let Some(preparation) = &mut self.preparation {
            if let Some(query) = preparation.step(budget)? {
                self.preparation = None;
                self.search = Some(shove::ShoveSearch::new(query, self.request.clone(), self.budget));
            }
            return Ok(None);
        }
        let search = self.search.as_mut().ok_or_else(|| CommandError::Unavailable("The shove has already finished.".into()))?;
        let Some(plan) = search.step(budget.min(SHOVE_UNITS_PER_STEP)) else { return Ok(None) };
        self.search = None;
        let plan = plan?;
        if plan.outcome != Outcome::Reached {
            return Err(refusal(&plan));
        }
        let project: &Project = &self.source.snapshot;
        let (mut tracks, mut vias) = (project.board.tracks.clone(), project.board.vias.clone());
        let changed = plan.apply(self.request.layer, self.width_nm, &mut tracks, &mut vias, &mut crate::fresh_id)?;
        super::track::validate(project, &mut tracks, &mut vias).map_err(CommandError::InvalidParams)?;
        if tracks == project.board.tracks && vias == project.board.vias {
            return Ok(Some((None, changed)));
        }
        let mut next = project.clone();
        let board = std::sync::Arc::make_mut(&mut next.board);
        board.tracks = tracks;
        board.vias = vias;
        super::track::protect_locked(project, &next)?;
        Ok(Some((Some(next), changed)))
    }
}

impl Session {
    /// Shove over `query` only while it still describes this session's current board; an edit,
    /// Undo/Redo, reopen or another session's query gives a stale result without probing.
    pub fn plan_shove(
        &self,
        query: &ClearanceQuery,
        request: &shove::Request,
        budget: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<shove::Shove, CommandError> {
        if !query.authority().is_current(self) {
            return Ok(shove::Shove::stale());
        }
        shove::shove(query, request, budget, cancel)
    }

    /// Answer `pcb.plan_shove` without changing revision, history or saved bytes. A given
    /// `expected_revision` that is not current gives a stale outcome instead of a plan.
    pub(crate) fn shove_plan_query(&self, params: &Value) -> Result<Value, CommandError> {
        let (target, request, width_nm, budget) = shove_request(self.open_project()?, params)?;
        let stale = params.get("expected_revision").is_some_and(|revision| revision.as_u64() != Some(self.revision()));
        let plan = if stale {
            shove::Shove::stale()
        } else {
            let query = self.clearance_query_with_rules(target, &json!({}))?;
            self.plan_shove(&query, &request, budget, &|| false)?
        };
        let mut report = json!(plan);
        report["board"] = json!(target.board);
        report["layer"] = json!(request.layer);
        report["budget"] = json!(budget);
        if matches!(request.shover, shove::Shover::Route { .. }) {
            report["width_nm"] = json!(width_nm);
        }
        Ok(json!({"revision":self.revision(),"shove_plan":report}))
    }
}

/// Parse one typed command parameter, reporting serde's message as invalid input.
fn field<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T, CommandError> {
    serde_json::from_value(params[name].clone()).map_err(|error| CommandError::InvalidParams(format!("{name}: {error}")))
}

/// Describe the accepted postures for command clients.
pub(crate) fn posture_schema() -> Value {
    json!({"type":"string","enum":Posture::NAMES})
}

/// Describe the probe budget for command clients.
pub(crate) fn budget_schema() -> Value {
    json!({"type":"integer","minimum":1,"maximum":MAX_BUDGET})
}
