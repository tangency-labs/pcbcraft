//! Shared displacement lanes for a cascade that cannot be solved one trace at a time.
use super::*;
use crate::board::track::chains::Chain;

/// One along/cross coordinate system, oriented from the last gesture leg toward the cursor.
#[derive(Clone, Copy)]
struct Axes {
    horizontal: bool,
    forward: i64,
    across: i64,
}
impl Axes {
    /// Express a board point in the forward and transverse directions of this retry.
    fn project(self, point: Point) -> (i64, i64) {
        if self.horizontal { (point.x_nm * self.forward, point.y_nm * self.across) } else { (point.y_nm * self.forward, point.x_nm * self.across) }
    }

    /// Map a lane or return point back to the board lattice.
    fn point(self, along: i64, across: i64) -> Point {
        if self.horizontal {
            Point { x_nm: along * self.forward, y_nm: across * self.across }
        } else {
            Point { x_nm: across * self.across, y_nm: along * self.forward }
        }
    }
}

/// Reserve an ordered passage for every run the incoming route already hits.
/// The later runs return farther around the bend, leaving room for their neighbors.
pub(super) struct Corridor {
    axes: Axes,
    along: i64,
    before: i64,
    after: i64,
    spacing: i64,
    pitch: i64,
    ranks: BTreeMap<usize, i64>,
}
impl Corridor {
    /// Use actual route extent, projected widths and captured clearance to lay out
    /// the whole group. This supplies candidate geometry, never clearance authority.
    pub(super) fn new(
        search: &Search<'_>,
        request: &Request,
        plan: &Plan,
        route: &[(Contour, Vec<Uuid>)],
        queue: &BTreeSet<usize>,
        pushes: &BTreeMap<usize, Push>,
        strategy: Cascade,
    ) -> Option<Self> {
        let Cascade::Shared { positive, advance } = strategy else { return None };
        let [a, b] = plan.points.last_chunk::<2>()?;
        let (dx, dy) = (b.x_nm - a.x_nm, b.y_nm - a.y_nm);
        let mut sources: Vec<_> = queue.iter().copied().filter(|i| search.chains.tracks[i].layer == request.layer).collect();
        if sources.len() < 2 {
            return None;
        }
        sources.sort_by(|a, b| compare_pushes(search.query, pushes, a, b));
        let start = pushes[&sources[0]].piece;
        // A diagonal tail can still share an axis-aligned passage. Use the
        // dominant axis from the first collision when the tail has no single axis.
        let from = pushes[&sources[0]].from;
        let horizontal = match (dx, dy) {
            (_, 0) => true,
            (0, _) => false,
            _ => (b.x_nm - from.x_nm).abs() >= (b.y_nm - from.y_nm).abs(),
        };
        let axes = Axes { horizontal, forward: if horizontal { dx.signum() } else { dy.signum() }, across: if positive { 1 } else { -1 } };
        let mut copper = route[start..].iter().filter(|(_, layers)| layers.contains(&request.layer)).flat_map(|(c, _)| c.core);
        let (_, v) = axes.project(copper.next()?);
        let (before, after) = copper.map(|p| axes.project(p).1).fold((v, v), |(min, max), v| (min.min(v), max.max(v)));
        let width = search
            .movable
            .iter()
            .map(|i| &search.chains.tracks[i])
            .filter(|track| track.layer == request.layer)
            .map(|track| track.width_nm)
            .fold(request.width_nm, i64::max);
        let spacing = width + search.query.authority().clearance_nm() + 2;
        let pitch = (spacing as f64 * std::f64::consts::SQRT_2).ceil() as i64;
        let ranks = sources.into_iter().enumerate().map(|(rank, index)| (index, rank as i64 + 1)).collect();
        Some(Self { axes, along: axes.project(*b).0 + advance, before, after, spacing, pitch, ranks })
    }

    /// Try the smallest local window first and widen only across ordinary bends.
    /// Exact checks retain the copper outside the window and all protected stations.
    pub(super) async fn path(
        &mut self,
        search: &Search<'_>,
        obstacles: RouteObstacles<'_>,
        request: &Request,
        source: usize,
        push: Push,
    ) -> Result<Option<Vec<Point>>, super::super::walkaround::Stop> {
        use super::super::walkaround::{Stop, Walker, simplify};

        let chain = &search.chains.units[&source];
        let track = &search.chains.tracks[&source];
        let object = &search.query.objects()[source];
        // A run encountered by a reserved lane joins the same group ahead of
        // its existing slots. Its inherited push still identifies the approach.
        let next = self.ranks.len() as i64 + 1;
        let rank = *self.ranks.entry(source).or_insert(next);
        let slot = self.along + rank * self.pitch;
        let exit = self.after + self.spacing + rank * self.pitch;
        let Some((hit, &(segment, reverse))) = chain.members.iter().enumerate().find(|(_, member)| member.0 == push.hit) else { return Ok(None) };
        let hit = hit as f64 + crossing_station(&search.chains.tracks[&segment], reverse, push);
        let (left, right) = windows(chain, hit);
        if left.is_empty() || right.is_empty() {
            return Ok(None);
        }
        let local = Request {
            head: chain.anchors[0].point,
            target: chain.anchors[chain.anchors.len() - 1].point,
            layer: track.layer,
            width_nm: track.width_nm,
            origin: None,
            previous: Vec::new(),
            fixed: Vec::new(),
            ..request.clone()
        };
        let nets = &object.intended_nets;
        let self_spacing = if request.merge_same_net { 0. } else { track.width_nm as f64 + search.query.membership_clearance(nets, nets) as f64 };

        for extension in 0..left.len() + right.len() - 1 {
            let earliest = extension.saturating_sub(right.len() - 1);
            for (i, &start) in left.iter().enumerate().take(extension + 1).skip(earliest) {
                let end = right[extension - i];
                if !charge(&search.progress, search.budget).await {
                    return Err(Stop::Exhausted);
                }
                let a = chain.anchors[start].point;
                let b = chain.anchors[end].point;
                let (_, av) = self.axes.project(a);
                let (_, bv) = self.axes.project(b);
                let Some(bridge) = self.bridge(a, b, slot, exit, request.posture) else { continue };
                let mut path: Vec<_> = chain.anchors[..start].iter().map(|anchor| anchor.point).collect();
                if av < bv {
                    path.extend(bridge);
                } else {
                    path.extend(bridge.into_iter().rev());
                }
                path.extend(chain.anchors[end + 1..].iter().map(|anchor| anchor.point));
                let path = simplify(&path);
                if path.len() <= super::super::super::track::MAX_PATH_POINTS
                    && path.iter().all(crate::wire::valid_point)
                    && chains::legal_stroke(&path, self_spacing).await
                {
                    if search.progress.load(Ordering::Relaxed) >= search.budget {
                        return Err(Stop::Exhausted);
                    }
                    let mut walker = Walker::new(obstacles, &local, search.budget, search.progress.clone()).map_err(Stop::Failed)?;
                    walker.inherit(object, (!request.merge_same_net).then(|| chain.contacts.clone()));
                    if let Some(plan) = walker.certify_candidate(&path).await? {
                        return Ok(Some(plan.points));
                    }
                }
            }
        }
        Ok(None)
    }

    /// A straight lane and staggered return, with chamfers contained in its legs.
    fn bridge(&self, a: Point, b: Point, slot: i64, exit: i64, posture: Posture) -> Option<Vec<Point>> {
        let ((u, v), (other_u, other_v)) = (self.axes.project(a), self.axes.project(b));
        let ((start_u, start_v), (end_u, end_v)) = if v < other_v { ((u, v), (other_u, other_v)) } else { ((other_u, other_v), (u, v)) };
        if start_v > self.before - self.spacing || end_v < exit {
            return None;
        }
        let corners = [(start_u, start_v), (slot, start_v), (slot, exit), (end_u, exit), (end_u, end_v)];
        if posture == Posture::Orthogonal {
            return Some(corners.into_iter().map(|(u, v)| self.axes.point(u, v)).collect());
        }
        let mut points = vec![self.axes.point(start_u, start_v)];
        for corner in corners.windows(3) {
            let [from, joint, to] = [corner[0], corner[1], corner[2]];
            let unit = |p: (i64, i64)| ((p.0 - joint.0).signum(), (p.1 - joint.1).signum());
            let distance = |p: (i64, i64)| (p.0 - joint.0).abs() + (p.1 - joint.1).abs();
            let cut = self.spacing.min(distance(from) / 2).min(distance(to) / 2);
            for p in [from, to] {
                let (du, dv) = unit(p);
                points.push(self.axes.point(joint.0 + du * cut, joint.1 + dv * cut));
            }
        }
        points.push(self.axes.point(end_u, end_v));
        Some(points)
    }
}

/// Collect each side in order of proximity to the push. A protected station
/// can be the edge of a window, but cannot disappear inside it.
fn windows(chain: &Chain, hit: f64) -> (Vec<usize>, Vec<usize>) {
    let middle = chain.anchors.partition_point(|anchor| anchor.order <= hit);
    let mut before = Vec::new();
    let mut after = Vec::new();
    for i in (0..middle).rev() {
        before.push(i);
        if chain.anchors[i].protected {
            break;
        }
    }
    for (i, anchor) in chain.anchors.iter().enumerate().skip(middle) {
        after.push(i);
        if anchor.protected {
            break;
        }
    }
    (before, after)
}

/// Locate the portion of the hit track affected by this leg, even when the
/// collision is clearance to an end instead of a centreline intersection.
fn crossing_station(track: &crate::board::track::Track, reverse: bool, push: Push) -> f64 {
    let leg = if reverse { [track.end, track.start] } else { [track.start, track.end] };
    crate::board::geometry::closest_station(leg, push.collision)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clearance-only endpoint hit belongs below the protected station,
    /// even though extending the incoming line would cross above it.
    #[test]
    fn finite_hit_chooses_the_window_on_the_approach_side() {
        let at = |x_nm, y_nm| Point { x_nm, y_nm };
        let track = crate::board::track::Track {
            id: Uuid::from_u128(1),
            layer: Uuid::from_u128(2),
            width_nm: 250_000,
            start: at(0, 0),
            end: at(0, 1_000_000),
            locked: false,
        };
        let from = at(-400_000, 100_000);
        let toward = at(-300_000, 300_000);
        let push = Push { from, toward, collision: [from, toward], piece: 0, hit: 0 };
        assert_eq!(crossing_station(&track, false, push), 0.3);
        assert_eq!(crossing_station(&track, true, push), 0.7);
    }
}
