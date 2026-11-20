//! Deterministic track displacement over disposable copper. Pads, vias and locks stay fixed;
//! every returned replacement is still checked by the shared compound transaction.
use super::*;
use crate::board::{
    clearance::{Object, Overlay},
    geometry::Contour,
    track::{
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

/// Track routing policy; walkaround preserves the pre-shove default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Walkaround,
    Shove,
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
    /// Source-displacement passes, at most two under the shared gesture budget.
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

/// Bound both speculative propagation and connected source anchors independently of probes.
const MAX_SHOVES: usize = 96;

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
struct Pickup {
    source: Option<usize>,
    nets: Vec<usize>,
    islands: BTreeSet<usize>,
}

/// Resolve source intent only at the gesture's original pickup, before any speculative contact.
fn pickup(query: &ClearanceQuery, request: &Request) -> Result<Pickup, CommandError> {
    let (point, layer) = request.origin.unwrap_or((request.head, request.layer));
    let contour = Contour::segment(point, point, request.width_nm)?;
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
                        let mut push = self.incoming.unwrap_or(Push { from: contour.core[0], toward: contour.core[1], piece, hit: hit.second });
                        push.hit = hit.second;
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

/// Add one speculative connected path, preserving immutable original membership/island. IDs
/// address only this overlay and are deterministic; authored segment IDs are allocated at commit.
fn add_path(
    overlay: &mut Overlay<'_>,
    pieces: &[(Contour, Vec<Uuid>)],
    source: Option<usize>,
    next: &mut u128,
    stats: &mut ShoveStats,
) -> Result<(), CommandError> {
    for (contour, layers) in pieces {
        overlay.add(Uuid::from_u128(*next), contour.clone(), layers.clone(), source)?;
        *next += 1;
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

/// Run a nested walk with inherited source intent; its probes charge the global gesture budget.
async fn displace(
    query: &ClearanceQuery,
    request: &Request,
    object: &Object,
    contacts: Option<BTreeSet<String>>,
    incoming: Push,
    budget: usize,
    progress: Arc<AtomicUsize>,
) -> Result<Plan, CommandError> {
    let mut walker = super::walkaround::Walker::new(query, request, budget, progress)?;
    walker.inherit(object, contacts);
    walker.push_forward(request.head, request.target, incoming.from, incoming.toward);
    let first = match walker.run(request).await {
        Ok(plan) if plan.outcome == Outcome::Reached => {
            return walker.localize_displacement(plan, request).await;
        }
        Ok(plan) if plan.start_blocked => return Ok(plan),
        Err(super::walkaround::Stop::Failed(error)) => return Err(error),
        first => first,
    };
    if walker.probes() < budget {
        match walker.parallel_alternatives(request).await {
            Ok(Some(plan)) => return walker.tidy_displacement(plan, request.flipped).await,
            Err(super::walkaround::Stop::Failed(error)) => return Err(error),
            Err(super::walkaround::Stop::Exhausted) => {
                return Ok(Plan {
                    adjusted_target: None,
                    start_blocked: false,
                    replacements: Vec::new(),
                    shove: None,
                    points: vec![request.head],
                    outcome: Outcome::Incomplete,
                    blocker: None,
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
            replacements: Vec::new(),
            shove: None,
            points: vec![request.head],
            outcome: Outcome::Incomplete,
            blocker: None,
            probes: walker.probes(),
        }),
        Err(super::walkaround::Stop::Failed(error)) => Err(error),
    }
}

/// Validate the original pickup before removing movable bodies, then reuse captured
/// topology and try one alternate return direction after a blocked cascade. Both attempts
/// share the source/probe budget; cancellation drops only disposable work. Explicit search
/// exhaustion stays incomplete, and an invalid pickup cannot become a placement authority.
pub(super) async fn run(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Plan, CommandError> {
    // A free pickup inside existing clearance is invalid even when that copper
    // could later move. Validate the original contact before removing any bodies.
    let (point, layer) = request.origin.unwrap_or((request.head, request.layer));
    let width = request
        .fixed
        .iter()
        .find_map(|piece| match piece {
            FixedCopper::Segment { start, layer: fixed_layer, width_nm, .. } if *start == point && *fixed_layer == layer => Some(*width_nm),
            _ => None,
        })
        .unwrap_or(request.width_nm);
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
    let mut pickup = super::walkaround::Walker::new(query, &start, budget, progress.clone())?;
    let rejected = match pickup.displacement_anchor(point).await {
        Ok(true) => None,
        Ok(false) => Some((Outcome::Blocked { last_legal: request.head }, true)),
        Err(super::walkaround::Stop::Exhausted) => Some((Outcome::Incomplete, false)),
        Err(super::walkaround::Stop::Failed(error)) => return Err(error),
    };
    if let Some((outcome, start_blocked)) = rejected {
        return Ok(Plan {
            adjusted_target: None,
            start_blocked,
            replacements: Vec::new(),
            shove: Some(ShoveStats::default()),
            points: vec![request.head],
            outcome,
            blocker: None,
            probes: progress.load(Ordering::Relaxed),
        });
    }
    let chains = match chains::capture(query).await {
        Ok(chains) => chains,
        Err(CommandError::CapExceeded(_)) => {
            return Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                replacements: Vec::new(),
                shove: Some(ShoveStats::default()),
                points: vec![request.head],
                outcome: Outcome::Incomplete,
                blocker: None,
                probes: progress.load(Ordering::Relaxed),
            });
        }
        Err(error) => return Err(error),
    };
    let first = attempt(query, &chains, request, budget, progress.clone(), false).await?;
    if !matches!(first.outcome, Outcome::Blocked { .. })
        || first.start_blocked
        || !first.shove.as_ref().is_some_and(|stats| stats.queued_tracks > 0)
        || progress.load(Ordering::Relaxed) >= budget
    {
        return Ok(first);
    }
    let mut second = attempt(query, &chains, request, budget, progress, true).await?;
    if let (Some(stats), Some(previous)) = (&mut second.shove, &first.shove) {
        stats.include_previous(previous);
    }
    Ok(second)
}

/// Plan around fixed obstacles first, then propagate each obstructing unlocked track through
/// a finite queue ordered along the push direction. Processed replacements become fixed obstacles for later work,
/// preventing displacement cycles. A failed cascade discards its overlay and reports a stop;
/// it never changes to walking around the original movable tracks.
async fn attempt(
    query: &ClearanceQuery,
    chains: &Chains,
    request: &Request,
    budget: usize,
    progress: Arc<AtomicUsize>,
    reverse_displacements: bool,
) -> Result<Plan, CommandError> {
    let Pickup { source: origin, nets, islands } = pickup(query, request)?;
    let fixed: Vec<_> = request.fixed.iter().map(|p| p.geometry(query)).collect::<Result<_, _>>()?;
    let relevant_layers: BTreeSet<_> = std::iter::once(request.layer).chain(fixed.iter().flat_map(|(_, layers)| layers.iter().copied())).collect();
    let movable: BTreeSet<_> = chains
        .tracks
        .iter()
        .filter(|(i, t)| {
            !t.locked
                && relevant_layers.contains(&t.layer)
                && !islands.contains(&query.objects()[**i].island)
                && chains.units.contains_key(&chains.owner[**i])
        })
        .map(|(i, _)| *i)
        .collect();
    let units: BTreeSet<_> = movable.iter().map(|i| chains.owner[*i]).collect();
    let mut next = 1u128;
    let mut stats = ShoveStats { attempts: 1, ..ShoveStats::default() };
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
    let fixed_query = query.with_objects(items)?;
    let mut overlay = fixed_query.overlay();
    add_path(&mut overlay, &fixed, origin, &mut next, &mut stats)?;
    super::search::yield_unit().await;
    let initial = overlay.routing_query(origin)?;
    let mut plan = super::search::walk(&initial, request, budget, progress.clone()).await?;
    stats.used_walkaround =
        plan.outcome == Outcome::Reached && plan.points != super::posture::elbow(request.head, request.target, request.posture, request.flipped);
    let mut incomplete = plan.outcome == Outcome::Incomplete;
    if plan.outcome == Outcome::Reached {
        let pieces: Vec<_> =
            plan.points.windows(2).map(|p| (Contour::capsule(p[0], p[1], request.width_nm as f64 / 2.), vec![request.layer])).collect();
        add_path(&mut overlay, &pieces, origin, &mut next, &mut stats)?;
        let fixed_count = fixed.len();
        let mut route = fixed;
        route.extend(pieces);
        let mut queue = BTreeSet::new();
        let mut pushes = BTreeMap::new();
        let mut replaced = BTreeSet::new();
        let mut replacements = Vec::new();
        let mut legal = Propagation {
            chains,
            movable: &movable,
            replaced: &replaced,
            queue: &mut queue,
            pushes: &mut pushes,
            incoming: None,
            contact_exemptions: None,
            stats: &mut stats,
            progress: &progress,
            budget,
        }
        .enqueue(query, &route, &nets, &islands)
        .await?;
        while legal && !queue.is_empty() {
            super::search::yield_unit().await;
            if replaced.len() >= MAX_SHOVES {
                incomplete = true;
                legal = false;
                break;
            }
            let Some(source) = queue.iter().copied().min_by(|a, b| {
                pushes[a]
                    .piece
                    .cmp(&pushes[b].piece)
                    .then_with(|| {
                        push_order(&query.objects()[pushes[a].hit], pushes[a]).total_cmp(&push_order(&query.objects()[pushes[b].hit], pushes[b]))
                    })
                    .then(a.cmp(b))
            }) else {
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
            let obstacles = overlay.routing_query(origin)?;
            let mut retained = Vec::new();
            for (position, anchor) in chain.anchors.iter().enumerate() {
                if anchor.protected {
                    retained.push(position);
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
                    posture: request.posture,
                    flipped: request.flipped,
                };
                let mut walker = super::walkaround::Walker::new(&obstacles, &probe, budget, progress.clone())?;
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
                    posture: request.posture,
                    flipped: request.flipped,
                    previous: chain.anchors[interval[0]..=interval[1]].iter().map(|a| a.point).collect(),
                    mode: Mode::Walkaround,
                    merge_same_net: request.merge_same_net,
                    fixed: Vec::new(),
                };
                if reverse_displacements {
                    std::mem::swap(&mut displaced.head, &mut displaced.target);
                    displaced.flipped = !displaced.flipped;
                }
                let mut result = displace(&obstacles, &displaced, object, contacts.clone(), incoming, budget, progress.clone()).await?;
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
            let Some(parts) = split_chain(chains, source, &path) else {
                incomplete = true;
                legal = false;
                break;
            };
            if replacements.len() + parts.len() > MAX_SHOVES {
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
            add_path(&mut overlay, &pieces, Some(source), &mut next, &mut stats)?;
            legal = Propagation {
                chains,
                movable: &movable,
                replaced: &replaced,
                queue: &mut queue,
                pushes: &mut pushes,
                incoming: Some(incoming),
                contact_exemptions: contacts.as_ref(),
                stats: &mut stats,
                progress: &progress,
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
                    if !charge(&progress, budget).await {
                        incomplete = true;
                        legal = false;
                        break;
                    }
                    let result = query.probe_membership_excluding(contour, layers, &nets, |i| replaced.contains(&chains.owner[i]))?;
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
