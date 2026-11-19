//! Deterministic track displacement over disposable copper. Pads, vias and locks stay fixed;
//! every returned replacement is still checked by the shared compound transaction.
use super::*;
use crate::board::{
    clearance::{Object, Overlay},
    geometry::Contour,
    track::replacements::Replacement,
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
const MAX_ANCHORS: usize = 128;

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

/// A connected source graph is represented by stable contact anchors on each displaced segment.
/// Branch junctions and pad/via contacts remain fixed; legal intervals between them can bend.
fn anchors(query: &ClearanceQuery, source: usize, stats: &mut ShoveStats) -> Result<Option<Vec<Point>>, CommandError> {
    let object = &query.objects()[source];
    let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let Some(track) = view.tracks().iter().find(|t| t.id.to_string() == object.name) else {
        return Ok(None);
    };
    let (a, b) = (track.start, track.end);
    let (dx, dy) = ((b.x_nm - a.x_nm) as f64, (b.y_nm - a.y_nm) as f64);
    let length = dx * dx + dy * dy;
    let project = |p: Point| {
        let t = (((p.x_nm - a.x_nm) as f64 * dx + (p.y_nm - a.y_nm) as f64 * dy) / length).clamp(0., 1.);
        (t, Point { x_nm: (a.x_nm as f64 + t * dx).round() as i64, y_nm: (a.y_nm as f64 + t * dy).round() as i64 })
    };
    let mut points = vec![(0., a), (1., b)];
    let candidates = query.candidates(&object.contour, &object.layers)?;
    stats.candidate_hits += candidates.len();
    for index in candidates {
        if index == source {
            continue;
        }
        let other = &query.objects()[index];
        if !object.contour.touches(&other.contour) {
            continue;
        }
        let best = other
            .contour
            .core
            .iter()
            .copied()
            .chain([other.at])
            .map(project)
            .filter(|(_, p)| other.contour.touches(&Contour::capsule(*p, *p, object.contour.radius)))
            .min_by(|(ta, _), (tb, _)| ta.total_cmp(tb));
        let Some(point) = best else {
            return Ok(None);
        };
        points.push(point);
        if points.len() > MAX_ANCHORS {
            return Ok(None);
        }
    }
    points.sort_by(|(a, _), (b, _)| a.total_cmp(b));
    let mut points: Vec<_> = points.into_iter().map(|(_, p)| p).collect();
    points.dedup();
    Ok(Some(points))
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
}

/// Source queue with the originating route-piece direction retained through a cascade.
struct Propagation<'a> {
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
            let result = query.probe_membership(contour, layers, nets, None)?;
            self.stats.candidate_hits += result.decisions.len();
            for hit in result.decisions {
                if self.replaced.contains(&hit.second) {
                    continue;
                }
                let object = &query.objects()[hit.second];
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
                if self.queue.insert(hit.second) {
                    self.pushes
                        .entry(hit.second)
                        .or_insert_with(|| self.incoming.unwrap_or(Push { from: contour.core[0], toward: contour.core[1], piece }));
                    self.stats.queued_tracks += 1;
                }
            }
            self.stats.maximum_queue = self.stats.maximum_queue.max(self.queue.len());
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
            return walker.tidy_displacement(plan, request.flipped).await;
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

/// Try one alternate return direction after a blocked cascade. Both attempts share the same
/// captured source and probe budget; cancellation drops the currently owned overlay. The
/// initial attempt must finish normally, so explicit search exhaustion stays incomplete.
pub(super) async fn run(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Plan, CommandError> {
    let first = attempt(query, request, budget, progress.clone(), false).await?;
    if !matches!(first.outcome, Outcome::Blocked { .. })
        || first.start_blocked
        || !first.shove.as_ref().is_some_and(|stats| stats.queued_tracks > 0)
        || progress.load(Ordering::Relaxed) >= budget
    {
        return Ok(first);
    }
    let mut second = attempt(query, request, budget, progress, true).await?;
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
    request: &Request,
    budget: usize,
    progress: Arc<AtomicUsize>,
    reverse_displacements: bool,
) -> Result<Plan, CommandError> {
    let Pickup { source: origin, nets, islands } = pickup(query, request)?;
    let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let tracks: BTreeMap<_, _> = view.tracks().iter().map(|t| (t.id.to_string(), t)).collect();
    let movable: BTreeSet<_> = query
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(i, o)| tracks.get(&o.name).filter(|t| !t.locked && !islands.contains(&o.island)).map(|_| i))
        .collect();
    let mut next = 1u128;
    let mut stats = ShoveStats { attempts: 1, ..ShoveStats::default() };
    let fixed: Vec<_> = request.fixed.iter().map(|p| p.geometry(query)).collect::<Result<_, _>>()?;
    // Endpoints are fixed graph anchors. Keeping their caps visible prevents an earlier
    // displacement from stranding a later neighbor's start inside displaced copper.
    let mut items = query.objects().to_vec();
    for source in &movable {
        super::search::yield_unit().await;
        let object = &query.objects()[*source];
        let Some(track) = tracks.get(&object.name) else {
            continue;
        };
        let Some(points) = anchors(query, *source, &mut stats)? else {
            return Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                replacements: Vec::new(),
                shove: Some(stats),
                points: vec![request.head],
                outcome: Outcome::Incomplete,
                blocker: None,
                probes: progress.load(Ordering::Relaxed),
            });
        };
        items[*source].contour = Contour::capsule(track.start, track.start, object.contour.radius);
        for point in points.iter().skip(1) {
            let mut anchor = object.clone();
            anchor.contour = Contour::capsule(*point, *point, object.contour.radius);
            items.push(anchor);
        }
        stats.fixed_anchor_objects += points.len();
        stats.overlay_geometry_bytes += points.len() * (std::mem::size_of::<Contour>() + std::mem::size_of::<Uuid>());
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
        let mut route = fixed;
        route.extend(pieces);
        let mut queue = BTreeSet::new();
        let mut pushes = BTreeMap::new();
        let mut replaced = BTreeSet::new();
        let mut replacements = Vec::new();
        let mut legal = Propagation {
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
                    .then_with(|| push_order(&query.objects()[*a], pushes[a]).total_cmp(&push_order(&query.objects()[*b], pushes[b])))
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
            let Some(anchors) = anchors(query, source, &mut stats)? else {
                incomplete = true;
                legal = false;
                break;
            };
            let Some(track) = tracks.get(&object.name) else {
                incomplete = true;
                legal = false;
                break;
            };
            let contacts = if request.merge_same_net {
                None
            } else {
                Some(
                    query
                        .candidates(&object.contour, &object.layers)?
                        .into_iter()
                        .filter(|i| query.objects()[*i].contour.touches(&object.contour))
                        .map(|i| query.objects()[i].name.clone())
                        .collect(),
                )
            };
            let mut path = vec![track.start];
            let obstacles = overlay.routing_query(origin)?;
            for anchor in anchors.windows(2) {
                let mut displaced = Request {
                    head: anchor[0],
                    target: anchor[1],
                    layer: track.layer,
                    origin: None,
                    width_nm: track.width_nm,
                    posture: request.posture,
                    flipped: request.flipped,
                    previous: Vec::new(),
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
            let pieces: Vec<_> = path.windows(2).map(|p| (Contour::capsule(p[0], p[1], track.width_nm as f64 / 2.), vec![track.layer])).collect();
            add_path(&mut overlay, &pieces, Some(source), &mut next, &mut stats)?;
            legal = Propagation {
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
            replacements.push(Replacement { track: track.id, points: path });
            stats.displaced_tracks = replacements.len();
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
