//! Clearance-aware walkaround and shove planning over captured copper.
//!
//! [`walkaround`] plans one single-layer route from a head point toward a target over a captured
//! [`ClearanceQuery`]: horizontal, vertical and 45° legs (or horizontal and vertical only) that
//! go round existing pads, tracks and vias, locked or not, with the effective width, pair
//! clearances and board-edge clearance. It never edits the board. Its points feed the existing
//! `pcb.add_tracks` command or copper preparation job, which revalidate and publish them in one
//! undo step. Every kept leg is confirmed by the same exact probe and outline predicates that
//! DRC uses. The search is deterministic and bounded by a probe budget and a cancellation flag.
//! Both shove modes return track replacements for atomic placement. The via-enabled mode can
//! also move a through via and its attached track endpoints when the incoming copper hits it.
//! Pads and locked copper stay fixed. Each through-via move is checked on every copper layer.
use super::clearance::ClearanceQuery;
use crate::{CommandError, Session, schematic::Point};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

mod hull;
mod posture;
mod preparation;
mod search;
pub(in crate::board) use search::Search;
mod shove;
pub use super::track::replacements::ViaMove;
pub use shove::{FixedCopper, Mode, ShoveStats};
pub(crate) use shove::{fixed_schema, valid_fixed};
#[cfg(test)]
mod tests;
mod walkaround;
pub use preparation::{PlanningContext, PlanningJob};

pub use posture::Posture;

/// Plan a route around existing copper without editing the board.
pub const PLAN_ROUTE: &str = "pcb.plan_route";
/// Exact probes one plan may charge when the caller gives no budget.
pub const DEFAULT_BUDGET: usize = 4096;
/// The largest accepted probe budget.
pub const MAX_BUDGET: usize = 65_536;

/// One planning request on one copper layer. `previous` is the caller's last result for the
/// same gesture, if any: an unchanged, still-legal result is kept as is, and otherwise the walk
/// prefers going round on the same side, so nearly equivalent routes do not alternate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Walk around existing copper, shove unlocked tracks, or also move through vias and their attached ends.
    pub mode: Mode,
    /// Opt into joining displaced copper elsewhere on its resolved net/physical island.
    /// False preserves spacing between source objects that were not directly touching.
    pub merge_same_net: bool,
    /// Already fixed gesture copper, checked together with Highlight and shove continuations.
    pub fixed: Vec<FixedCopper>,
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
    /// The requested target is obstructed or no legal way round was found. Points end at the
    /// last legal point; `adjusted_target` identifies a certified placeable endpoint when available.
    /// When the head itself is illegal, points are the head alone and nothing can be added.
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
    /// Certified endpoint of a blocked route, including a refined clearance boundary.
    /// Clients may fix this vertex without claiming contact at the raw cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjusted_target: Option<Point>,
    /// The captured head itself violates a rule, rather than merely having a blocked target.
    /// A host may reject an initial pickup while retaining a valid multi-vertex gesture.
    pub start_blocked: bool,
    /// Source track paths to replace atomically with this route; empty for walkaround.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replacements: Vec<super::track::replacements::Replacement>,
    /// Through vias translated together with their endpoint tracks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via_moves: Vec<ViaMove>,
    /// Bounded shove work, absent for walkaround.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shove: Option<ShoveStats>,
    pub points: Vec<Point>,
    pub outcome: Outcome,
    /// Query address of the copper that stopped a blocked route, absent for the board edge.
    pub blocker: Option<String>,
    /// Source addresses of the existing obstacles touched by a Highlight proposal.
    #[serde(skip)]
    pub collisions: std::collections::BTreeSet<String>,
    /// Exact probes charged, never more than the budget.
    pub probes: usize,
}

/// Check a continuation against the contiguous fixed stroke on its layer, using each leg's width.
async fn legal_incoming(request: &Request, points: &[Point], clearance_nm: i64) -> bool {
    if points.len() < 2 {
        return true;
    }
    let mut prefix = Vec::new();
    let mut widths = Vec::new();
    let mut head = points[0];
    for piece in request.fixed.iter().rev() {
        let FixedCopper::Segment { start, end, layer, width_nm } = piece else { continue };
        if *end != head || *layer != request.layer {
            break;
        }
        prefix.push(*start);
        widths.push(*width_nm);
        head = *start;
    }
    prefix.reverse();
    widths.reverse();
    prefix.extend_from_slice(points);
    widths.resize(prefix.len() - 1, request.width_nm);
    super::track::chains::legal_stroke_with_spacing(&prefix, |a, b| (widths[a] + widths[b]) as f64 / 2. + clearance_nm as f64).await
}

/// Plan a route over a captured board. Validates the request first: bounded endpoints and previous hints, a supported
/// width and a copper layer of the query's board; an invalid request charges nothing. Copper
/// the head touches lends the route its membership and island, so the route may join it; all
/// other copper, including copper of unknown membership, is an obstacle at its pair clearance.
/// The result is a pure function of the query, request and budget unless `cancel` returns true.
pub fn walkaround(query: &ClearanceQuery, request: &Request, budget: usize, cancel: &dyn Fn() -> bool) -> Result<Plan, CommandError> {
    if request.mode != Mode::Walkaround {
        return Err(CommandError::InvalidParams("Use Session::plan_route for shove requests.".into()));
    }
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
    shove::validate_fixed(query, &request.fixed)?;
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
            return Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                via_moves: Vec::new(),
                replacements: Vec::new(),
                shove: None,
                points: Vec::new(),
                outcome: Outcome::Stale,
                blocker: None,
                collisions: Default::default(),
                probes: 0,
            });
        }
        validate(query, request, budget)?;
        search::complete(query, request, budget, cancel)
    }

    /// Answer `pcb.plan_route` without changing revision, history or saved bytes. An omitted
    /// width uses the head contact's net-class preference or the board's preferred width; an
    /// explicit width must satisfy board and applicable class minimums. A given `expected_revision` that
    /// is not current gives a stale outcome instead of a plan. The default mode is walkaround;
    /// the default budget is 4,096 for walkaround or 65,536 for shove. Same-net merging is off
    /// by default. The first fixed segment in gesture order supplies the original pickup.
    pub(crate) fn route_plan_query(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let (head, goal, layer): (Point, Point, Uuid) = (field(params, "from")?, field(params, "to")?, field(params, "layer")?);
        let explicit_width: Option<i64> = params.get("width_nm").map(|_| field(params, "width_nm")).transpose()?;
        let width_nm = match view.route_rules_at(head, Some(layer))?.filter(|policy| policy.has_dimensions()) {
            Some(policy) => policy.track_width(explicit_width)?,
            None => explicit_width.unwrap_or(view.effective_rules().values.preferred_track_width_nm),
        };
        let posture: Posture = params.get("posture").map(|_| field(params, "posture")).transpose()?.unwrap_or_default();
        let flipped = params.get("flipped").and_then(Value::as_bool).unwrap_or(false);
        let previous: Vec<Point> = params.get("previous").map(|_| field(params, "previous")).transpose()?.unwrap_or_default();
        let mode: Mode = params.get("mode").map(|_| field(params, "mode")).transpose()?.unwrap_or_default();
        let budget = params.get("budget").map(|_| field(params, "budget")).transpose()?.unwrap_or_else(|| mode.default_probe_budget());
        let merge_same_net = params.get("merge_same_net").and_then(Value::as_bool).unwrap_or(false);
        let fixed: Vec<FixedCopper> = params.get("fixed").map(|_| field(params, "fixed")).transpose()?.unwrap_or_default();
        let origin = fixed.iter().find_map(FixedCopper::origin);
        let request = Request { mode, merge_same_net, fixed, head, target: goal, layer, origin, width_nm, posture, flipped, previous };
        let stale = params.get("expected_revision").is_some_and(|revision| revision.as_u64() != Some(self.revision()));
        let plan = if stale {
            Plan {
                adjusted_target: None,
                start_blocked: false,
                via_moves: Vec::new(),
                replacements: Vec::new(),
                shove: None,
                points: Vec::new(),
                outcome: Outcome::Stale,
                blocker: None,
                collisions: Default::default(),
                probes: 0,
            }
        } else {
            let query = self.clearance_query_with_rules(target, &json!({}))?;
            self.plan_route(&query, &request, budget, &|| false)?
        };
        let mut result = json!({"revision":self.revision(),"route_plan":{
            "board":target.board,"layer":layer,"mode":mode,"merge_same_net":merge_same_net,"width_nm":width_nm,"posture":posture,"flipped":flipped,"budget":budget,
            "points":plan.points,"outcome":plan.outcome,"blocker":plan.blocker,"probes":plan.probes,
            "replacements":plan.replacements,"via_moves":plan.via_moves,"shove":plan.shove,"start_blocked":plan.start_blocked
        }});
        if let Some(point) = plan.adjusted_target {
            result["route_plan"]["adjusted_target"] = json!(point);
        }
        Ok(result)
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
