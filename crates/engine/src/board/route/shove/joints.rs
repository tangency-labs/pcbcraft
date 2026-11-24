//! Through-via moves and connected drag inputs for the ordinary track cascade.
use super::*;
use crate::board::{
    clearance::ObjectKind,
    track::{Ends, replacements::retained_joint},
};

/// A sparse set of source contours translated before the track cascade.
/// Every follower remains a single segment with the original identity.
#[derive(Default)]
pub(super) struct Motion {
    contours: BTreeMap<usize, Contour>,
    tracks: Vec<Replacement>,
    vias: Vec<ViaMove>,
}

impl Motion {
    /// Observe the tentative move before guide or cascade validation on the same snapshot.
    #[cfg(test)]
    pub(super) fn preview_for_test(query: &ClearanceQuery, request: &Request) -> Vec<ViaMove> {
        use std::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };
        let mut job = pin!(async {
            let chains = chains::capture(query).await.unwrap();
            let guide = super::super::posture::elbow(request.head, request.target, request.posture, request.flipped);
            let route = gesture_geometry(query, request, &guide).unwrap();
            Self::vias(query, &route, &chains, &pickup(query, request).unwrap(), MAX_BUDGET, &AtomicUsize::new(0)).await.unwrap().vias
        });
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..MAX_BUDGET {
            if let Poll::Ready(vias) = job.as_mut().poll(&mut cx) {
                return vias;
            }
        }
        panic!("tentative via search exceeded the test work bound")
    }

    /// Capture each via hit by the fixed prefix or preferred incoming elbow in source order.
    /// Unsupported contacts pin the via. The ordinary walkaround then sees it as fixed.
    async fn vias(
        query: &ClearanceQuery,
        route: &[(Contour, Vec<Uuid>)],
        chains: &Chains,
        pickup: &Pickup,
        budget: usize,
        progress: &AtomicUsize,
    ) -> Result<Self, CommandError> {
        let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Missing board.".into()))?;
        let mut result = Self::default();
        if view.vias().is_empty() || !view.zones().is_empty() {
            return Ok(result);
        }

        for (shape, layers) in route {
            super::super::search::yield_unit().await;
            let candidates = query.candidates(shape, layers)?;
            let eligible: BTreeMap<_, _> = candidates
                .iter()
                .filter_map(|&i| {
                    let item = &query.objects()[i];
                    if item.kind != ObjectKind::Via
                        || pickup.islands.contains(&item.island)
                        || result.contours.contains_key(&i)
                        || (!pickup.nets.is_empty() && pickup.nets == item.intended_nets)
                    {
                        return None;
                    }
                    let id = Uuid::parse_str(&item.name).ok()?;
                    let via = view.vias().binary_search_by_key(&id, |v| v.id).ok()?;
                    (!view.vias()[via].locked).then_some((i, id))
                })
                .collect();
            if eligible.is_empty() {
                continue;
            }
            if !charge(progress, budget).await {
                break;
            }
            // Non-via contacts can affect the probe's net membership and pair clearances.
            for hit in query.probe_candidates(shape, &pickup.nets, candidates.into_iter())?.decisions {
                let i = hit.second;
                let Some(&id) = eligible.get(&i) else { continue };
                if !hit.touches && hit.gap_nm >= hit.clearance_nm as f64 {
                    continue;
                }
                let item = &query.objects()[i];
                let mut attached = Vec::new();
                let mut pinned = false;
                for j in query.candidates(&item.contour, &item.layers)? {
                    super::super::search::yield_unit().await;
                    if j == i || !item.contour.touches(&query.objects()[j].contour) {
                        continue;
                    }
                    if let Some(track) = chains.tracks.get(&j)
                        && !track.locked
                        && [track.start, track.end].contains(&item.at)
                        && !result.contours.contains_key(&j)
                    {
                        attached.push(j);
                    } else {
                        pinned = true;
                        break;
                    }
                }
                if pinned {
                    continue;
                }
                let Some(hull) = super::super::hull::Hull::grown(
                    &shape.core,
                    shape.radius + item.contour.radius + hit.clearance_nm as f64 + 2.,
                    Posture::Diagonal,
                ) else {
                    continue;
                };
                for at in hull.exits(item.at) {
                    if progress.load(Ordering::Relaxed) >= budget {
                        return Ok(result);
                    }
                    let mut moved = BTreeMap::from([(i, Contour::capsule(at, at, item.contour.radius))]);
                    let mut paths = Vec::new();
                    for j in &attached {
                        let t = &chains.tracks[j];
                        let ends = [t.start, t.end].map(|p| if p == item.at { at } else { p });
                        if ends[0] == ends[1] {
                            break;
                        }
                        moved.insert(*j, Contour::capsule(ends[0], ends[1], t.width_nm as f64 / 2.));
                        paths.push(Replacement { track: t.id, points: ends.to_vec() });
                    }
                    if paths.len() != attached.len() || !result.legal(query, &moved, route, &pickup.nets, budget, progress).await? {
                        continue;
                    }
                    result.contours.extend(moved);
                    result.vias.push(ViaMove { via: id, at });
                    result.tracks.extend(paths);
                    break;
                }
            }
        }
        Ok(result)
    }

    /// Check every translated body on all occupied layers, with its original class and contacts.
    /// Approved motions are tested at their new locations and incoming copper stays reserved.
    async fn legal(
        &self,
        query: &ClearanceQuery,
        moved: &BTreeMap<usize, Contour>,
        route: &[(Contour, Vec<Uuid>)],
        nets: &[usize],
        budget: usize,
        progress: &AtomicUsize,
    ) -> Result<bool, CommandError> {
        let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Missing board.".into()))?;
        let outline = view.outline_edges();
        let baseline = query.authority().rules().values;
        for (&i, shape) in moved {
            if !charge(progress, budget).await {
                return Ok(false);
            }
            let old = &query.objects()[i];
            let probe = query
                .probe_membership_excluding(shape, &old.layers, &old.intended_nets, |j| moved.contains_key(&j) || self.contours.contains_key(&j))?;
            let rules = probe.probe_rules.as_ref().map_or(baseline, |p| p.hard_values(baseline));
            if !Uuid::parse_str(&old.name).is_ok_and(|id| crate::board::track::replacements::joint_sizes(view, id, shape, rules))
                || !outline.allows_copper(shape, rules.edge_clearance_nm)
            {
                return Ok(false);
            }
            for j in query.candidates(&old.contour, &old.layers)? {
                super::super::search::yield_unit().await;
                let other = &query.objects()[j];
                if i != j
                    && old.contour.touches(&other.contour)
                    && !retained_joint(old, other, shape, moved.get(&j).or(self.contours.get(&j)).unwrap_or(&other.contour), 0.)
                {
                    return Ok(false);
                }
            }
            for hit in probe.decisions {
                let other = &query.objects()[hit.second];
                if hit.conflicting_nets
                    || (hit.gap_nm < hit.clearance_nm as f64 || hit.touches)
                        && (other.mechanical || !retained_joint(old, other, shape, &other.contour, hit.clearance_nm as f64))
                {
                    return Ok(false);
                }
            }
            for (&j, other_shape) in self.contours.iter().chain(moved) {
                super::super::search::yield_unit().await;
                let other = &query.objects()[j];
                let clearance = query.membership_clearance(&old.intended_nets, &other.intended_nets) as f64;
                if i != j
                    && old.layers.iter().any(|l| other.layers.contains(l))
                    && (shape.touches(other_shape) || shape.gap(other_shape) < clearance)
                    && !retained_joint(old, other, shape, other_shape, clearance)
                {
                    return Ok(false);
                }
            }
            for (other, layers) in route {
                super::super::search::yield_unit().await;
                if layers.iter().any(|l| old.layers.contains(l))
                    && (shape.touches(other) || shape.gap(other) < query.membership_clearance(&old.intended_nets, nets) as f64)
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Keep moved joints fixed throughout the subsequent track cascade.
    fn projection(&self, query: &ClearanceQuery, chains: &Chains) -> Result<Option<(ClearanceQuery, BTreeSet<usize>)>, CommandError> {
        if self.vias.is_empty() {
            return Ok(None);
        }
        let mut items = query.objects().to_vec();
        let mut frozen = BTreeSet::new();
        for (&i, shape) in &self.contours {
            items[i].at = shape.core[0];
            items[i].contour = shape.clone();
            frozen.insert(chains.owner[i]);
        }
        Ok(Some((query.with_objects(items)?, frozen)))
    }

    /// Try moving only vias obstructing the preferred path. If that exact guide fails,
    /// discard its moves before the ordinary solver considers a different incoming path.
    pub(super) async fn route(
        query: &ClearanceQuery,
        chains: &Chains,
        request: &Request,
        pickup: &Pickup,
        budget: usize,
        progress: Arc<AtomicUsize>,
    ) -> Result<Option<Plan>, CommandError> {
        let guide = super::super::posture::elbow(request.head, request.target, request.posture, request.flipped);
        let route = gesture_geometry(query, request, &guide)?;
        let motion = Self::vias(query, &route, chains, pickup, budget, &progress).await?;
        let Some((projected, frozen)) = motion.projection(query, chains)? else { return Ok(None) };
        let search = Search::prepare(&projected, chains, request, budget, progress, pickup.clone(), &frozen).await?;
        // At a fixed vertex the open guide is a single point. The ordinary walker
        // certifies that point before the cascade rechecks the whole fixed prefix.
        let approach = if request.head == request.target { Approach::Walk(request.flipped) } else { Approach::Continue(guide) };
        let mut plan = search.solve_guides(request, std::iter::once(approach)).await?;
        if plan.outcome != Outcome::Reached {
            return Ok(None);
        }
        motion.finish(&mut plan);
        Ok(Some(plan))
    }

    /// Include accepted joint changes in the same placement and diagnostics as track changes.
    fn finish(self, plan: &mut Plan) {
        plan.via_moves = self.vias;
        plan.replacements.extend(self.tracks);
        plan.replacements.sort_by_key(|r| r.track);
        if let Some(stats) = &mut plan.shove {
            stats.displaced_tracks = plan.replacements.len();
        }
    }
}

/// Reserve the exact fixed pieces and open segment geometry used for this candidate.
fn gesture_geometry(query: &ClearanceQuery, request: &Request, points: &[Point]) -> Result<Vec<(Contour, Vec<Uuid>)>, CommandError> {
    let mut route: Vec<_> = request.fixed.iter().map(|p| p.geometry(query)).collect::<Result<_, _>>()?;
    route.extend(points.windows(2).map(|p| (Contour::capsule(p[0], p[1], request.width_nm as f64 / 2.), vec![request.layer])));
    Ok(route)
}

/// Place the requested D-drag segments on the gesture overlay and feed their collisions to
/// the existing track cascade. The final reshape command checks the old contacts independently.
pub(in crate::board::route) async fn drag(
    query: &ClearanceQuery,
    ends: Vec<Ends>,
    mode: Mode,
    budget: usize,
    progress: Arc<AtomicUsize>,
) -> Result<Plan, CommandError> {
    let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Missing board.".into()))?;
    if !view.zones().is_empty() {
        return Err(CommandError::Unavailable("Moving routing joints on boards with zones is not supported.".into()));
    }
    // Bound the directly supplied geometry by the existing placement gesture budget.
    if ends.len() > crate::board::track::MAX_PATH_POINTS {
        return Err(CommandError::InvalidParams("A shove drag supports at most 256 directly moved tracks.".into()));
    }
    let first = ends.first().ok_or_else(|| CommandError::InvalidParams("Empty drag.".into()))?;
    let tracks = view.tracks();
    let source_track = |id| {
        tracks
            .binary_search_by_key(&id, |t| t.id)
            .ok()
            .map(|i| &tracks[i])
            .ok_or_else(|| CommandError::InvalidParams("Unknown dragged track.".into()))
    };
    let primary = source_track(first.track)?;
    let mut moved = BTreeMap::new();
    let mut fixed = Vec::new();
    for (k, row) in ends.iter().enumerate() {
        super::super::search::yield_unit().await;
        let track = source_track(row.track)?;
        if track.locked || track.layer != primary.layer || row.start == row.end || moved.insert(row.track, (row.start, row.end)).is_some() {
            return Err(CommandError::InvalidParams("Drag distinct unlocked segments on one layer without collapsing them.".into()));
        }
        if k > 0 {
            fixed.push(FixedCopper::Segment { start: row.start, end: row.end, layer: track.layer, width_nm: track.width_nm });
        }
    }
    let mut items = Vec::new();
    let mut membership = None;
    for object in query.objects() {
        super::super::search::yield_unit().await;
        if Uuid::parse_str(&object.name).is_ok_and(|id| moved.contains_key(&id)) {
            let intent = (object.island, object.intended_nets.clone());
            if membership.replace(intent.clone()).is_some_and(|old| old != intent) {
                return Err(CommandError::InvalidParams("Shove-drag tracks must belong to one connected island.".into()));
            }
        } else {
            items.push(object.clone());
        }
    }
    let (island, nets) = membership.ok_or_else(|| CommandError::InvalidParams("Missing drag source.".into()))?;
    let origin = items.iter().position(|o| !o.mechanical && o.island == island);
    let projected = query.with_objects(items)?;
    let chains = chains::capture(&projected).await?;
    let request = Request {
        head: first.start,
        target: first.end,
        width_nm: primary.width_nm,
        layer: primary.layer,
        origin: None,
        fixed,
        merge_same_net: false,
        mode: Mode::Shove,
        posture: Posture::Diagonal,
        flipped: false,
        previous: Vec::new(),
    };
    let pickup = Pickup { source: origin, nets, islands: BTreeSet::from([island]) };
    let motion = if mode == Mode::ShoveVias {
        let route = gesture_geometry(&projected, &request, &[first.start, first.end])?;
        Motion::vias(&projected, &route, &chains, &pickup, budget, &progress).await?
    } else {
        Motion::default()
    };
    let projection = motion.projection(&projected, &chains)?;
    let fixed = BTreeSet::new();
    let (geometry, frozen) = projection.as_ref().map_or((&projected, &fixed), |(q, f)| (q, f));
    let mut search = Search::prepare(geometry, &chains, &request, budget, progress, pickup, frozen).await?;
    // The collision direction follows the gesture, including a drag perpendicular to a parallel trace.
    search.drag_direction = Some(if primary.start != first.start { (primary.start, first.start) } else { (primary.end, first.end) });
    let mut plan = search.solve_guides(&request, std::iter::once(Approach::Drag)).await?;
    if plan.outcome == Outcome::Reached {
        motion.finish(&mut plan);
    }
    Ok(plan)
}
