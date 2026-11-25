//! Deterministic track displacement over disposable copper, with optional through-via motion.
//! The shared compound transaction checks every proposed change before publication.
use super::*;
use crate::board::{
    clearance::Object,
    geometry::Contour,
    track::{
        MAX_TRACKS,
        chains::{self, Chains},
        replacements::Replacement,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

mod corridor;
mod joints;
use crate::board::clearance::{RouteFragments, RouteObstacles};
pub(super) use joints::drag;

/// A coordinated retry reopens the directly hit runs with shared spacing.
#[derive(Clone, Copy)]
enum Cascade {
    Walk { reverse: bool, flipped: bool, posture: Posture, keep_bends: bool },
    Shared { positive: bool, advance: i64 },
}

/// Track routing policy; walkaround preserves the pre-shove default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Walkaround,
    /// Show collisions on the preferred path and refuse an obstructed edit.
    Highlight,
    Shove,
    /// Allow an incoming route to displace through vias and their attached endpoints.
    ShoveVias,
}

impl Mode {
    /// Whether the policy may move copper outside the active gesture.
    pub fn shoves(self) -> bool {
        matches!(self, Self::Shove | Self::ShoveVias)
    }

    /// A shove can affect many independent runs. Give its cooperatively stepped
    /// search the full allowed budget, with the smaller default for walkaround.
    pub fn default_probe_budget(self) -> usize {
        match self {
            Self::Walkaround | Self::Highlight => DEFAULT_BUDGET,
            Self::Shove | Self::ShoveVias => MAX_BUDGET,
        }
    }
}

/// Copper already fixed by the current gesture, never a live model edit. The final route
/// transaction validates these pieces too; they only guide speculative displacement here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FixedCopper {
    Segment { start: Point, end: Point, layer: Uuid, width_nm: i64 },
    Via { at: Point, diameter_nm: i64 },
}
impl FixedCopper {
    /// Recover the gesture pickup from its first fixed segment for command clients.
    pub(super) fn origin(&self) -> Option<(Point, Uuid)> {
        match self {
            Self::Segment { start, layer, .. } => Some((*start, *layer)),
            Self::Via { .. } => None,
        }
    }
    /// Resolve supported exact copper and explicit layers without mutating a board.
    fn geometry(&self, query: &ClearanceQuery) -> Result<(Contour, Vec<Uuid>), CommandError> {
        match *self {
            Self::Segment { start, end, layer, width_nm } => {
                if start == end {
                    return Err(CommandError::InvalidParams("Fixed route segments need distinct ends.".into()));
                }
                Ok((Contour::segment(start, end, width_nm)?, vec![layer]))
            }
            Self::Via { at, diameter_nm } => {
                if !(crate::board::track::MIN_VIA_DRILL_NM + 1..=crate::board::track::MAX_VIA_DIAMETER_NM).contains(&diameter_nm) {
                    return Err(CommandError::InvalidParams("Fixed route via diameter is out of range.".into()));
                }
                let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
                let layers = view.layers().into_iter().filter(|l| l.purpose == crate::board::setup::LayerPurpose::Copper).map(|l| l.id).collect();
                Ok((Contour::segment(at, at, diameter_nm)?, layers))
            }
        }
    }
}

/// Work retained for diagnostics. Geometry bytes count contour/point payload, excluding
/// allocator overhead and the source query/index; process-resource measurements remain separate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ShoveStats {
    /// Incoming-route and displacement attempts charged to the shared gesture budget.
    pub attempts: usize,
    pub displaced_tracks: usize,
    pub queued_tracks: usize,
    pub candidate_hits: usize,
    pub maximum_queue: usize,
    pub overlay_objects: usize,
    pub fixed_anchor_objects: usize,
    pub overlay_geometry_bytes: usize,
    /// The incoming path detoured around fixed obstacles before track propagation.
    pub used_walkaround: bool,
}

impl ShoveStats {
    /// Combine bounded attempts: collision/queue work adds, while retained geometry payload
    /// reports the larger attempt. Displaced tracks describe the final attempt's candidate.
    fn include_previous(&mut self, previous: &Self) {
        self.attempts += previous.attempts;
        self.queued_tracks += previous.queued_tracks;
        self.candidate_hits += previous.candidate_hits;
        self.maximum_queue = self.maximum_queue.max(previous.maximum_queue);
        self.overlay_objects = self.overlay_objects.max(previous.overlay_objects);
        self.fixed_anchor_objects = self.fixed_anchor_objects.max(previous.fixed_anchor_objects);
        self.overlay_geometry_bytes = self.overlay_geometry_bytes.max(previous.overlay_geometry_bytes);
        self.used_walkaround |= previous.used_walkaround;
    }
}

/// Reserve most probe work for reaching a full cascade before optimizing bends.
const REFINEMENT_BUDGET_DIVISOR: usize = 8;

/// Validate fixed geometry before starting a continuation; bounded count and explicit layers
/// prevent callers from growing work outside ordinary gesture/model limits.
pub(super) fn validate_fixed(query: &ClearanceQuery, fixed: &[FixedCopper]) -> Result<(), CommandError> {
    if fixed.len() > super::super::track::MAX_PATH_POINTS * 2 {
        return Err(CommandError::InvalidParams("Too much fixed route geometry.".into()));
    }
    for piece in fixed {
        let (_, layers) = piece.geometry(query)?;
        query.validate_layers(&layers)?;
    }
    Ok(())
}

/// Yield and charge an exact collision query under the same global budget as walkaround.
async fn charge(progress: &AtomicUsize, budget: usize) -> bool {
    super::search::yield_unit().await;
    if progress.load(Ordering::Relaxed) >= budget {
        return false;
    }
    progress.fetch_add(1, Ordering::Relaxed);
    true
}

/// Source-qualified membership and physical contacts at the immutable gesture pickup.
#[derive(Clone)]
struct Pickup {
    source: Option<usize>,
    nets: Vec<usize>,
    islands: BTreeSet<usize>,
}

/// The fixed segment at the original pickup supplies its width when the active leg differs.
pub(super) fn pickup_geometry(request: &Request) -> (Point, Uuid, i64) {
    let (point, layer) = request.origin.unwrap_or((request.head, request.layer));
    let width = request
        .fixed
        .iter()
        .find_map(|piece| match piece {
            FixedCopper::Segment { start, layer: fixed_layer, width_nm, .. } if *start == point && *fixed_layer == layer => Some(*width_nm),
            _ => None,
        })
        .unwrap_or(request.width_nm);
    (point, layer, width)
}

/// Resolve source intent only at the gesture's original pickup, before any speculative contact.
fn pickup(query: &ClearanceQuery, request: &Request) -> Result<Pickup, CommandError> {
    let (point, layer, width) = pickup_geometry(request);
    let contour = Contour::segment(point, point, width)?;
    let contacts: Vec<_> = query.candidates(&contour, &[layer])?.into_iter().filter(|i| query.objects()[*i].contour.touches(&contour)).collect();
    let nets = contacts.iter().flat_map(|i| query.objects()[*i].intended_nets.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
    let islands = contacts.iter().map(|i| query.objects()[*i].island).collect();
    Ok(Pickup { source: contacts.first().copied(), nets, islands })
}

/// Retain the first colliding route piece's direction throughout its displacement cascade.
#[derive(Clone, Copy)]
struct Push {
    from: Point,
    toward: Point,
    piece: usize,
    hit: usize,
    collision: [Point; 2],
}

/// Source queue with the originating route-piece direction retained through a cascade.
struct Propagation<'a> {
    chains: &'a Chains,
    movable: &'a BTreeSet<usize>,
    replaced: &'a BTreeSet<usize>,
    queue: &'a mut BTreeSet<usize>,
    pushes: &'a mut BTreeMap<usize, Push>,
    incoming: Option<Push>,
    contact_exemptions: Option<&'a BTreeSet<String>>,
    stats: &'a mut ShoveStats,
    progress: &'a AtomicUsize,
    budget: usize,
}
impl Propagation<'_> {
    /// Inspect one proposed path against originals still present, coalescing its neighbors.
    async fn enqueue(
        &mut self,
        query: &ClearanceQuery,
        pieces: &[(Contour, Vec<Uuid>)],
        nets: &[usize],
        islands: &BTreeSet<usize>,
    ) -> Result<bool, CommandError> {
        for (piece, (contour, layers)) in pieces.iter().enumerate() {
            if !charge(self.progress, self.budget).await {
                return Ok(false);
            }
            let result = query.probe_membership_excluding(contour, layers, nets, |i| self.replaced.contains(&self.chains.owner[i]))?;
            self.stats.candidate_hits += result.decisions.len();
            for hit in result.decisions {
                let owner = self.chains.owner[hit.second];
                let object = &query.objects()[hit.second];
                if object.mechanical && hit.clearance_violation {
                    return Ok(false);
                }
                let exempt = self.contact_exemptions.map_or_else(
                    || islands.contains(&object.island) || (nets.len() == 1 && object.intended_nets == nets),
                    |contacts| contacts.contains(&object.name),
                );
                if exempt && !hit.conflicting_nets {
                    continue;
                }
                if hit.gap_nm >= hit.clearance_nm as f64
                    && !(hit.touches && !nets.is_empty() && !object.intended_nets.is_empty() && nets != object.intended_nets)
                {
                    continue;
                }
                if !self.movable.contains(&hit.second) {
                    return Ok(false);
                }
                if self.queue.insert(owner) {
                    self.pushes.entry(owner).or_insert_with(|| {
                        let collision = [contour.core[0], contour.core[1]];
                        let mut push = self.incoming.unwrap_or(Push { from: collision[0], toward: collision[1], piece, hit: hit.second, collision });
                        push.hit = hit.second;
                        push.collision = collision;
                        push
                    });
                    self.stats.queued_tracks += self.chains.units[&owner].members.len();
                }
            }
            self.stats.maximum_queue = self.stats.maximum_queue.max(self.queue.iter().map(|i| self.chains.units[i].members.len()).sum());
        }
        Ok(true)
    }
}

/// Append one speculative path in bounded work units, preserving original membership.
async fn add_path(
    overlay: &mut RouteFragments,
    base: &ClearanceQuery,
    pieces: &[(Contour, Vec<Uuid>)],
    source: Option<usize>,
    stats: &mut ShoveStats,
) -> Result<(), CommandError> {
    for (contour, layers) in pieces {
        super::search::yield_unit().await;
        overlay.push(base, contour.clone(), layers.clone(), source)?;
        stats.overlay_objects += 1;
        stats.overlay_geometry_bytes += std::mem::size_of::<Contour>() + layers.len() * std::mem::size_of::<Uuid>();
    }
    Ok(())
}

/// Visit obstacles in the order the incoming direction meets their source lines, so near
/// tracks push later neighbors ahead rather than crossing an already-displaced farther track.
fn push_order(object: &Object, push: Push) -> f64 {
    let (Some(start), Some(end)) = (object.contour.core.first(), object.contour.core.get(1)) else { return 0. };
    let (dx, dy) = ((end.x_nm - start.x_nm) as f64, (end.y_nm - start.y_nm) as f64);
    let (mx, my) = ((push.toward.x_nm - push.from.x_nm) as f64, (push.toward.y_nm - push.from.y_nm) as f64);
    let denominator = dx * my - dy * mx;
    if denominator != 0. {
        (dx * (start.y_nm - push.from.y_nm) as f64 - dy * (start.x_nm - push.from.x_nm) as f64) / denominator
    } else {
        let length = mx * mx + my * my;
        if length == 0. { 0. } else { ((object.at.x_nm - push.from.x_nm) as f64 * mx + (object.at.y_nm - push.from.y_nm) as f64 * my) / length }
    }
}

/// Give individual walks and shared lanes the same collision order.
fn compare_pushes(query: &ClearanceQuery, pushes: &BTreeMap<usize, Push>, a: &usize, b: &usize) -> std::cmp::Ordering {
    let (first, second) = (pushes[a], pushes[b]);
    first
        .piece
        .cmp(&second.piece)
        .then_with(|| push_order(&query.objects()[first.hit], first).total_cmp(&push_order(&query.objects()[second.hit], second)))
        .then(a.cmp(b))
}

/// Partition a certified chain into source-owned paths. Subdividing existing
/// lattice legs retains every original UUID without changing copper or posture.
/// Insufficient whole-nanometre length refuses the candidate rather than losing IDs.
fn split_chain(chains: &Chains, source: usize, path: &[Point]) -> Option<Vec<Replacement>> {
    let chain = chains.units.get(&source)?;
    let mut boundaries = vec![(0, 0)];
    let mut cursor = 0;
    for (position, (member, reverse)) in chain.members.iter().enumerate().take(chain.members.len() - 1) {
        let track = &chains.tracks[member];
        let point = if *reverse { track.start } else { track.end };
        if let Some(offset) = path[cursor + 1..].iter().position(|p| *p == point) {
            cursor += offset + 1;
            boundaries.push((position + 1, cursor));
        }
    }
    boundaries.push((chain.members.len(), path.len() - 1));
    let mut result = Vec::new();
    for pair in boundaries.windows(2) {
        let (first, last) = (pair[0].0, pair[1].0);
        let count = last - first;
        let mut points = path[pair[0].1..=pair[1].1].to_vec();
        while points.len() <= count {
            let (index, steps) = points.windows(2).enumerate().map(|(i, p)| (i, super::posture::lattice(p[0], p[1]).0)).max_by_key(|p| p.1)?;
            if steps < 2 || points.len() >= chains::MAX_POINTS {
                return None;
            }
            points.insert(index + 1, super::posture::along(points[index], points[index + 1], steps / 2, steps));
        }
        let mut start = 0;
        for (position, (member, reverse)) in chain.members[first..last].iter().enumerate() {
            let end = (position + 1) * (points.len() - 1) / count;
            let mut part = super::walkaround::simplify(&points[start..=end]);
            if *reverse {
                part.reverse();
            }
            let track = &chains.tracks[member];
            if part != [track.start, track.end] {
                result.push(Replacement { track: track.id, points: part });
            }
            start = end;
        }
    }
    Some(result)
}

/// Validate the original pickup before making any speculative copper movable. All
/// candidates share the same captured source and probe budget; cancellation drops the search.
pub(super) async fn run(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Plan, CommandError> {
    // A free pickup inside existing clearance is invalid even when that copper
    // could later move. Validate the original contact before removing any bodies.
    let (point, layer, width) = pickup_geometry(request);
    let start = Request {
        head: point,
        target: point,
        layer,
        width_nm: width,
        origin: None,
        previous: Vec::new(),
        fixed: Vec::new(),
        mode: Mode::Walkaround,
        ..request.clone()
    };
    let mut picker = super::walkaround::Walker::new(query, &start, budget, progress.clone())?;
    let rejected = match picker.displacement_anchor(point).await {
        Ok(true) => None,
        Ok(false) => Some((Outcome::Blocked { last_legal: request.head }, true)),
        Err(super::walkaround::Stop::Exhausted) => Some((Outcome::Incomplete, false)),
        Err(super::walkaround::Stop::Failed(error)) => return Err(error),
    };
    if let Some((outcome, start_blocked)) = rejected {
        return Ok(Plan {
            adjusted_target: None,
            start_blocked,
            via_moves: Vec::new(),
            replacements: Vec::new(),
            shove: Some(ShoveStats::default()),
            points: vec![request.head],
            outcome,
            blocker: None,
            collisions: Default::default(),
            probes: progress.load(Ordering::Relaxed),
        });
    }
    let chains = match chains::capture(query).await {
        Ok(chains) => chains,
        Err(CommandError::CapExceeded(_)) => {
            return Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                via_moves: Vec::new(),
                replacements: Vec::new(),
                shove: Some(ShoveStats::default()),
                points: vec![request.head],
                outcome: Outcome::Incomplete,
                blocker: None,
                collisions: Default::default(),
                probes: progress.load(Ordering::Relaxed),
            });
        }
        Err(error) => return Err(error),
    };
    let pickup = pickup(query, request)?;
    if request.mode == Mode::ShoveVias
        && let Some(plan) = joints::Motion::route(query, &chains, request, &pickup, budget, progress.clone()).await?
    {
        return Ok(plan);
    }
    Search::prepare(query, &chains, request, budget, progress, pickup, &BTreeSet::new()).await?.solve(request).await
}

/// A previously certified guide is one candidate for the new cursor. The whole
/// continuation must be certified again and never becomes part of the fixed gesture.
enum Approach {
    Walk(bool),
    BoardEdge,
    Continue(Vec<Point>),
    /// D-drag supplies exact existing segment endpoints, including non-posture angles.
    Drag,
}

/// Preserve earlier turns when possible, trying both ways to connect the last corner
/// to the cursor. Try joining before or after the last leg and keep the path bound.
fn continuations(request: &Request) -> Vec<Approach> {
    let previous = &request.previous;
    if previous.len() < 2 || previous.first() != Some(&request.head) || previous.last() == Some(&request.target) {
        return Vec::new();
    }
    let mut options = Vec::new();
    for length in [previous.len() - 1, previous.len()] {
        let shared = &previous[..length];
        for flipped in [request.flipped, !request.flipped] {
            let tail = super::posture::elbow(shared[length - 1], request.target, request.posture, flipped);
            let mut path = shared.to_vec();
            path.extend(tail.into_iter().skip(1));
            let path = super::walkaround::simplify(&path);
            if path.len() <= super::super::track::MAX_PATH_POINTS {
                options.push(Approach::Continue(path));
            }
        }
    }
    options
}

/// Use the same frozen pickup, protected contacts and fixed copper for every incoming
/// candidate. A failed cascade drops its own overlay before the next candidate is tried.
struct Search<'a> {
    query: &'a ClearanceQuery,
    chains: &'a Chains,
    geometry: ClearanceQuery,
    incoming: RouteFragments,
    pickup: Pickup,
    drag_direction: Option<(Point, Point)>,
    fixed: Vec<(Contour, Vec<Uuid>)>,
    movable: BTreeSet<usize>,
    stats: ShoveStats,
    budget: usize,
    progress: Arc<AtomicUsize>,
}

impl<'a> Search<'a> {
    /// Capture immovable geometry once for the route search and all its cascades.
    async fn prepare(
        query: &'a ClearanceQuery,
        chains: &'a Chains,
        request: &Request,
        budget: usize,
        progress: Arc<AtomicUsize>,
        pickup: Pickup,
        frozen: &BTreeSet<usize>,
    ) -> Result<Self, CommandError> {
        let fixed: Vec<_> = request.fixed.iter().map(|p| p.geometry(query)).collect::<Result<_, _>>()?;
        let relevant_layers: BTreeSet<_> =
            std::iter::once(request.layer).chain(fixed.iter().flat_map(|(_, layers)| layers.iter().copied())).collect();
        let movable: BTreeSet<_> = chains
            .tracks
            .iter()
            .filter(|(i, t)| {
                !t.locked
                    && !frozen.contains(&chains.owner[**i])
                    && relevant_layers.contains(&t.layer)
                    && !pickup.islands.contains(&query.objects()[**i].island)
                    && chains.units.contains_key(&chains.owner[**i])
            })
            .map(|(i, _)| *i)
            .collect();
        let units: BTreeSet<_> = movable.iter().map(|i| chains.owner[*i]).collect();
        let mut stats = ShoveStats::default();
        // Free ends and protected contacts are fixed graph anchors. Ordinary degree-two
        // bends remain soft hints, so an earlier displacement may move them later.
        let mut items = query.objects().to_vec();
        for root in &units {
            super::search::yield_unit().await;
            let chain = &chains.units[root];
            let protected: Vec<_> = chain.anchors.iter().filter(|a| a.protected).collect();
            let Some(first) = protected.first() else { continue };
            for (source, _) in &chain.members {
                items[*source].contour = Contour::capsule(first.point, first.point, query.objects()[*source].contour.radius);
            }
            for point in protected.iter().skip(1) {
                let mut anchor = query.objects()[*root].clone();
                anchor.contour = Contour::capsule(point.point, point.point, anchor.contour.radius);
                items.push(anchor);
            }
            let caps = chain.members.len() + protected.len() - 1;
            stats.fixed_anchor_objects += caps;
            stats.overlay_geometry_bytes += caps * (std::mem::size_of::<Contour>() + std::mem::size_of::<Uuid>());
        }
        let geometry = query.with_objects(items)?;
        let mut incoming = RouteFragments::default();
        add_path(&mut incoming, &geometry, &fixed, pickup.source, &mut ShoveStats::default()).await?;
        incoming.finish_gesture();
        Ok(Self { query, chains, geometry, incoming, pickup, drag_direction: None, fixed, movable, stats, budget, progress })
    }

    /// Try both elbow directions before carrying turns forward from an earlier cursor target.
    /// All approaches use the same exact clearance and contact rules.
    async fn solve(&self, request: &Request) -> Result<Plan, CommandError> {
        let guides = [Approach::Walk(request.flipped), Approach::Walk(!request.flipped)]
            .into_iter()
            .chain([Approach::BoardEdge, Approach::BoardEdge])
            .chain(std::iter::once_with(|| continuations(request)).flatten());
        self.solve_guides(request, guides).await
    }

    /// Certify each guide and propagate its collisions under the same gesture budget.
    async fn solve_guides(&self, request: &Request, guides: impl Iterator<Item = Approach>) -> Result<Plan, CommandError> {
        let nets = &self.pickup.nets;
        let clearance_nm = self.query.membership_clearance(nets, nets);
        let reuse_previous = request.previous.last() != Some(&request.target) || legal_incoming(request, &request.previous, clearance_nm).await;
        let incoming = RouteObstacles::new(&self.geometry, &self.incoming);
        let fixed_segments = request.fixed.iter().filter(|piece| matches!(piece, FixedCopper::Segment { .. })).count();
        let mut tested = Vec::new();
        let mut edge_approaches = Vec::new();
        let mut stats = self.stats.clone();
        let mut blocker = None;
        let mut incomplete = false;
        for guide in guides {
            if self.progress.load(Ordering::Relaxed) >= self.budget {
                incomplete = true;
                break;
            }
            let mut route = request.clone();
            if !reuse_previous {
                route.previous.clear();
            }
            let mut proposed = match guide {
                Approach::Walk(flipped) => {
                    route.flipped = flipped;
                    let plan = super::search::walk(incoming, &route, self.budget, self.progress.clone()).await?;
                    if let Some(edge) = plan.adjusted_target {
                        // Try the prefix nearest the cursor first, so an elbow that
                        // hits the edge early cannot stop the preview along it.
                        let dx = i128::from(edge.x_nm) - i128::from(request.target.x_nm);
                        let dy = i128::from(edge.y_nm) - i128::from(request.target.y_nm);
                        edge_approaches.push((dx * dx + dy * dy, flipped, plan));
                        continue;
                    }
                    plan
                }
                Approach::BoardEdge => {
                    let Some((index, _)) = edge_approaches.iter().enumerate().min_by_key(|(_, (distance, _, _))| *distance) else {
                        continue;
                    };
                    let (_, flipped, plan) = edge_approaches.swap_remove(index);
                    route.flipped = flipped;
                    plan
                }
                Approach::Drag => Plan {
                    points: vec![request.head, request.target],
                    outcome: Outcome::Reached,
                    adjusted_target: None,
                    start_blocked: false,
                    replacements: Vec::new(),
                    via_moves: Vec::new(),
                    shove: None,
                    blocker: None,
                    collisions: Default::default(),
                    probes: self.progress.load(Ordering::Relaxed),
                },
                Approach::Continue(points) => {
                    if tested.contains(&points) {
                        continue;
                    }
                    let mut walker = super::walkaround::Walker::new(incoming, &route, self.budget, self.progress.clone())?;
                    match walker.certify_candidate(&points).await {
                        Ok(Some(plan)) => plan,
                        Ok(None) => {
                            tested.push(points);
                            continue;
                        }
                        Err(super::walkaround::Stop::Exhausted) => {
                            incomplete = true;
                            continue;
                        }
                        Err(super::walkaround::Stop::Failed(error)) => return Err(error),
                    }
                }
            };
            let adjusted_target = proposed.adjusted_target.take();
            if let Some(target) = adjusted_target {
                route.target = target;
                proposed.outcome = Outcome::Reached;
            }
            if proposed.outcome != Outcome::Reached {
                if proposed.start_blocked {
                    let mut refused = proposed;
                    refused.shove = Some(stats);
                    return Ok(refused);
                }
                incomplete |= proposed.outcome == Outcome::Incomplete;
                blocker = proposed.blocker.or(blocker);
                continue;
            }
            if tested.contains(&proposed.points) {
                continue;
            }
            tested.push(proposed.points.clone());
            // Dragged pieces can branch. Only an incoming route is one contiguous stroke.
            if self.drag_direction.is_none() {
                if fixed_segments + proposed.points.len() > crate::board::track::MAX_PATH_POINTS {
                    incomplete = true;
                    continue;
                }
                if !legal_incoming(&route, &proposed.points, clearance_nm).await {
                    continue;
                }
            }
            let orders = [(false, route.flipped), (true, route.flipped), (false, !route.flipped), (true, !route.flipped)];
            let walk = orders.map(|(reverse, flipped)| Cascade::Walk { reverse, flipped, posture: route.posture, keep_bends: true });
            // A square return is also a legal diagonal-route candidate. Try it
            // when a short chamfer would break the full run's return spacing.
            let square = (route.posture == Posture::Diagonal)
                .then(|| orders.map(|(reverse, flipped)| Cascade::Walk { reverse, flipped, posture: Posture::Orthogonal, keep_bends: true }))
                .into_iter()
                .flatten();
            // A short move can still need a deeper shove when a distant leg
            // blocks the group's return. Sample room ahead in clearance-sized
            // steps that grow with the approach and share its probe budget.
            let first = route.width_nm + self.query.authority().clearance_nm() + 2;
            let extent = (route.target.x_nm - route.head.x_nm).abs().max((route.target.y_nm - route.head.y_nm).abs());
            let advances =
                std::iter::once(0).chain(std::iter::successors(Some(first), |step| step.checked_mul(2)).take_while(|step| *step <= extent));
            let shared = advances.flat_map(|advance| [true, false].map(move |positive| Cascade::Shared { positive, advance }));
            // A bend may clear on its own but trap the return between it and a
            // neighboring attachment. Retry with only the protected anchors fixed.
            let reopen = orders.map(|(reverse, flipped)| Cascade::Walk { reverse, flipped, posture: route.posture, keep_bends: false });
            for strategy in walk.into_iter().chain(shared).chain(square).chain(reopen) {
                let mut attempt = self.propagate(&route, proposed.clone(), strategy).await?;
                let mut attempt_stats = attempt.shove.take().unwrap_or_default();
                let could_retry = attempt_stats.queued_tracks > 0 && self.progress.load(Ordering::Relaxed) < self.budget;
                attempt_stats.include_previous(&stats);
                stats = attempt_stats;
                if attempt.outcome == Outcome::Reached {
                    if let Some(last_legal) = adjusted_target {
                        attempt.outcome = Outcome::Blocked { last_legal };
                        attempt.adjusted_target = Some(last_legal);
                    }
                    attempt.shove = Some(stats);
                    return Ok(attempt);
                }
                incomplete |= attempt.outcome == Outcome::Incomplete;
                blocker = attempt.blocker.or(blocker);
                if !could_retry {
                    break;
                }
            }
        }
        let probes = self.progress.load(Ordering::Relaxed);
        let incomplete = incomplete || probes >= self.budget;
        Ok(Plan {
            adjusted_target: None,
            start_blocked: false,
            via_moves: Vec::new(),
            replacements: Vec::new(),
            shove: Some(stats),
            points: vec![request.head],
            outcome: if incomplete { Outcome::Incomplete } else { Outcome::Blocked { last_legal: request.head } },
            blocker: if incomplete { None } else { blocker },
            collisions: Default::default(),
            probes,
        })
    }

    /// Run a nested walk with inherited source intent; its probes charge the global gesture budget.
    async fn displace(
        &self,
        query: RouteObstacles<'_>,
        request: &Request,
        object: &Object,
        contacts: Option<BTreeSet<String>>,
        incoming: Push,
        refinement_remaining: &mut usize,
    ) -> Result<Plan, CommandError> {
        let budget = self.budget;
        let mut walker = super::walkaround::Walker::new(query, request, budget, self.progress.clone())?;
        walker.inherit(object, contacts);
        walker.push_forward(request.head, request.target, incoming.from, incoming.toward);
        let first = match walker.run(request).await {
            Ok(plan) if plan.outcome == Outcome::Reached => {
                return walker.polish_displacement(plan, request, refinement_remaining, true).await;
            }
            Ok(plan) if plan.start_blocked => return Ok(plan),
            Err(super::walkaround::Stop::Failed(error)) => return Err(error),
            first => first,
        };
        if walker.probes() < budget {
            match walker.parallel_alternatives(request).await {
                Ok(Some(plan)) => return walker.polish_displacement(plan, request, refinement_remaining, false).await,
                Err(super::walkaround::Stop::Failed(error)) => return Err(error),
                Err(super::walkaround::Stop::Exhausted) => {
                    return Ok(Plan {
                        adjusted_target: None,
                        start_blocked: false,
                        via_moves: Vec::new(),
                        replacements: Vec::new(),
                        shove: None,
                        points: vec![request.head],
                        outcome: Outcome::Incomplete,
                        blocker: None,
                        collisions: Default::default(),
                        probes: walker.probes(),
                    });
                }
                Ok(None) => {}
            }
        }
        match first {
            Ok(mut plan) => {
                plan.probes = walker.probes();
                Ok(plan)
            }
            Err(super::walkaround::Stop::Exhausted) => Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                via_moves: Vec::new(),
                replacements: Vec::new(),
                shove: None,
                points: vec![request.head],
                outcome: Outcome::Incomplete,
                blocker: None,
                collisions: Default::default(),
                probes: walker.probes(),
            }),
            Err(super::walkaround::Stop::Failed(error)) => Err(error),
        }
    }

    /// Displace a finite queue around one certified incoming path. Each strategy
    /// starts with a fresh overlay, and failure discards every speculative displacement.
    async fn propagate(&self, request: &Request, mut plan: Plan, strategy: Cascade) -> Result<Plan, CommandError> {
        let (query, chains, budget, progress) = (self.query, self.chains, self.budget, &self.progress);
        let Pickup { source: origin, nets, islands } = &self.pickup;
        let origin = *origin;
        let movable = &self.movable;
        let mut stats = self.stats.clone();
        stats.attempts = 1;
        stats.used_walkaround = plan.points != super::posture::elbow(request.head, request.target, request.posture, request.flipped);
        let fixed = &self.fixed;
        let mut overlay = RouteFragments::default();
        add_path(&mut overlay, &self.geometry, fixed, origin, &mut stats).await?;
        let mut incomplete = false;
        let pieces: Vec<_> =
            plan.points.windows(2).map(|p| (Contour::capsule(p[0], p[1], request.width_nm as f64 / 2.), vec![request.layer])).collect();
        add_path(&mut overlay, &self.geometry, &pieces, origin, &mut stats).await?;
        overlay.finish_gesture();
        let fixed_count = fixed.len();
        let mut route = fixed.clone();
        route.extend(pieces);
        let mut queue = BTreeSet::new();
        let mut pushes = BTreeMap::new();
        let mut replaced = BTreeSet::new();
        let fixed_segments = request.fixed.iter().filter(|piece| matches!(piece, FixedCopper::Segment { .. })).count();
        let mut segment_count = self.chains.tracks.len() + fixed_segments + plan.points.len().saturating_sub(1);
        let mut refinement_remaining = self.budget / REFINEMENT_BUDGET_DIVISOR;
        let mut replacements = Vec::new();
        let mut legal = segment_count <= MAX_TRACKS
            && Propagation {
                chains,
                movable,
                replaced: &replaced,
                queue: &mut queue,
                pushes: &mut pushes,
                incoming: None,
                contact_exemptions: None,
                stats: &mut stats,
                progress,
                budget,
            }
            .enqueue(query, &route, nets, islands)
            .await?;
        if let Some((from, toward)) = self.drag_direction {
            for push in pushes.values_mut() {
                push.from = from;
                push.toward = toward;
            }
        }
        incomplete |= segment_count > MAX_TRACKS;
        let (mut corridor, reverse_displacements, displacement_flipped, displacement_posture, keep_bends) = match strategy {
            Cascade::Walk { reverse, flipped, posture, keep_bends } => (None, reverse, flipped, posture, keep_bends),
            Cascade::Shared { .. } => {
                let corridor = corridor::Corridor::new(self, request, &plan, &route, &queue, &pushes, strategy);
                legal &= corridor.is_some();
                (corridor, false, request.flipped, request.posture, true)
            }
        };
        while legal && !queue.is_empty() {
            super::search::yield_unit().await;
            let Some(source) = queue.iter().copied().min_by(|a, b| compare_pushes(query, &pushes, a, b)) else {
                break;
            };
            queue.remove(&source);
            let incoming = pushes[&source];
            if !replaced.insert(source) {
                incomplete = true;
                legal = false;
                break;
            }
            let object = &query.objects()[source];
            let chain = &chains.units[&source];
            let track = &chains.tracks[&source];
            let contacts = (!request.merge_same_net).then(|| chain.contacts.clone());
            let obstacles = RouteObstacles::new(&self.geometry, &overlay);
            let path = if let Some(shared) = corridor.as_mut().filter(|_| track.layer == request.layer) {
                match shared.path(self, obstacles, request, source, incoming).await {
                    Ok(Some(path)) => path,
                    Ok(None) => {
                        plan.blocker = Some(object.name.clone());
                        legal = false;
                        break;
                    }
                    Err(super::walkaround::Stop::Exhausted) => {
                        incomplete = true;
                        legal = false;
                        break;
                    }
                    Err(super::walkaround::Stop::Failed(error)) => return Err(error),
                }
            } else {
                let mut retained = Vec::new();
                for (position, anchor) in chain.anchors.iter().enumerate() {
                    if anchor.protected {
                        retained.push(position);
                        continue;
                    }
                    if !keep_bends {
                        continue;
                    }
                    let probe = Request {
                        head: anchor.point,
                        target: anchor.point,
                        layer: track.layer,
                        width_nm: track.width_nm,
                        previous: Vec::new(),
                        origin: None,
                        fixed: Vec::new(),
                        mode: Mode::Walkaround,
                        merge_same_net: request.merge_same_net,
                        posture: displacement_posture,
                        flipped: request.flipped,
                    };
                    let mut walker = super::walkaround::Walker::new(obstacles, &probe, budget, progress.clone())?;
                    walker.inherit(object, contacts.clone());
                    match walker.displacement_anchor(anchor.point).await {
                        Ok(true) => retained.push(position),
                        Ok(false) => {}
                        Err(super::walkaround::Stop::Failed(error)) => return Err(error),
                        Err(super::walkaround::Stop::Exhausted) => {
                            incomplete = true;
                            legal = false;
                            break;
                        }
                    }
                }
                if !legal {
                    break;
                }
                let Some(first) = retained.first() else {
                    incomplete = true;
                    legal = false;
                    break;
                };
                let mut path = vec![chain.anchors[*first].point];
                for interval in retained.windows(2) {
                    let anchor = [chain.anchors[interval[0]].point, chain.anchors[interval[1]].point];
                    let mut displaced = Request {
                        head: anchor[0],
                        target: anchor[1],
                        layer: track.layer,
                        origin: None,
                        width_nm: track.width_nm,
                        posture: displacement_posture,
                        flipped: displacement_flipped,
                        previous: chain.anchors[interval[0]..=interval[1]].iter().map(|a| a.point).collect(),
                        mode: Mode::Walkaround,
                        merge_same_net: request.merge_same_net,
                        fixed: Vec::new(),
                    };
                    if reverse_displacements {
                        std::mem::swap(&mut displaced.head, &mut displaced.target);
                        displaced.flipped = !displaced.flipped;
                    }
                    let mut result = self.displace(obstacles, &displaced, object, contacts.clone(), incoming, &mut refinement_remaining).await?;
                    if result.outcome != Outcome::Reached {
                        incomplete |= result.outcome == Outcome::Incomplete;
                        plan.blocker = result.blocker.or_else(|| Some(object.name.clone()));
                        legal = false;
                        break;
                    }
                    if reverse_displacements {
                        result.points.reverse();
                    }
                    path.extend(result.points.into_iter().skip(1));
                    if path.len() > super::super::track::MAX_PATH_POINTS {
                        incomplete = true;
                        legal = false;
                        break;
                    }
                }
                if !legal {
                    break;
                }
                path
            };
            let Some(parts) = split_chain(chains, source, &path) else {
                incomplete = true;
                legal = false;
                break;
            };
            segment_count += parts.iter().map(|part| part.points.len().saturating_sub(2)).sum::<usize>();
            if segment_count > MAX_TRACKS {
                incomplete = true;
                legal = false;
                break;
            }
            if !chains.legal_replacements(query, chain, &parts, request.merge_same_net).await? {
                legal = false;
                plan.blocker = Some(object.name.clone());
                break;
            }
            let pieces: Vec<_> = path.windows(2).map(|p| (Contour::capsule(p[0], p[1], track.width_nm as f64 / 2.), vec![track.layer])).collect();
            add_path(&mut overlay, &self.geometry, &pieces, Some(source), &mut stats).await?;
            legal = Propagation {
                chains,
                movable,
                replaced: &replaced,
                queue: &mut queue,
                pushes: &mut pushes,
                incoming: Some(incoming),
                contact_exemptions: contacts.as_ref(),
                stats: &mut stats,
                progress,
                budget,
            }
            .enqueue(query, &pieces, &object.intended_nets, &[object.island].into_iter().collect())
            .await?;
            replacements.extend(parts);
            stats.displaced_tracks = replacements.len();
        }
        if legal && fixed_count != 0 {
            let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
            let outline = view.outline_edges();
            if !outline.polygons.is_empty() {
                let baseline = query.authority().rules().values;
                for (contour, layers) in &route[..fixed_count] {
                    if !charge(progress, budget).await {
                        incomplete = true;
                        legal = false;
                        break;
                    }
                    let result = query.probe_membership_excluding(contour, layers, nets, |i| replaced.contains(&chains.owner[i]))?;
                    let rules = result.probe_rules.as_ref().map_or(baseline, |p| p.hard_values(baseline));
                    if !outline.allows_copper(contour, rules.edge_clearance_nm) {
                        legal = false;
                        break;
                    }
                }
            }
        }
        if legal {
            replacements.sort_by_key(|r| r.track);
            plan.replacements = replacements;
            plan.probes = progress.load(Ordering::Relaxed);
            plan.shove = Some(stats);
            return Ok(plan);
        }
        // A head-only refusal lets interactive clients retain their last valid shove preview.
        // No unvalidated prefix or partial replacement can become placement authority.
        plan.points = vec![request.head];
        plan.probes = progress.load(Ordering::Relaxed);
        plan.outcome = if incomplete || plan.probes >= budget { Outcome::Incomplete } else { Outcome::Blocked { last_legal: request.head } };
        if incomplete || plan.probes >= budget {
            plan.blocker = None;
        }
        plan.shove = Some(stats);
        Ok(plan)
    }
}

/// Command schemas expose bounded primitive geometry rather than captured source indices.
pub(crate) fn fixed_schema() -> Value {
    let point = crate::point_schema();
    json!({"type":"array","maxItems":super::super::track::MAX_PATH_POINTS*2,"items":{"oneOf":[
        {"type":"object","properties":{"kind":{"const":"segment"},"start":point,"end":point,"layer":{"type":"string","format":"uuid"},"width_nm":{"type":"integer","minimum":super::super::track::MIN_WIDTH_NM,"maximum":super::super::track::MAX_WIDTH_NM}},"required":["kind","start","end","layer","width_nm"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"via"},"at":point,"diameter_nm":{"type":"integer","minimum":super::super::track::MIN_VIA_DRILL_NM+1,"maximum":super::super::track::MAX_VIA_DIAMETER_NM}},"required":["kind","at","diameter_nm"],"additionalProperties":false}
    ]}})
}

/// Validate catalog shapes before source capture; actual layer ownership is checked by planning.
pub(crate) fn valid_fixed(value: &Value) -> bool {
    serde_json::from_value::<Vec<FixedCopper>>(value.clone()).is_ok_and(|pieces| {
        pieces.len() <= super::super::track::MAX_PATH_POINTS * 2
            && pieces.iter().all(|piece| match piece {
                FixedCopper::Segment { start, end, layer, width_nm } => {
                    start != end && !layer.is_nil() && Contour::segment(*start, *end, *width_nm).is_ok()
                }
                FixedCopper::Via { at, diameter_nm } => {
                    crate::wire::valid_point(at)
                        && (super::super::track::MIN_VIA_DRILL_NM + 1..=super::super::track::MAX_VIA_DIAMETER_NM).contains(diameter_nm)
                }
            })
    })
}

#[cfg(test)]
mod tests;
