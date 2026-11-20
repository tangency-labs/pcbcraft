//! Shove over one captured board. The shover, a new route or dragged tracks on one copper layer,
//! stays where it is asked to go; unlocked tracks and vias of other copper that break clearance
//! with it are pushed aside, and copper that moved pushes in turn, breadth first.
//!
//! A pushed track moves as its chain: tracks of one width on the shove layer joined end to end
//! where exactly two track ends meet and no other copper touches. The chain's two ends stay put,
//! and so does all copper joined to it, so connectivity does not change. The part of the chain
//! inside the pusher's hull (as in walkaround, grown by the pair clearance and the chain's half
//! width) is replaced by the hull's boundary on the side whose result is legal, the shorter
//! first. A pushed via moves the least distance in one of eight 45° directions that clears the
//! pusher's hull; the tracks ending on it follow with an elbow.
//!
//! Pads, locked copper, copper on other layers, the board edge, the shover itself and copper
//! that cannot move without detaching something (a chain touched only in mid-span, a via joined
//! to a pad, another via or another layer's track) never move. Every result is confirmed by the
//! same exact overlay probes and outline predicates DRC uses, with island exemptions only for
//! copper still joined in the result; the propagation is bounded by depth, moved copper, pushes
//! and a probe budget, and is deterministic.
use super::{
    Outcome,
    hull::Hull,
    posture::{Posture, along, elbow, lattice},
    walkaround::{entry, simplify},
};
use crate::{
    CommandError,
    board::{
        clearance::{ClearanceQuery, ObstacleId, Overlay},
        drc,
        geometry::Contour,
        track::{MAX_PATH_POINTS, Track, Via},
    },
    schematic::Point,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

/// Most pushes in one chain of cause and effect: copper pushed by the shover is at depth 1.
pub const MAX_DEPTH: usize = 8;
/// Most track segments and vias one shove may change, add or remove.
pub const MAX_MOVED: usize = 64;
/// Most tracks one drag may move, as the shove commands publish.
pub const MAX_DRAGGED: usize = MAX_PATH_POINTS;
/// Exact probes one shove may charge when the caller gives no budget.
pub const DEFAULT_BUDGET: usize = 16_384;
/// Most times other parts of the pushing copper merge into the hull a chain bends round.
const MAX_MERGES: usize = 4;
/// Most single pushes in one attempt, so copper pushed back and forth cannot loop.
const MAX_PUSHES: usize = 128;
/// Hull growth beyond the exact requirement, covering whole-nanometre rounding of corners.
const MARGIN_NM: f64 = 2.;
/// A blocked route prefix shorter than this adds no copper.
const MIN_REACH_NM: i64 = 1_000;
/// Island marker of the new route's own legs; real islands are indices from the query.
const ROUTE_ISLAND: usize = usize::MAX;
/// Lattice directions a via may move in, counter-clockwise from +x.
const STEPS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// New ends for one existing track segment, as `pcb.reshape_tracks` takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ends {
    pub track: Uuid,
    pub start: Point,
    pub end: Point,
}

/// What pushes: a new route, or existing tracks dragged to new ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shover {
    /// A new path on the shove layer. Copper its first point touches lends it membership.
    Route { points: Vec<Point>, width_nm: i64 },
    /// Existing unlocked tracks on the shove layer with their new ends. They keep their own
    /// membership; the caller moves any neighbours that must follow them.
    Drag { tracks: Vec<Ends> },
}

/// One shove on one copper layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub layer: Uuid,
    pub shover: Shover,
}

/// A segment the shove adds, with the width, layer and membership of the track `like`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Added {
    pub like: Uuid,
    pub start: Point,
    pub end: Point,
}

/// A via the shove moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MovedVia {
    pub via: Uuid,
    pub at: Point,
}

/// A planned shove. Applying all of it (and nothing less) gives the legal board it describes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Shove {
    /// The new route as laid: all of it when reached, the legal prefix when blocked, the head
    /// alone when nothing can be laid. Empty for a drag.
    pub points: Vec<Point>,
    /// A reached drag's tracks with their new ends; empty otherwise.
    pub dragged: Vec<Ends>,
    /// Pushed tracks that keep their identity, with their new ends.
    pub reshaped: Vec<Ends>,
    /// Segments a pushed chain gains.
    pub added: Vec<Added>,
    /// Segments a pushed chain loses.
    pub removed: Vec<Uuid>,
    pub vias: Vec<MovedVia>,
    pub outcome: Outcome,
    /// For `blocked`, the copper that could not move or be passed (absent for the board edge);
    /// for `incomplete`, the copper being pushed when a propagation bound ran out.
    pub blocker: Option<String>,
    /// Exact probes charged, never more than the budget.
    pub probes: usize,
    /// The deepest push the result needed.
    pub depth: usize,
    /// Track segments and vias changed, added or removed: never more than [`MAX_MOVED`].
    pub moved: usize,
}

impl Shove {
    /// A result that moves nothing.
    fn nothing(points: Vec<Point>, outcome: Outcome, blocker: Option<String>, probes: usize) -> Self {
        Self {
            points,
            dragged: Vec::new(),
            reshaped: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
            vias: Vec::new(),
            outcome,
            blocker,
            probes,
            depth: 0,
            moved: 0,
        }
    }

    /// The stale answer: nothing was planned against a board that is no longer current.
    pub(crate) fn stale() -> Self {
        Self::nothing(Vec::new(), Outcome::Stale, None, 0)
    }

    /// The canceled answer: the route's head alone (nothing for a drag), nothing moved.
    fn canceled(request: &Request, probes: usize) -> Self {
        let head = match &request.shover {
            Shover::Route { points, .. } => points.first().copied().into_iter().collect(),
            Shover::Drag { .. } => Vec::new(),
        };
        Self::nothing(head, Outcome::Canceled, None, probes)
    }

    /// Apply the whole result to copies of a board's tracks and vias: the route's segments (new
    /// identities from `fresh`), dragged and pushed ends, added and removed segments and moved
    /// vias. Returns the identities of every changed or added item. Unknown identities fail;
    /// the caller validates and publishes the copies.
    pub(crate) fn apply(
        &self,
        layer: Uuid,
        route_width_nm: i64,
        tracks: &mut Vec<Track>,
        vias: &mut [Via],
        fresh: &mut dyn FnMut() -> Result<Uuid, CommandError>,
    ) -> Result<BTreeSet<Uuid>, CommandError> {
        let mut changed = BTreeSet::new();
        let positions: BTreeMap<Uuid, usize> = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let unknown = || CommandError::NotFound("unknown track in the shove".into());
        for ends in self.dragged.iter().chain(&self.reshaped) {
            let &i = positions.get(&ends.track).ok_or_else(unknown)?;
            (tracks[i].start, tracks[i].end) = (ends.start, ends.end);
            changed.insert(ends.track);
        }
        for added in &self.added {
            let like = tracks.get(*positions.get(&added.like).ok_or_else(unknown)?).ok_or_else(unknown)?.clone();
            let id = fresh()?;
            tracks.push(Track { id, locked: false, start: added.start, end: added.end, ..like });
            changed.insert(id);
        }
        for leg in self.points.windows(2) {
            let id = fresh()?;
            tracks.push(Track { id, locked: false, layer, width_nm: route_width_nm, start: leg[0], end: leg[1] });
            changed.insert(id);
        }
        let removed: BTreeSet<Uuid> = self.removed.iter().copied().collect();
        if removed.iter().any(|id| !positions.contains_key(id)) {
            return Err(unknown());
        }
        tracks.retain(|t| !removed.contains(&t.id));
        for moved in &self.vias {
            let via = vias.iter_mut().find(|v| v.id == moved.via).ok_or_else(|| CommandError::NotFound("unknown via in the shove".into()))?;
            via.at = moved.at;
            changed.insert(moved.via);
        }
        Ok(changed)
    }
}

/// Copper walked between suspensions while confirming that exempt copper is still joined.
const WALK_UNIT: usize = 64;

/// Plan a shove over a captured board. Validates the request first (a copper layer of the
/// query's board, bounded points, a supported width, known unlocked dragged tracks on the layer
/// and a budget from 1 to [`super::MAX_BUDGET`]); an invalid request charges nothing. The result
/// is a pure function of the query, request and budget unless `cancel` returns true: it is
/// checked before every exact probe and every bounded connectivity walk, and a cancel returns
/// `canceled` with the route's head alone and nothing moved.
pub fn shove(query: &ClearanceQuery, request: &Request, budget: usize, cancel: &dyn Fn() -> bool) -> Result<Shove, CommandError> {
    let progress = Arc::new(AtomicUsize::new(0));
    let mut future = std::pin::pin!(solve(query, request, budget, progress.clone()));
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if cancel() {
            return Ok(Shove::canceled(request, progress.load(Ordering::Relaxed)));
        }
        if let Poll::Ready(plan) = future.as_mut().poll(&mut cx) {
            return plan;
        }
    }
}

/// Validate and run one shove, suspending before every exact probe and bounded walk.
async fn solve(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Shove, CommandError> {
    let mut solver = Solver::new(query, request, budget, progress)?;
    solver.run().await
}

/// An owned, resumable shove over a captured query, for hosts that advance work in bounded
/// steps. Dropping it cancels the private work; nothing is published until it finishes.
pub(crate) struct ShoveSearch {
    future: Pin<Box<dyn Future<Output = Result<Shove, CommandError>> + Send>>,
    progress: Arc<AtomicUsize>,
}

impl ShoveSearch {
    /// Own the query and request so suspension borrows no session or editor.
    pub(crate) fn new(query: Arc<ClearanceQuery>, request: Request, budget: usize) -> Self {
        let progress = Arc::new(AtomicUsize::new(0));
        let counter = progress.clone();
        Self { future: Box::pin(async move { solve(&query, &request, budget, counter).await }), progress }
    }

    /// Exact probes charged so far.
    pub(crate) fn probes(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }

    /// Advance at most `units` suspensions (each at most one exact probe or one bounded walk);
    /// `None` while the shove is unfinished. A zero `units` does no work.
    pub(crate) fn step(&mut self, units: usize) -> Option<Result<Shove, CommandError>> {
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..units {
            if let Poll::Ready(plan) = self.future.as_mut().poll(&mut cx) {
                return Some(plan);
            }
        }
        None
    }
}

/// Why an attempt stopped.
enum Halt {
    /// Copper that cannot move or be passed; its query address when known.
    Blocked(Option<String>),
    /// A propagation bound ran out while pushing this copper.
    Bound(Option<String>),
    /// The probe budget ran out.
    Budget,
    Failed(CommandError),
}

impl From<CommandError> for Halt {
    fn from(error: CommandError) -> Self {
        Self::Failed(error)
    }
}

/// Copper that takes part in a shove, as one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Item {
    /// The new route's legs.
    Route,
    /// A dragged track (query index).
    Dragged(usize),
    /// A movable chain (index into [`Solver::shapes`]).
    Chain(usize),
    /// A movable via (query index).
    Via(usize),
}

/// The fixed facts of a movable chain, from the captured board.
struct Shape {
    /// Query indices of its tracks, in path order.
    tracks: Vec<usize>,
    ids: Vec<Uuid>,
    original: Vec<Point>,
    half: f64,
    width_nm: i64,
    posture: Posture,
    nets: Vec<usize>,
    island: usize,
}

/// A movable via's tracks: each chain ending on it, and whether that is the chain's last point.
struct Joint {
    chains: Vec<(usize, bool)>,
}

/// The exact verdict for one contour.
struct Verdict {
    /// The board edge or outline forbids it.
    edge: bool,
    /// Copper of other memberships that breaks clearance, with each pair's requirement.
    offenders: Vec<(ObstacleId, i64)>,
    /// Breaks a rule even with no offender (a short through own copper, a width minimum).
    illegal: bool,
    /// Copper closer than its clearance that is exempt only because it was in one of the item's
    /// islands on the captured board, without touching the item now. Confirmation keeps the
    /// exemption only while that copper is still joined to the item.
    joined: Vec<ObstacleId>,
}

/// One try at shoving one shover: an overlay of moved copper over the captured board.
struct Attempt<'a> {
    overlay: Overlay<'a>,
    route: Vec<Point>,
    route_ids: Vec<Uuid>,
    chains: BTreeMap<usize, (Vec<Point>, Vec<Uuid>)>,
    vias: BTreeMap<usize, Point>,
    /// Chain owning each added overlay segment.
    owners: BTreeMap<Uuid, usize>,
    queue: VecDeque<(Item, usize)>,
    pushes: usize,
    depth: usize,
    next_id: u128,
    /// The route's settling membership, cleared whenever copper moves.
    route_nets: Option<Vec<usize>>,
}

/// One shove: the captured query, the shover's copper and membership, the movable copper found
/// so far and the probe budget.
struct Solver<'a> {
    query: &'a ClearanceQuery,
    request: &'a Request,
    layer: [Uuid; 1],
    lands: usize,
    tracks: Vec<Track>,
    vias: Vec<Via>,
    outline: drc::Outline,
    /// Track ends on the shove layer, by point.
    ends: BTreeMap<(i64, i64), Vec<usize>>,
    /// Membership of the route: its head contacts' nets and islands, and the contact it inherits from.
    route_nets: Vec<usize>,
    route_islands: Vec<usize>,
    route_from: Option<usize>,
    route_half: f64,
    route_width_nm: i64,
    /// Query indices of dragged tracks with their new ends.
    dragged: Vec<(usize, Point, Point)>,
    chain_of: BTreeMap<usize, Option<usize>>,
    shapes: Vec<Shape>,
    joints: BTreeMap<usize, Option<Joint>>,
    budget: usize,
    probes: usize,
    /// Probes charged so far, readable while the shove is suspended.
    progress: Arc<AtomicUsize>,
}

impl<'a> Solver<'a> {
    /// Validate the request and capture the board's tracks, vias, outline and the shover's membership.
    fn new(query: &'a ClearanceQuery, request: &'a Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Self, CommandError> {
        if !(1..=super::MAX_BUDGET).contains(&budget) {
            return Err(CommandError::InvalidParams(format!("A shove budget must be from 1 to {} probes.", super::MAX_BUDGET)));
        }
        let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("The captured board is no longer available.".into()))?;
        let (tracks, vias) = (view.tracks().to_vec(), view.vias().to_vec());
        let lands = query.objects().len().saturating_sub(tracks.len() + vias.len());
        if query.objects().get(lands..lands + tracks.len()).is_none_or(|o| o.iter().zip(&tracks).any(|(o, t)| o.name != t.id.to_string())) {
            return Err(CommandError::Unavailable("The captured copper does not match its board.".into()));
        }
        let layer = [request.layer];
        let objects = query.objects();
        let (mut route_nets, mut route_islands, mut route_from, mut route_half, mut route_width_nm, mut dragged) =
            (Vec::new(), Vec::new(), None, 0., 0, Vec::new());
        match &request.shover {
            Shover::Route { points, width_nm } => {
                if !(2..=MAX_PATH_POINTS).contains(&points.len()) || points.windows(2).any(|leg| leg[0] == leg[1]) {
                    return Err(CommandError::InvalidParams(format!("A shoved route needs 2 to {MAX_PATH_POINTS} points with distinct neighbours.")));
                }
                for leg in points.windows(2) {
                    Contour::segment(leg[0], leg[1], *width_nm)?;
                }
                if *width_nm < query.authority().rules().values.min_track_width_nm {
                    return Err(CommandError::InvalidParams("Track width cannot weaken the board minimum.".into()));
                }
                route_half = *width_nm as f64 / 2.;
                route_width_nm = *width_nm;
                let at_head = Contour::capsule(points[0], points[0], route_half);
                let contacts: Vec<usize> =
                    query.candidates(&at_head, &layer)?.into_iter().filter(|i| objects[*i].contour.gap(&at_head) <= 0.).collect();
                route_nets = contacts.iter().flat_map(|i| objects[*i].intended_nets.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
                route_islands = contacts.iter().map(|i| objects[*i].island).chain([ROUTE_ISLAND]).collect();
                route_from = contacts.iter().copied().find(|i| !objects[*i].intended_nets.is_empty());
            }
            Shover::Drag { tracks: ends } => {
                query.candidates(&Contour::capsule(Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 0, y_nm: 0 }, 0.), &layer)?;
                let unique: BTreeSet<_> = ends.iter().map(|e| e.track).collect();
                if ends.is_empty() || ends.len() > MAX_DRAGGED || unique.len() != ends.len() {
                    return Err(CommandError::InvalidParams(format!("Drag 1 to {MAX_DRAGGED} distinct tracks.")));
                }
                for e in ends {
                    let k = tracks.iter().position(|t| t.id == e.track).ok_or_else(|| CommandError::NotFound("unknown track".into()))?;
                    let track = &tracks[k];
                    if track.layer != request.layer || track.locked {
                        return Err(CommandError::InvalidParams("Dragged tracks must be unlocked tracks on the shove layer.".into()));
                    }
                    Track { start: e.start, end: e.end, ..track.clone() }.validate().map_err(CommandError::InvalidParams)?;
                    dragged.push((lands + k, e.start, e.end));
                }
            }
        }
        let mut ends: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
        for (k, track) in tracks.iter().enumerate().filter(|(_, t)| t.layer == request.layer) {
            for p in [track.start, track.end] {
                ends.entry((p.x_nm, p.y_nm)).or_default().push(lands + k);
            }
        }
        Ok(Self {
            query,
            request,
            layer,
            lands,
            outline: view.outline_edges(),
            tracks,
            vias,
            ends,
            route_nets,
            route_islands,
            route_from,
            route_half,
            route_width_nm,
            dragged,
            chain_of: BTreeMap::new(),
            shapes: Vec::new(),
            joints: BTreeMap::new(),
            budget,
            probes: 0,
            progress,
        })
    }

    /// Shove the whole shover; when a route is blocked, find its longest legal prefix by
    /// bisection along the path. Bounds and blockers of the whole shover are reported.
    async fn run(&mut self) -> Result<Shove, CommandError> {
        let full = match &self.request.shover {
            Shover::Route { points, .. } => points.clone(),
            Shover::Drag { .. } => Vec::new(),
        };
        let head = full.first().copied();
        let blocker = match self.attempt(&full).await {
            Ok(attempt) => return Ok(self.finish(attempt, Outcome::Reached)),
            Err(Halt::Blocked(blocker)) => blocker,
            Err(halt) => return self.halted(halt, head),
        };
        let Some(head) = head else {
            let last_legal = self.dragged.first().map_or(Point { x_nm: 0, y_nm: 0 }, |(i, _, _)| self.tracks[i - self.lands].start);
            return Ok(Shove::nothing(Vec::new(), Outcome::Blocked { last_legal }, blocker, self.probes));
        };
        let steps: Vec<i64> = full.windows(2).map(|leg| lattice(leg[0], leg[1]).0).collect();
        let total: i64 = steps.iter().sum();
        let prefix = |s: i64| -> Vec<Point> {
            let mut points = vec![head];
            let mut left = s;
            for (leg, &n) in full.windows(2).zip(&steps) {
                if left >= n {
                    points.push(leg[1]);
                    left -= n;
                } else {
                    if left > 0 {
                        points.push(along(leg[0], leg[1], left, n));
                    }
                    break;
                }
            }
            simplify(&points)
        };
        let (mut lo, mut hi, mut best) = (0, total, None);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            match self.attempt(&prefix(mid)).await {
                Ok(attempt) => {
                    lo = mid;
                    best = Some(attempt);
                }
                Err(Halt::Blocked(_) | Halt::Bound(_)) => hi = mid,
                Err(halt) => return self.halted(halt, Some(head)),
            }
        }
        match best.filter(|a| length(&a.route) >= MIN_REACH_NM as f64) {
            Some(attempt) => {
                let last_legal = attempt.route.last().copied().unwrap_or(head);
                let mut plan = self.finish(attempt, Outcome::Blocked { last_legal });
                plan.blocker = blocker;
                Ok(plan)
            }
            None => Ok(Shove::nothing(vec![head], Outcome::Blocked { last_legal: head }, blocker, self.probes)),
        }
    }

    /// The result for an attempt stopped by the budget, a bound or a refused probe.
    fn halted(&self, halt: Halt, head: Option<Point>) -> Result<Shove, CommandError> {
        let points: Vec<Point> = head.into_iter().collect();
        match halt {
            Halt::Failed(error) => Err(error),
            Halt::Bound(at) => Ok(Shove::nothing(points, Outcome::Incomplete, at, self.probes)),
            // `run` handles `Blocked` itself; only the budget reaches here in practice.
            Halt::Budget | Halt::Blocked(_) => Ok(Shove::nothing(points, Outcome::Incomplete, None, self.probes)),
        }
    }

    /// Lay `route` (or the dragged tracks) on a fresh overlay, settle every push and confirm the result.
    async fn attempt(&mut self, route: &[Point]) -> Result<Attempt<'a>, Halt> {
        let mut attempt = Attempt {
            overlay: self.query.overlay(),
            route: route.to_vec(),
            route_ids: Vec::new(),
            chains: BTreeMap::new(),
            vias: BTreeMap::new(),
            owners: BTreeMap::new(),
            queue: VecDeque::new(),
            pushes: 0,
            depth: 0,
            next_id: 0,
            route_nets: None,
        };
        for leg in route.windows(2) {
            let id = attempt.fresh();
            attempt.overlay.add(id, Contour::capsule(leg[0], leg[1], self.route_half), self.layer.to_vec(), self.route_from)?;
            attempt.route_ids.push(id);
        }
        if !attempt.route_ids.is_empty() {
            attempt.queue.push_back((Item::Route, 0));
        }
        for &(i, start, end) in &self.dragged {
            attempt.overlay.replace(i, Contour::capsule(start, end, self.tracks[i - self.lands].width_nm as f64 / 2.), self.layer.to_vec())?;
            attempt.queue.push_back((Item::Dragged(i), 0));
        }
        self.settle(&mut attempt).await?;
        self.confirm(&mut attempt).await?;
        Ok(attempt)
    }

    /// Push everything each queued item breaks clearance with, breadth first, until nothing is
    /// left to push. Copper that cannot move, or pushed copper with no legal place, blocks.
    async fn settle(&mut self, attempt: &mut Attempt<'a>) -> Result<(), Halt> {
        while let Some((item, depth)) = attempt.queue.pop_front() {
            for contour in self.contours(attempt, item) {
                let mut last = None;
                loop {
                    let verdict = self.check(attempt, item, &contour).await?;
                    if verdict.edge {
                        return Err(Halt::Blocked(None));
                    }
                    let mut offenders = verdict.offenders;
                    offenders.sort_by_key(|(id, _)| (!self.is_via(*id), *id));
                    let Some(&(obstacle, clearance)) = offenders.first() else { break };
                    if last == Some(obstacle) {
                        return Err(Halt::Blocked(self.name(attempt, obstacle)));
                    }
                    last = Some(obstacle);
                    match self.item_of(attempt, obstacle)? {
                        Some(Item::Chain(c)) if Item::Chain(c) != item => self.push_chain(attempt, c, item, &contour, clearance, depth + 1).await?,
                        Some(Item::Via(v)) if Item::Via(v) != item => self.push_via(attempt, v, item, &contour, clearance, depth + 1).await?,
                        _ => return Err(Halt::Blocked(self.name(attempt, obstacle))),
                    }
                }
            }
        }
        Ok(())
    }

    /// Recheck the settled result as a whole: every shover leg and every moved item must break
    /// no clearance, short or edge rule. The route is checked with the union of every net it
    /// touches, so legs touching different nets cannot each pass alone.
    async fn confirm(&mut self, attempt: &mut Attempt<'a>) -> Result<(), Halt> {
        let mut items = vec![Item::Route];
        items.extend(self.dragged.iter().map(|(i, _, _)| Item::Dragged(*i)));
        items.extend(attempt.chains.keys().map(|c| Item::Chain(*c)));
        items.extend(attempt.vias.keys().map(|v| Item::Via(*v)));
        let nets = self.route_membership(attempt)?;
        let (layer, islands) = (self.layer.to_vec(), self.route_islands.clone());
        for item in items {
            let mut component = None;
            for contour in self.contours(attempt, item) {
                let verdict = if item == Item::Route {
                    self.check_with(attempt, &contour, &layer, &nets, &islands, Some(self.route_width_nm)).await?
                } else {
                    self.check(attempt, item, &contour).await?
                };
                if verdict.edge {
                    return Err(Halt::Blocked(None));
                }
                if let Some((obstacle, _)) = verdict.offenders.first() {
                    return Err(Halt::Blocked(self.name(attempt, *obstacle)));
                }
                if verdict.illegal {
                    return Err(Halt::Blocked(None));
                }
                // Island exemptions hold only for copper the item is still joined to: a drag can
                // split an island, and DRC rebuilds islands from the committed copper.
                for obstacle in verdict.joined {
                    let joined = match &component {
                        Some(joined) => joined,
                        None => component.insert(self.component(attempt, item).await?),
                    };
                    if !joined.contains(&obstacle) {
                        return Err(Halt::Blocked(self.name(attempt, obstacle)));
                    }
                }
            }
        }
        Ok(())
    }

    /// The copper joined to `item` as it stands in the attempt: a walk over touching copper of
    /// the item's own captured islands, starting from the item's obstacles. Copper of other
    /// islands is not walked, so a joint made only through another island does not count (a
    /// contact between islands is itself refused unless the nets agree).
    /// The walk suspends every [`WALK_UNIT`] objects, so a large island is visited in bounded steps.
    async fn component(&self, attempt: &Attempt<'a>, item: Item) -> Result<BTreeSet<ObstacleId>, Halt> {
        let (mut queue, islands): (Vec<ObstacleId>, Vec<usize>) = match item {
            Item::Route => (attempt.route_ids.iter().map(|u| ObstacleId::Added(*u)).collect(), self.route_islands.clone()),
            Item::Dragged(i) | Item::Via(i) => (vec![ObstacleId::Source(i)], vec![self.query.objects()[i].island]),
            Item::Chain(c) => {
                let extra = attempt.chains.get(&c).map(|(_, extra)| extra.clone()).unwrap_or_default();
                let ids = self.shapes[c].tracks.iter().map(|t| ObstacleId::Source(*t)).chain(extra.into_iter().map(ObstacleId::Added));
                (ids.filter(|id| attempt.overlay.object(*id).is_some()).collect(), vec![self.shapes[c].island])
            }
        };
        let mut seen: BTreeSet<ObstacleId> = queue.iter().copied().collect();
        let mut walked = 0;
        while let Some(id) = queue.pop() {
            if walked % WALK_UNIT == 0 {
                super::search::yield_unit().await;
            }
            walked += 1;
            let Some(object) = attempt.overlay.object(id) else { continue };
            for next in attempt.overlay.candidates(&object.contour, &object.layers)? {
                if seen.contains(&next) || !islands.contains(&self.island(attempt, next)) {
                    continue;
                }
                if let Some(other) = attempt.overlay.object(next)
                    && !other.mechanical
                    && other.contour.gap(&object.contour) <= 0.
                {
                    seen.insert(next);
                    queue.push(next);
                }
            }
        }
        Ok(seen)
    }

    /// The route's nets: its head contacts' and those of all copper its legs touch.
    fn route_membership(&self, attempt: &Attempt<'a>) -> Result<Vec<usize>, Halt> {
        let mut nets: BTreeSet<usize> = self.route_nets.iter().copied().collect();
        for leg in attempt.route.windows(2) {
            let contour = Contour::capsule(leg[0], leg[1], self.route_half);
            for id in attempt.overlay.candidates(&contour, &self.layer)? {
                if let ObstacleId::Added(u) = id
                    && attempt.route_ids.contains(&u)
                {
                    continue;
                }
                if let Some(object) = attempt.overlay.object(id)
                    && object.contour.gap(&contour) <= 0.
                {
                    nets.extend(object.intended_nets.iter().copied());
                }
            }
        }
        Ok(nets.into_iter().collect())
    }

    /// The membership a route settles with, cached until copper moves. The committed route joins
    /// the copper it touches, so its membership is the nets of that copper, as confirmation and
    /// DRC see it. When that copper carries more than one net, only the head's nets and those
    /// of copper that cannot move count, and movable copper of other nets is pushed away rather
    /// than letting one contact exempt the others. A route touching a pad without a net (an
    /// unconnected terminal) keeps the head's nets alone: it never acquires a net, as in walkaround.
    fn route_settle_nets(&mut self, attempt: &mut Attempt<'a>) -> Result<Vec<usize>, Halt> {
        if let Some(nets) = &attempt.route_nets {
            return Ok(nets.clone());
        }
        let mut contacts: BTreeMap<ObstacleId, Vec<usize>> = BTreeMap::new();
        let mut unconnected_pad = false;
        for leg in attempt.route.windows(2) {
            let contour = Contour::capsule(leg[0], leg[1], self.route_half);
            for id in attempt.overlay.candidates(&contour, &self.layer)? {
                if matches!(id, ObstacleId::Added(u) if attempt.route_ids.contains(&u)) || contacts.contains_key(&id) {
                    continue;
                }
                let Some(object) = attempt.overlay.object(id).filter(|o| !o.mechanical && o.contour.gap(&contour) <= 0.) else { continue };
                unconnected_pad |= matches!(id, ObstacleId::Source(i) if i < self.lands) && object.intended_nets.is_empty();
                contacts.insert(id, object.intended_nets.clone());
            }
        }
        let head: BTreeSet<usize> = self.route_nets.iter().copied().collect();
        let all: BTreeSet<usize> = head.iter().copied().chain(contacts.values().flatten().copied()).collect();
        let nets = if unconnected_pad {
            head
        } else if all.len() <= 1 {
            all
        } else {
            let mut fixed = head;
            for (id, nets) in &contacts {
                if !matches!(self.item_of(attempt, *id)?, Some(Item::Chain(_) | Item::Via(_))) {
                    fixed.extend(nets.iter().copied());
                }
            }
            fixed
        };
        let nets: Vec<usize> = nets.into_iter().collect();
        attempt.route_nets = Some(nets.clone());
        Ok(nets)
    }

    /// Push chain `c` out of the hull of `pusher` (part of `by`), trying the shorter side first.
    /// A side is taken when its result clears every immovable copper and `by`; movable copper
    /// it meets is pushed when the chain's turn comes. When a side still meets other parts of
    /// `by` (another leg of the pushing route or chain), their hulls merge into the pusher's
    /// and the sides are chosen again, up to [`MAX_MERGES`] times.
    async fn push_chain(
        &mut self,
        attempt: &mut Attempt<'a>,
        c: usize,
        by: Item,
        pusher: &Contour,
        clearance: i64,
        depth: usize,
    ) -> Result<(), Halt> {
        self.bound(attempt, Item::Chain(c), depth)?;
        let (half, posture, original) = (self.shapes[c].half, self.shapes[c].posture, self.shapes[c].original.clone());
        let now = attempt.chains.get(&c).map_or_else(|| original.clone(), |(points, _)| points.clone());
        let grow = |contour: &Contour| Hull::grown(&contour.core, contour.radius + half + clearance as f64 + MARGIN_NM, posture);
        let mut hull = grow(pusher).ok_or(Halt::Blocked(None))?;
        let mut parts = self.contours(attempt, by);
        parts.retain(|part| part.core != pusher.core);
        let mut blocker = None;
        for _ in 0..=MAX_MERGES {
            let mut options: Vec<(f64, bool, Vec<Point>)> = [true, false]
                .into_iter()
                .filter_map(|ccw| bend(&now, hull.clone(), ccw).map(|points| (length(&points), !ccw, points)))
                .filter(|(_, _, points)| points.len() <= MAX_PATH_POINTS && !sharp(points, &original))
                .collect();
            options.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut met: Vec<Contour> = Vec::new();
            for (index, (_, _, points)) in options.iter().enumerate() {
                self.set_chain(attempt, c, points)?;
                match self.admits(attempt, Item::Chain(c), by).await? {
                    Ok(()) => return self.accept(attempt, &[Item::Chain(c)], depth),
                    Err(name) => blocker = blocker.or(name),
                }
                // Only the preferred side grows the hull, so a longer side's crossings do not
                // swallow the chain's fixed ends.
                if index == 0 {
                    let legs: Vec<Contour> = points.windows(2).map(|leg| Contour::capsule(leg[0], leg[1], half)).collect();
                    met.extend(parts.iter().filter(|part| legs.iter().any(|leg| part.gap(leg) < clearance as f64)).cloned());
                }
            }
            if met.is_empty() {
                break;
            }
            for part in &met {
                hull.merge(&grow(part).ok_or(Halt::Blocked(None))?);
            }
            parts.retain(|part| !met.iter().any(|m| m.core == part.core));
        }
        self.set_chain(attempt, c, &now)?;
        Err(Halt::Blocked(blocker.or_else(|| self.item_name(Item::Chain(c)))))
    }

    /// Move via `v` the least distance, in a 45° multiple, that clears the hull of `pusher`;
    /// chains ending on it follow with an elbow. The nearest legal place wins.
    async fn push_via(&mut self, attempt: &mut Attempt<'a>, v: usize, by: Item, pusher: &Contour, clearance: i64, depth: usize) -> Result<(), Halt> {
        self.bound(attempt, Item::Via(v), depth)?;
        let center = self.via_at(attempt, v);
        let radius = self.query.objects()[v].contour.radius;
        let hull = Hull::grown(&pusher.core, pusher.radius + radius + clearance as f64 + MARGIN_NM, Posture::Diagonal).ok_or(Halt::Blocked(None))?;
        let mut options: Vec<(f64, usize, Point)> = STEPS
            .iter()
            .enumerate()
            .filter_map(|(k, &(dx, dy))| {
                let t = hull.escape(center, Point { x_nm: dx, y_nm: dy })?.max(1);
                let at = Point { x_nm: center.x_nm.checked_add(dx.checked_mul(t)?)?, y_nm: center.y_nm.checked_add(dy.checked_mul(t)?)? };
                crate::wire::valid_point(&at).then_some((t as f64 * (dx as f64).hypot(dy as f64), k, at))
            })
            .collect();
        options.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let attached: Vec<(usize, bool)> = self.joints.get(&v).and_then(|j| j.as_ref()).map(|j| j.chains.clone()).unwrap_or_default();
        if attached.iter().any(|(c, _)| Item::Chain(*c) == by) {
            return Err(Halt::Blocked(Some(self.query.objects()[v].name.clone())));
        }
        let before: Vec<(usize, Vec<Point>)> =
            attached.iter().map(|(c, _)| (*c, attempt.chains.get(c).map_or_else(|| self.shapes[*c].original.clone(), |(p, _)| p.clone()))).collect();
        let mut blocker = None;
        'options: for (_, _, at) in options {
            self.set_via(attempt, v, at)?;
            for ((c, last), (_, points)) in attached.iter().zip(&before) {
                let Some(moved) = follow(points, *last, at, self.shapes[*c].posture, &self.shapes[*c].original) else {
                    self.restore(attempt, v, center, &before)?;
                    continue 'options;
                };
                self.set_chain(attempt, *c, &moved)?;
            }
            let mut items = vec![Item::Via(v)];
            items.extend(attached.iter().map(|(c, _)| Item::Chain(*c)));
            let mut admitted = true;
            for item in &items {
                if let Err(name) = self.admits(attempt, *item, by).await? {
                    blocker = blocker.or(name);
                    admitted = false;
                    break;
                }
            }
            if admitted {
                return self.accept(attempt, &items, depth);
            }
            self.restore(attempt, v, center, &before)?;
        }
        Err(Halt::Blocked(blocker.or_else(|| Some(self.query.objects()[v].name.clone()))))
    }

    /// Put a via and the chains ending on it back where they were before a refused option.
    fn restore(&mut self, attempt: &mut Attempt<'a>, v: usize, center: Point, chains: &[(usize, Vec<Point>)]) -> Result<(), Halt> {
        self.set_via(attempt, v, center)?;
        for (c, points) in chains {
            self.set_chain(attempt, *c, points)?;
        }
        Ok(())
    }

    /// Charge one push at `depth`, refusing it past the depth or push bound.
    fn bound(&self, attempt: &mut Attempt<'a>, item: Item, depth: usize) -> Result<(), Halt> {
        attempt.pushes += 1;
        if depth > MAX_DEPTH || attempt.pushes > MAX_PUSHES {
            return Err(Halt::Bound(self.item_name(item)));
        }
        Ok(())
    }

    /// Take pushed items: queue them to push in turn and enforce the moved-copper bound.
    fn accept(&self, attempt: &mut Attempt<'a>, items: &[Item], depth: usize) -> Result<(), Halt> {
        attempt.depth = attempt.depth.max(depth);
        if self.edits(attempt).moved() > MAX_MOVED {
            return Err(Halt::Bound(items.first().and_then(|item| self.item_name(*item))));
        }
        attempt.queue.extend(items.iter().map(|item| (*item, depth)));
        Ok(())
    }

    /// Whether `item`'s current geometry is clear of the board edge, of `by` and of every copper
    /// that cannot move. `Err` names what refuses it (absent for the edge or a rule).
    async fn admits(&mut self, attempt: &mut Attempt<'a>, item: Item, by: Item) -> Result<Result<(), Option<String>>, Halt> {
        for contour in self.contours(attempt, item) {
            let verdict = self.check(attempt, item, &contour).await?;
            if verdict.edge {
                return Ok(Err(None));
            }
            for (obstacle, _) in verdict.offenders {
                match self.item_of(attempt, obstacle)? {
                    Some(other @ (Item::Chain(_) | Item::Via(_))) if other != by && other != item => {}
                    _ => return Ok(Err(self.name(attempt, obstacle))),
                }
            }
        }
        Ok(Ok(()))
    }

    /// Current contours of an item.
    fn contours(&self, attempt: &Attempt<'a>, item: Item) -> Vec<Contour> {
        match item {
            Item::Route => attempt.route.windows(2).map(|leg| Contour::capsule(leg[0], leg[1], self.route_half)).collect(),
            Item::Dragged(i) => attempt.overlay.object(ObstacleId::Source(i)).map(|o| o.contour.clone()).into_iter().collect(),
            Item::Chain(c) => {
                let shape = &self.shapes[c];
                let points = attempt.chains.get(&c).map_or(&shape.original, |(points, _)| points);
                points.windows(2).map(|leg| Contour::capsule(leg[0], leg[1], shape.half)).collect()
            }
            Item::Via(v) => {
                let at = self.via_at(attempt, v);
                vec![Contour::capsule(at, at, self.query.objects()[v].contour.radius)]
            }
        }
    }

    /// Probe one contour of `item` with the item's membership, own islands and track width.
    /// The route uses its settling membership ([`Self::route_settle_nets`]).
    async fn check(&mut self, attempt: &mut Attempt<'a>, item: Item, contour: &Contour) -> Result<Verdict, Halt> {
        let query = self.query;
        let objects = query.objects();
        let (nets, islands, layers, width) = match item {
            Item::Route => (self.route_settle_nets(attempt)?, self.route_islands.clone(), self.layer.to_vec(), Some(self.route_width_nm)),
            Item::Dragged(i) => {
                (objects[i].intended_nets.clone(), vec![objects[i].island], objects[i].layers.clone(), Some(self.tracks[i - self.lands].width_nm))
            }
            Item::Via(i) => (objects[i].intended_nets.clone(), vec![objects[i].island], objects[i].layers.clone(), None),
            Item::Chain(c) => (self.shapes[c].nets.clone(), vec![self.shapes[c].island], self.layer.to_vec(), Some(self.shapes[c].width_nm)),
        };
        self.check_with(attempt, contour, &layers, &nets, &islands, width).await
    }

    /// Charge one exact probe: overlay clearance and short decisions, then the outline and
    /// edge-clearance predicates DRC uses. Copper of the same single net is exempt as in DRC;
    /// copper of the item's own islands is exempt unless the contact joins distinct nets, and
    /// such copper that no longer touches the item is listed in [`Verdict::joined`]. A track
    /// `width` is illegal below the board minimum or the minimum of the net classes the probe
    /// connects, by the same rule the copper commit applies. Suspends before charging the probe.
    async fn check_with(
        &mut self,
        attempt: &Attempt<'a>,
        contour: &Contour,
        layers: &[Uuid],
        nets: &[usize],
        islands: &[usize],
        width: Option<i64>,
    ) -> Result<Verdict, Halt> {
        super::search::yield_unit().await;
        if self.probes >= self.budget {
            return Err(Halt::Budget);
        }
        self.probes += 1;
        self.progress.store(self.probes, Ordering::Relaxed);
        let results = attempt.overlay.probe_membership(contour, layers, nets, None)?;
        let mut verdict = Verdict { edge: false, offenders: Vec::new(), illegal: false, joined: Vec::new() };
        for decision in &results.decisions {
            let Some(object) = attempt.overlay.object(decision.obstacle) else { continue };
            let same_net = nets.len() == 1 && object.intended_nets == nets;
            let violation = !same_net && decision.gap_nm < decision.clearance_nm as f64;
            if !violation && !decision.conflicting_nets {
                continue;
            }
            if !decision.conflicting_nets && islands.contains(&self.island(attempt, decision.obstacle)) {
                if violation && decision.gap_nm > 0. {
                    verdict.joined.push(decision.obstacle);
                }
                continue;
            }
            verdict.illegal = true;
            if !same_net && !islands.contains(&self.island(attempt, decision.obstacle)) {
                verdict.offenders.push((decision.obstacle, decision.clearance_nm));
            }
        }
        let baseline = self.query.authority().rules().values;
        let rules = results.probe_rules.as_ref().map_or(baseline, |rules| rules.hard_values(baseline));
        if let Some(width) = width
            && (width < rules.min_track_width_nm
                || results
                    .probe_rules
                    .as_ref()
                    .filter(|policy| policy.has_dimensions())
                    .is_some_and(|policy| policy.track_width(Some(width)).is_err()))
        {
            verdict.illegal = true;
        }
        if !self.outline.polygons.is_empty() {
            let (distance, crosses) = drc::edge_gap(&self.outline.edges, contour);
            if !drc::contains(&self.outline.polygons, contour) || crosses || distance < rules.edge_clearance_nm as f64 {
                verdict.edge = true;
            }
        }
        Ok(verdict)
    }

    /// The physical island an overlay obstacle belongs to; the route's legs share one marker.
    fn island(&self, attempt: &Attempt<'a>, id: ObstacleId) -> usize {
        match id {
            ObstacleId::Source(i) => self.query.objects().get(i).map_or(ROUTE_ISLAND, |o| o.island),
            ObstacleId::Added(u) => attempt.owners.get(&u).map_or(ROUTE_ISLAND, |c| self.shapes[*c].island),
        }
    }

    /// Whether an obstacle is a via of the captured board.
    fn is_via(&self, id: ObstacleId) -> bool {
        matches!(id, ObstacleId::Source(i) if i >= self.lands + self.tracks.len())
    }

    /// The item an obstacle belongs to, or `None` for copper that never moves: pads, locked
    /// copper, other layers' tracks and copper joined so that moving it would detach something.
    fn item_of(&mut self, attempt: &Attempt<'a>, id: ObstacleId) -> Result<Option<Item>, Halt> {
        match id {
            ObstacleId::Added(u) if attempt.route_ids.contains(&u) => Ok(Some(Item::Route)),
            ObstacleId::Added(u) => Ok(attempt.owners.get(&u).map(|c| Item::Chain(*c))),
            ObstacleId::Source(i) if self.dragged.iter().any(|(d, _, _)| *d == i) => Ok(Some(Item::Dragged(i))),
            ObstacleId::Source(i) if i < self.lands => Ok(None),
            ObstacleId::Source(i) if i < self.lands + self.tracks.len() => Ok(self.chain(i)?.map(Item::Chain)),
            ObstacleId::Source(i) => Ok(self.joint(i)?.then_some(Item::Via(i))),
        }
    }

    /// The movable chain through track `t`, built once: `None` when the track is locked, on
    /// another layer, joined in mid-span, part of a dragged run or of a closed loop.
    fn chain(&mut self, t: usize) -> Result<Option<usize>, Halt> {
        if let Some(chain) = self.chain_of.get(&t) {
            return Ok(*chain);
        }
        let chain = self.build_chain(t)?;
        let members = chain.as_ref().map_or_else(|| vec![t], |shape| shape.tracks.clone());
        let index = chain.map(|shape| {
            self.shapes.push(shape);
            self.shapes.len() - 1
        });
        for member in members {
            self.chain_of.insert(member, index);
        }
        Ok(index)
    }

    /// Follow track `t` through two-track joints both ways and verify the run can move: every
    /// copper touching the run must touch one of its two end discs. Such copper stays joined
    /// however the run is reshaped, because the ends never move, even when it also overlaps
    /// the run further in; copper touching only mid-span would be detached, so it pins the run.
    fn build_chain(&self, t: usize) -> Result<Option<Shape>, Halt> {
        let query = self.query;
        let objects = query.objects();
        let track = &self.tracks[t - self.lands];
        if track.layer != self.request.layer {
            return Ok(None);
        }
        let mut run: VecDeque<usize> = VecDeque::from([t]);
        let (mut points, mut seen) = (VecDeque::from([track.start, track.end]), BTreeSet::from([t]));
        for forward in [true, false] {
            let (mut at, mut from) = (if forward { track.end } else { track.start }, t);
            while let Some(next) = self.through(at, from)? {
                if !seen.insert(next) || run.len() >= MAX_PATH_POINTS - 1 {
                    return Ok(None);
                }
                let other = &self.tracks[next - self.lands];
                at = if other.start == at { other.end } else { other.start };
                if forward {
                    run.push_back(next);
                    points.push_back(at);
                } else {
                    run.push_front(next);
                    points.push_front(at);
                }
                from = next;
            }
        }
        let (run, points): (Vec<usize>, Vec<Point>) = (run.into(), points.into());
        if run.iter().any(|i| self.tracks[i - self.lands].locked || self.dragged.iter().any(|(d, _, _)| d == i)) {
            return Ok(None);
        }
        let half = track.width_nm as f64 / 2.;
        let (first, last) = (points[0], points[points.len() - 1]);
        if first == last {
            return Ok(None);
        }
        let discs = [Contour::capsule(first, first, half), Contour::capsule(last, last, half)];
        for &i in &run {
            let contour = &objects[i].contour;
            for other in self.query.candidates(contour, &self.layer)? {
                if run.contains(&other) || objects[other].contour.gap(contour) > 0. {
                    continue;
                }
                if discs.iter().all(|disc| objects[other].contour.gap(disc) > 0.) {
                    return Ok(None);
                }
            }
        }
        let posture = if points.windows(2).all(|leg| leg[0].x_nm == leg[1].x_nm || leg[0].y_nm == leg[1].y_nm) {
            Posture::Orthogonal
        } else {
            Posture::Diagonal
        };
        Ok(Some(Shape {
            ids: run.iter().map(|i| self.tracks[i - self.lands].id).collect(),
            nets: objects[t].intended_nets.clone(),
            island: objects[t].island,
            tracks: run,
            original: points,
            half,
            width_nm: track.width_nm,
            posture,
        }))
    }

    /// The track continuing a chain through `at` from track `from`: exactly two equal-width
    /// track ends meet there and no other copper touches the joint.
    fn through(&self, at: Point, from: usize) -> Result<Option<usize>, Halt> {
        let Some(meeting) = self.ends.get(&(at.x_nm, at.y_nm)) else { return Ok(None) };
        let others: Vec<usize> = meeting.iter().copied().filter(|i| *i != from).collect();
        let [next] = others[..] else { return Ok(None) };
        let width = self.tracks[from - self.lands].width_nm;
        if self.tracks[next - self.lands].width_nm != width {
            return Ok(None);
        }
        let disc = Contour::capsule(at, at, width as f64 / 2.);
        let objects = self.query.objects();
        let touching = self.query.candidates(&disc, &self.layer)?.into_iter().any(|i| i != from && i != next && objects[i].contour.gap(&disc) <= 0.);
        Ok((!touching).then_some(next))
    }

    /// Whether via `v` can move, built once: unlocked, and touched only by movable chains on
    /// the shove layer that end at its centre.
    fn joint(&mut self, v: usize) -> Result<bool, Halt> {
        if let Some(joint) = self.joints.get(&v) {
            return Ok(joint.is_some());
        }
        let joint = self.build_joint(v)?;
        let movable = joint.is_some();
        self.joints.insert(v, joint);
        Ok(movable)
    }

    /// Collect the chains ending on via `v`, or `None` when it must stay.
    fn build_joint(&mut self, v: usize) -> Result<Option<Joint>, Halt> {
        let via = &self.vias[v - self.lands - self.tracks.len()];
        if via.locked {
            return Ok(None);
        }
        let at = via.at;
        let query = self.query;
        let objects = query.objects();
        let contour = objects[v].contour.clone();
        let contacts: Vec<usize> =
            self.query.candidates(&contour, &objects[v].layers)?.into_iter().filter(|i| *i != v && objects[*i].contour.gap(&contour) <= 0.).collect();
        let mut chains = Vec::new();
        for i in contacts {
            if i < self.lands || i >= self.lands + self.tracks.len() {
                return Ok(None);
            }
            let track = &self.tracks[i - self.lands];
            if track.layer != self.request.layer || (track.start != at && track.end != at) {
                return Ok(None);
            }
            let Some(c) = self.chain(i)? else { return Ok(None) };
            let shape = &self.shapes[c];
            let last = shape.original.last() == Some(&at);
            if shape.original.first() == Some(&at) && last {
                return Ok(None);
            }
            if !chains.contains(&(c, last)) {
                chains.push((c, last));
            }
        }
        Ok(Some(Joint { chains }))
    }

    /// Rewrite chain `c`'s overlay segments for `points`: source tracks by position, extra
    /// segments as added obstacles, unused source tracks hidden.
    fn set_chain(&self, attempt: &mut Attempt<'a>, c: usize, points: &[Point]) -> Result<(), Halt> {
        let shape = &self.shapes[c];
        let mut extra = attempt.chains.remove(&c).map(|(_, extra)| extra).unwrap_or_default();
        for id in extra.drain(..) {
            attempt.overlay.remove_added(id)?;
            attempt.owners.remove(&id);
        }
        let legs = points.len().saturating_sub(1);
        for (k, leg) in points.windows(2).enumerate() {
            let contour = Contour::capsule(leg[0], leg[1], shape.half);
            match shape.tracks.get(k) {
                Some(&source) => attempt.overlay.replace(source, contour, self.layer.to_vec())?,
                None => {
                    let id = attempt.fresh();
                    attempt.overlay.add(id, contour, self.layer.to_vec(), Some(shape.tracks[0]))?;
                    attempt.owners.insert(id, c);
                    extra.push(id);
                }
            }
        }
        for &source in shape.tracks.iter().skip(legs) {
            attempt.overlay.remove(source)?;
        }
        attempt.chains.insert(c, (points.to_vec(), extra));
        attempt.route_nets = None;
        Ok(())
    }

    /// Move via `v`'s overlay copy to `at`.
    fn set_via(&self, attempt: &mut Attempt<'a>, v: usize, at: Point) -> Result<(), Halt> {
        let object = &self.query.objects()[v];
        attempt.overlay.replace(v, Contour::capsule(at, at, object.contour.radius), object.layers.clone())?;
        attempt.vias.insert(v, at);
        attempt.route_nets = None;
        Ok(())
    }

    /// Via `v`'s current centre.
    fn via_at(&self, attempt: &Attempt<'a>, v: usize) -> Point {
        attempt.vias.get(&v).copied().unwrap_or(self.query.objects()[v].at)
    }

    /// The query address of an obstacle for reports; a pushed chain's new segment is named by
    /// the chain's first track, and the route by nothing.
    fn name(&self, attempt: &Attempt<'a>, id: ObstacleId) -> Option<String> {
        match id {
            ObstacleId::Source(i) => self.query.objects().get(i).map(|o| o.name.clone()),
            ObstacleId::Added(u) => attempt.owners.get(&u).and_then(|c| self.item_name(Item::Chain(*c))),
        }
    }

    /// The query address naming an item: its first track or its via.
    fn item_name(&self, item: Item) -> Option<String> {
        let objects = self.query.objects();
        match item {
            Item::Route => None,
            Item::Dragged(i) | Item::Via(i) => objects.get(i).map(|o| o.name.clone()),
            Item::Chain(c) => self.shapes.get(c).and_then(|s| objects.get(s.tracks[0])).map(|o| o.name.clone()),
        }
    }

    /// The board edits an attempt describes, with stable identities: a pushed chain's segments
    /// equal to an original keep its identity, the rest reuse the remaining identities in path
    /// order, then add segments or remove unused tracks.
    fn edits(&self, attempt: &Attempt<'a>) -> Edits {
        let mut edits = Edits::default();
        for (c, (points, _)) in &attempt.chains {
            let shape = &self.shapes[*c];
            if *points == shape.original {
                continue;
            }
            let originals: Vec<(Point, Point)> = shape.original.windows(2).map(|leg| (leg[0], leg[1])).collect();
            let mut used = vec![false; originals.len()];
            let segments: Vec<(Point, Point)> = points.windows(2).map(|leg| (leg[0], leg[1])).collect();
            let mut slot: Vec<Option<usize>> = segments
                .iter()
                .map(|&(a, b)| {
                    let k = originals.iter().enumerate().position(|(k, &(c, d))| !used[k] && ((a, b) == (c, d) || (a, b) == (d, c)))?;
                    used[k] = true;
                    Some(k)
                })
                .collect();
            let mut free = (0..originals.len()).filter(|k| !used[*k]).collect::<VecDeque<_>>();
            for (j, &(start, end)) in segments.iter().enumerate() {
                if slot[j].is_some() {
                    continue;
                }
                match free.pop_front() {
                    Some(k) => {
                        slot[j] = Some(k);
                        edits.reshaped.push(Ends { track: shape.ids[k], start, end });
                    }
                    None => edits.added.push(Added { like: shape.ids[0], start, end }),
                }
            }
            edits.removed.extend(free.into_iter().map(|k| shape.ids[k]));
        }
        for (&v, &at) in &attempt.vias {
            if at != self.query.objects()[v].at {
                edits.vias.push(MovedVia { via: self.vias[v - self.lands - self.tracks.len()].id, at });
            }
        }
        edits
    }

    /// The plan for a settled attempt.
    fn finish(&self, attempt: Attempt<'a>, outcome: Outcome) -> Shove {
        let edits = self.edits(&attempt);
        let dragged = match &self.request.shover {
            Shover::Drag { tracks } if outcome == Outcome::Reached => tracks.clone(),
            _ => Vec::new(),
        };
        Shove {
            moved: edits.moved(),
            points: attempt.route,
            dragged,
            reshaped: edits.reshaped,
            added: edits.added,
            removed: edits.removed,
            vias: edits.vias,
            outcome,
            blocker: None,
            probes: self.probes,
            depth: attempt.depth,
        }
    }
}

impl Attempt<'_> {
    /// A deterministic identity for a disposable overlay obstacle; never saved.
    fn fresh(&mut self) -> Uuid {
        self.next_id += 1;
        Uuid::from_u128(self.next_id)
    }
}

/// Copper edits of pushed chains and vias.
#[derive(Default)]
struct Edits {
    reshaped: Vec<Ends>,
    added: Vec<Added>,
    removed: Vec<Uuid>,
    vias: Vec<MovedVia>,
}

impl Edits {
    /// Track segments and vias changed, added or removed.
    fn moved(&self) -> usize {
        self.reshaped.len() + self.added.len() + self.removed.len() + self.vias.len()
    }
}

/// Replace the part of `points` inside `hull` with the hull boundary between where the run first
/// enters and last leaves it, in the given sense. `None` when an end of the run lies inside the
/// hull (ends never move) or the run does not enter it.
fn bend(points: &[Point], mut hull: Hull, ccw: bool) -> Option<Vec<Point>> {
    let (&first, &last) = (points.first()?, points.last()?);
    if hull.contains(first) || hull.contains(last) {
        return None;
    }
    let (i, enter) = entry(points, &mut hull)?;
    let reversed: Vec<Point> = points.iter().rev().copied().collect();
    let (r, leave) = entry(&reversed, &mut hull)?;
    let j = points.len().checked_sub(2 + r)?;
    if j < i {
        return None;
    }
    let mut out = points[..=i].to_vec();
    out.push(enter);
    out.extend(hull.path(enter, leave, ccw)?);
    out.push(leave);
    out.extend_from_slice(&points[j + 1..]);
    Some(simplify(&out))
}

/// Move one end of a chain to `at`, replacing the leg into it with an elbow. `None` when the
/// end would collapse or the elbow would turn sharper than 90°.
fn follow(points: &[Point], last: bool, at: Point, posture: Posture, original: &[Point]) -> Option<Vec<Point>> {
    let mut run: Vec<Point> = if last { points.to_vec() } else { points.iter().rev().copied().collect() };
    run.pop()?;
    let from = *run.last()?;
    if from == at {
        return None;
    }
    let candidates = [false, true].into_iter().map(|flipped| {
        let mut moved = run.clone();
        moved.extend(elbow(from, at, posture, flipped).into_iter().skip(1));
        simplify(&moved)
    });
    let mut moved = candidates.into_iter().find(|moved| !sharp(moved, original))?;
    if !last {
        moved.reverse();
    }
    Some(moved)
}

/// Whether a corner the shove made (one not in the original run) turns by more than 90°.
fn sharp(points: &[Point], original: &[Point]) -> bool {
    points.windows(3).any(|w| {
        let (a, b, c) = (w[0], w[1], w[2]);
        let turn = i128::from(b.x_nm - a.x_nm) * i128::from(c.x_nm - b.x_nm) + i128::from(b.y_nm - a.y_nm) * i128::from(c.y_nm - b.y_nm);
        turn < 0 && !original.contains(&b)
    })
}

/// Total centre-line length in nanometres.
fn length(points: &[Point]) -> f64 {
    points.windows(2).map(|leg| ((leg[1].x_nm - leg[0].x_nm) as f64).hypot((leg[1].y_nm - leg[0].y_nm) as f64)).sum()
}
