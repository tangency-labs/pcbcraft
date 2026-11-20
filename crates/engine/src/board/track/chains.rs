//! Snapshot trace chains shared by shove and its independent compound validator.
use super::{Track, replacements::Replacement};
use crate::{
    CommandError,
    board::{
        clearance::ClearanceQuery,
        geometry::{Contour, segment_distance},
        work::yield_unit,
    },
    schematic::Point,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

/// Longer or closed components keep their original per-segment anchors in this slice.
const MAX_MEMBERS: usize = MAX_POINTS - 1;
/// Same bound on fixed contact scans as the per-segment shove implementation.
const MAX_CONTACTS: usize = 128;
/// A chain candidate must fit the ordinary bounded route-path representation.
pub(crate) const MAX_POINTS: usize = 256;

/// An original point along a trace; only actual graph contacts are mandatory.
#[derive(Clone, Copy)]
pub(crate) struct Anchor {
    pub order: f64,
    pub point: Point,
    pub protected: bool,
}

/// Uniform open trace, oriented from one stable end to the other.
pub(crate) struct Chain {
    pub members: Vec<(usize, bool)>,
    pub anchors: Vec<Anchor>,
    pub contacts: BTreeSet<String>,
}

/// Query indices remain snapshot-local; saved track UUIDs are retained independently.
pub(crate) struct Chains {
    pub owner: Vec<usize>,
    pub units: BTreeMap<usize, Chain>,
    pub tracks: BTreeMap<usize, Track>,
    touching: BTreeSet<(usize, usize)>,
}

/// Prepared topology belongs to one captured query, including any immutable failure.
type PreparedChains = Result<Arc<Chains>, CommandError>;
type ChainFuture = Pin<Box<dyn Future<Output = PreparedChains> + Send>>;

/// Small constant-work batches avoid making one UI frame necessary for every
/// metadata item or internal segment pair. Dense scans still suspend predictably.
#[derive(Default)]
struct Work(usize);
impl Work {
    /// Yield after sixteen graph/contact/pair operations; no unbounded loop drains here.
    async fn tick(&mut self) {
        self.0 += 1;
        if self.0.is_multiple_of(16) {
            yield_unit().await;
        }
    }
}

/// Resumable topology shared by all requests against one immutable query. Incomplete
/// pointer jobs leave their scan here, so coalescing cannot restart preparation forever.
#[derive(Default)]
pub(crate) struct Cache {
    future: Option<ChainFuture>,
    result: Option<PreparedChains>,
}
impl Cache {
    /// Advance a bounded number of graph/contact units while holding only this private
    /// cache. The owned source copy has an empty cache, preventing a reference cycle.
    fn step(&mut self, query: &ClearanceQuery, budget: usize) -> Option<PreparedChains> {
        if let Some(result) = &self.result {
            return Some(result.clone());
        }
        if self.future.is_none() {
            let source = query.chain_source();
            self.future = Some(Box::pin(async move { build(source).await.map(Arc::new) }));
        }
        let mut cx = Context::from_waker(Waker::noop());
        if let Some(future) = &mut self.future {
            for _ in 0..budget {
                if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
                    self.result = Some(result.clone());
                    self.future = None;
                    return Some(result);
                }
            }
        }
        None
    }
}

/// Nonblocking shared preparation. Cancellation drops only this waiter's continuation;
/// cached progress/results belong to the captured query and expire with it.
pub(crate) async fn capture(query: &ClearanceQuery) -> Result<Arc<Chains>, CommandError> {
    loop {
        yield_unit().await;
        let result = match query.trace_chains.try_lock() {
            Ok(mut cache) => cache.step(query, 8),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(CommandError::Unavailable("Trace topology preparation failed.".into())),
        };
        if let Some(result) = result {
            return result;
        }
    }
}

/// Resolve only unambiguous endpoint joins. Pad/via/lock contacts, width/layer/net
/// transitions and branch junctions cannot become flexible degree-two bends.
async fn build(query: ClearanceQuery) -> Result<Chains, CommandError> {
    let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let mut work = Work::default();
    let mut names = BTreeMap::new();
    for (i, o) in query.objects().iter().enumerate() {
        work.tick().await;
        names.insert(o.name.as_str(), i);
    }
    let mut tracks = BTreeMap::new();
    let mut ends = BTreeMap::<(Uuid, i64, i64), Vec<(usize, bool)>>::new();
    for track in view.tracks() {
        work.tick().await;
        let Some(&index) = names.get(track.id.to_string().as_str()) else { continue };
        tracks.insert(index, track.clone());
        if !track.locked {
            for (end, p) in [(false, track.start), (true, track.end)] {
                ends.entry((track.layer, p.x_nm, p.y_nm)).or_default().push((index, end));
            }
        }
    }
    let mut joins = BTreeMap::new();
    for (key, pair) in ends {
        work.tick().await;
        let [(a, ae), (b, be)] = pair.as_slice() else { continue };
        let (ta, tb) = (&tracks[a], &tracks[b]);
        if ta.width_nm != tb.width_nm || query.objects()[*a].intended_nets != query.objects()[*b].intended_nets {
            continue;
        }
        let p = Point { x_nm: key.1, y_nm: key.2 };
        let cap = Contour::capsule(p, p, ta.width_nm as f64 / 2.);
        let mut clear = true;
        for i in query.candidates(&cap, &[ta.layer])? {
            work.tick().await;
            if i != *a && i != *b && cap.touches(&query.objects()[i].contour) {
                clear = false;
                break;
            }
        }
        if clear {
            joins.insert((*a, *ae), (*b, *be));
            joins.insert((*b, *be), (*a, *ae));
        }
    }
    let mut owner: Vec<_> = (0..query.objects().len()).collect();
    let mut units = BTreeMap::new();
    let mut seen = BTreeSet::<usize>::new();
    for &source in tracks.keys() {
        if seen.contains(&source) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut todo = vec![source];
        while let Some(index) = todo.pop() {
            work.tick().await;
            if !component.insert(index) {
                continue;
            }
            for end in [false, true] {
                if let Some((next, _)) = joins.get(&(index, end)) {
                    todo.push(*next);
                }
            }
        }
        seen.extend(component.iter().copied());
        let start = component.iter().flat_map(|i| [(*i, false), (*i, true)]).filter(|key| !joins.contains_key(key)).min();
        let groups = if component.len() > MAX_MEMBERS || start.is_none() {
            component.into_iter().map(|i| vec![(i, false)]).collect::<Vec<_>>()
        } else {
            let mut members = Vec::new();
            let mut next = start;
            while let Some((index, entry)) = next {
                work.tick().await;
                members.push((index, entry));
                next = joins.get(&(index, !entry)).copied();
            }
            vec![members]
        };
        for members in groups {
            let Some(root) = members.iter().map(|p| p.0).min() else { continue };
            for (i, _) in &members {
                owner[*i] = root;
            }
            if let Some(chain) = contacts(&query, &tracks, members).await? {
                units.insert(root, chain);
            }
        }
    }
    let mut touching = BTreeSet::new();
    for (&root, chain) in &units {
        for name in &chain.contacts {
            work.tick().await;
            if let Some(&index) = names.get(name.as_str()) {
                let other = owner[index];
                touching.insert((root.min(other), root.max(other)));
            }
        }
    }
    let mut neighbors = BTreeMap::<usize, BTreeSet<usize>>::new();
    for &(a, b) in &touching {
        neighbors.entry(a).or_default().insert(b);
        neighbors.entry(b).or_default().insert(a);
    }
    let mut member_names = BTreeMap::<usize, Vec<String>>::new();
    for (&i, track) in &tracks {
        member_names.entry(owner[i]).or_default().push(track.id.to_string());
    }
    for (&root, chain) in &mut units {
        if let Some(neighbors) = neighbors.get(&root) {
            for other in neighbors {
                work.tick().await;
                if let Some(names) = member_names.get(other) {
                    chain.contacts.extend(names.iter().cloned());
                }
            }
        }
    }
    Ok(Chains { owner, units, tracks, touching })
}

/// Keep contact stations fixed and retain ordinary bends as soft hints. Unrepresentable
/// chains stay as fixed copper in the planner and cannot be replaced by a caller.
async fn contacts(query: &ClearanceQuery, tracks: &BTreeMap<usize, Track>, members: Vec<(usize, bool)>) -> Result<Option<Chain>, CommandError> {
    let mut work = Work::default();
    let own: BTreeSet<_> = members.iter().map(|p| p.0).collect();
    let mut contacts = own.iter().map(|i| query.objects()[*i].name.clone()).collect::<BTreeSet<_>>();
    let mut anchors = Vec::new();
    let mut protected = 2;
    for (position, &(i, reverse)) in members.iter().enumerate() {
        work.tick().await;
        let track = &tracks[&i];
        let object = &query.objects()[i];
        let (a, b) = if reverse { (track.end, track.start) } else { (track.start, track.end) };
        anchors.push(Anchor { order: position as f64, point: a, protected: position == 0 });
        anchors.push(Anchor { order: (position + 1) as f64, point: b, protected: position + 1 == members.len() });
        let (dx, dy) = ((b.x_nm - a.x_nm) as f64, (b.y_nm - a.y_nm) as f64);
        let span = dx * dx + dy * dy;
        for other in query.candidates(&object.contour, &object.layers)? {
            work.tick().await;
            if own.contains(&other) {
                continue;
            }
            let other = &query.objects()[other];
            if !object.contour.touches(&other.contour) {
                continue;
            }
            protected += 1;
            if protected > MAX_CONTACTS {
                return Ok(None);
            }
            contacts.insert(other.name.clone());
            let on_source = |t: f64| (t, Point { x_nm: (a.x_nm as f64 + t * dx).round() as i64, y_nm: (a.y_nm as f64 + t * dy).round() as i64 });
            let touches = |(_, p): &(f64, Point)| other.contour.touches(&Contour::capsule(*p, *p, object.contour.radius));
            let core = other.contour.core.as_slice();
            let crossing = match core {
                [c, d, end, start] if c == start && d == end => interior_crossing(a, b, *c, *d).map(on_source).filter(touches),
                _ => None,
            };
            let best = crossing.or_else(|| {
                core.iter()
                    .copied()
                    .chain([other.at])
                    .map(|p| on_source((((p.x_nm - a.x_nm) as f64 * dx + (p.y_nm - a.y_nm) as f64 * dy) / span).clamp(0., 1.)))
                    .filter(touches)
                    .min_by(|(a, _), (b, _)| a.total_cmp(b))
            });
            let best = best.or_else(|| other.contour.edges().filter_map(|(c, d)| interior_crossing(a, b, c, d)).map(on_source).find(touches));
            let Some((t, point)) = best else {
                return Ok(None);
            };
            anchors.push(Anchor { order: position as f64 + t, point, protected: true });
        }
    }
    anchors.sort_by(|a, b| a.order.total_cmp(&b.order).then(b.protected.cmp(&a.protected)));
    anchors.dedup_by(|a, b| a.point == b.point);
    Ok(Some(Chain { members, anchors, contacts }))
}

/// Find a proper centerline crossing using integer predicates even for off-angle segments.
/// Endpoint and collinear contacts continue through the ordinary station projection.
fn interior_crossing(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let delta = |a: Point, b: Point| (i128::from(b.x_nm) - i128::from(a.x_nm), i128::from(b.y_nm) - i128::from(a.y_nm));
    let cross = |(ax, ay), (bx, by)| ax * by - ay * bx;
    let (u, v, w) = (delta(a, b), delta(c, d), delta(a, c));
    let denominator = cross(u, v);
    let sign = denominator.signum();
    let (t, s, total) = (cross(w, v) * sign, cross(w, u) * sign, denominator.abs());
    (0 < t && t < total && 0 < s && s < total).then(|| t as f64 / total as f64)
}

impl Chains {
    /// Same trace or an existing external trace contact. Internal candidate geometry
    /// must pass `validate_paths` before this permission can authorize a transaction.
    pub fn contact(&self, a: usize, b: usize) -> bool {
        let (a, b) = (self.owner[a], self.owner[b]);
        a == b || self.touching.contains(&(a.min(b), a.max(b)))
    }

    /// Independently reconstruct submitted chains in source order, preserving all
    /// protected stations and refusing crossings or folded runs that lose spacing.
    async fn validate_paths(&self, query: &ClearanceQuery, replacements: &[Replacement], merge: bool) -> Result<(), CommandError> {
        let paths: BTreeMap<_, _> = replacements.iter().map(|r| (r.track, &r.points)).collect();
        let mut work = Work::default();
        for (index, track) in &self.tracks {
            work.tick().await;
            if paths.contains_key(&track.id) && !self.units.contains_key(&self.owner[*index]) {
                return Err(invalid("A replacement trace has contacts that cannot be preserved by bounded displacement."));
            }
        }
        for chain in self.units.values() {
            yield_unit().await;
            if !chain.members.iter().any(|(i, _)| paths.contains_key(&self.tracks[i].id)) {
                continue;
            }
            if !self.legal_replacements(query, chain, replacements, merge).await? {
                return Err(invalid("A replacement trace moves a protected contact, crosses itself or collapses a returning run."));
            }
        }
        Ok(())
    }
    /// Rebuild each candidate chain in source order without trusting a solver's
    /// partition. Missing records retain their original geometry and must still join.
    pub(crate) async fn legal_replacements(
        &self,
        query: &ClearanceQuery,
        chain: &Chain,
        replacements: &[Replacement],
        merge: bool,
    ) -> Result<bool, CommandError> {
        let paths: BTreeMap<_, _> = replacements.iter().map(|r| (r.track, &r.points)).collect();
        let mut points = Vec::new();
        for (i, reverse) in &chain.members {
            yield_unit().await;
            let track = &self.tracks[i];
            let original = [track.start, track.end];
            let path = paths.get(&track.id).map_or(original.as_slice(), |p| p.as_slice());
            let path: Vec<_> = if *reverse { path.iter().rev().copied().collect() } else { path.to_vec() };
            if !points.is_empty() && points.last() != path.first() {
                return Ok(false);
            }
            points.extend(path.into_iter().skip(usize::from(!points.is_empty())));
            if points.len() > MAX_POINTS {
                return Ok(false);
            }
        }
        Ok(self.legal_path(query, chain, &points, merge).await)
    }

    /// Shared exact topology/self-spacing oracle for a proposed whole-chain path.
    /// Ordinary non-returning corners form one stroke; separated return runs keep spacing.
    async fn legal_path(&self, query: &ClearanceQuery, chain: &Chain, points: &[Point], merge: bool) -> bool {
        let mut work = Work::default();
        let (Some(first), Some(last)) = (chain.anchors.first(), chain.anchors.last()) else {
            return false;
        };
        if points.first() != Some(&first.point) || points.last() != Some(&last.point) || points.len() > MAX_POINTS {
            return false;
        }
        let mut previous = 0.;
        for anchor in chain.anchors.iter().filter(|a| a.protected) {
            work.tick().await;
            let station = points
                .windows(2)
                .enumerate()
                .filter(|(_, p)| on_leg(anchor.point, p[0], p[1]))
                .map(|(i, p)| {
                    let (dx, dy) = (p[1].x_nm as f64 - p[0].x_nm as f64, p[1].y_nm as f64 - p[0].y_nm as f64);
                    i as f64 + (((anchor.point.x_nm - p[0].x_nm) as f64 * dx + (anchor.point.y_nm - p[0].y_nm) as f64 * dy) / (dx * dx + dy * dy))
                })
                .find(|s| *s >= previous);
            let Some(station) = station else {
                return false;
            };
            previous = station;
        }
        for p in points.windows(3) {
            let (ax, ay) = (p[1].x_nm as i128 - p[0].x_nm as i128, p[1].y_nm as i128 - p[0].y_nm as i128);
            let (bx, by) = (p[2].x_nm as i128 - p[1].x_nm as i128, p[2].y_nm as i128 - p[1].y_nm as i128);
            if ax * by - ay * bx == 0 && ax * bx + ay * by < 0 {
                return false;
            }
        }
        let Some((i, _)) = chain.members.first() else {
            return false;
        };
        let track = &self.tracks[i];
        let clearance = query.membership_clearance(&query.objects()[*i].intended_nets, &query.objects()[*i].intended_nets) as f64;
        for a in 0..points.len().saturating_sub(1) {
            let mut cone = Cone::new(points[a], points[a + 1]);
            for b in a + 1..points.len().saturating_sub(1) {
                work.tick().await;
                cone.extend(points[b], points[b + 1]);
                if b == a + 1 {
                    continue;
                }
                let gap = segment_distance(points[a], points[a + 1], points[b], points[b + 1]);
                let contact = gap == 0.
                    || on_leg(points[a], points[b], points[b + 1])
                    || on_leg(points[a + 1], points[b], points[b + 1])
                    || on_leg(points[b], points[a], points[a + 1])
                    || on_leg(points[b + 1], points[a], points[a + 1]);
                if contact || (!merge && gap < track.width_nm as f64 + clearance && !cone.corner) {
                    return false;
                }
            }
        }
        true
    }
}

/// Exact integer containment for protected stations, independent of floating
/// distance projection and of which collinear vertices survived simplification.
fn on_leg(p: Point, a: Point, b: Point) -> bool {
    (b.x_nm as i128 - a.x_nm as i128) * (p.y_nm as i128 - a.y_nm as i128) == (b.y_nm as i128 - a.y_nm as i128) * (p.x_nm as i128 - a.x_nm as i128)
        && (a.x_nm.min(b.x_nm)..=a.x_nm.max(b.x_nm)).contains(&p.x_nm)
        && (a.y_nm.min(b.y_nm)..=a.y_nm.max(b.y_nm)).contains(&p.y_nm)
}

/// A non-returning corner has all leg directions in a 90-degree cone. Keeping
/// the two extreme integer directions makes every range extension constant work.
struct Cone {
    low: (i128, i128),
    high: (i128, i128),
    corner: bool,
}
impl Cone {
    /// Start with one nonzero direction from bounded native coordinates.
    fn new(a: Point, b: Point) -> Self {
        let d = (b.x_nm as i128 - a.x_nm as i128, b.y_nm as i128 - a.y_nm as i128);
        Self { low: d, high: d, corner: true }
    }
    /// Once the range contains a return, later legs cannot make it a local corner.
    fn extend(&mut self, a: Point, b: Point) {
        if !self.corner {
            return;
        }
        let d = (b.x_nm as i128 - a.x_nm as i128, b.y_nm as i128 - a.y_nm as i128);
        let cross = |a: (i128, i128), b: (i128, i128)| a.0 * b.1 - a.1 * b.0;
        let dot = |a: (i128, i128), b: (i128, i128)| a.0 * b.0 + a.1 * b.1;
        if (cross(self.low, d) == 0 && dot(self.low, d) < 0) || (cross(self.high, d) == 0 && dot(self.high, d) < 0) {
            self.corner = false;
            return;
        }
        if cross(self.low, d) >= 0 && cross(d, self.high) >= 0 {
            return;
        }
        if cross(d, self.high) > 0 && cross(d, self.low) >= 0 {
            self.low = d;
        } else if cross(self.low, d) > 0 && cross(self.high, d) >= 0 {
            self.high = d;
        } else {
            self.corner = false;
        }
    }
}

/// Ordinary command rejection, before any live state/history publication.
fn invalid(message: &str) -> CommandError {
    CommandError::InvalidParams(message.into())
}

/// Owned continuation for independent command-side topology and self-spacing checks.
pub(crate) struct Validation {
    future: ChainFuture,
}
impl Validation {
    /// Capture immutable source authority and submitted paths only; no solver state is trusted.
    pub fn new(query: Arc<ClearanceQuery>, replacements: Vec<Replacement>, merge: bool) -> Self {
        Self {
            future: Box::pin(async move {
                let chains = capture(&query).await?;
                chains.validate_paths(&query, &replacements, merge).await?;
                Ok(chains)
            }),
        }
    }
    /// Each poll advances at most one contact, graph or self-spacing boundary.
    pub fn step(&mut self, budget: usize) -> Option<PreparedChains> {
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..budget {
            if let Poll::Ready(result) = self.future.as_mut().poll(&mut cx) {
                return Some(result);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Coalesced/canceled waiters preserve bounded preparation, warm requests reuse
    /// one result, contended locks never wait, and source/overlay lifetimes stay separate.
    #[test]
    fn topology_preparation_survives_coalescing_without_retaining_its_query() {
        use crate::{
            ADD_TRACKS, INITIALIZE_BOARD_SETUP, Session,
            board::track::tests::{copper_layers, view},
        };
        let mut s = Session::default();
        s.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
        s.apply(INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        for i in 0..20 {
            s.apply(ADD_TRACKS,&serde_json::json!({"layer":copper_layers(&s)[0],"points":[Point{x_nm:i*1_000_000,y_nm:0},Point{x_nm:i*1_000_000,y_nm:4_000_000}],"expected_revision":s.revision()})).unwrap();
        }
        let source = Arc::downgrade(&s.snapshot().unwrap());
        let q = Arc::new(s.clearance_query_with_rules(view(&s).target, &serde_json::json!({})).unwrap());
        let weak = Arc::downgrade(&q);
        let mut cx = Context::from_waker(Waker::noop());
        let guard = q.trace_chains.lock().unwrap();
        let mut blocked = Box::pin(capture(&q));
        assert!(blocked.as_mut().poll(&mut cx).is_pending());
        assert!(blocked.as_mut().poll(&mut cx).is_pending());
        drop(blocked);
        drop(guard);
        let mut result = None;
        for _ in 0..128 {
            let mut waiter = Box::pin(capture(&q));
            for _ in 0..2 {
                if let Poll::Ready(done) = waiter.as_mut().poll(&mut cx) {
                    result = Some(done.unwrap());
                    break;
                }
            }
            if result.is_some() {
                break;
            }
        }
        let result = result.expect("bounded coalesced requests must finish the shared preparation");
        let mut warm = Box::pin(capture(&q));
        assert!(warm.as_mut().poll(&mut cx).is_pending());
        let Poll::Ready(Ok(again)) = warm.as_mut().poll(&mut cx) else { panic!("warm topology must be ready") };
        assert!(Arc::ptr_eq(&result, &again));
        drop(warm);
        let overlay = q.with_objects(q.objects().to_vec()).unwrap();
        assert!(overlay.trace_chains.lock().unwrap().result.is_none());
        drop(overlay);
        drop(q);
        drop(s);
        assert!(weak.upgrade().is_none());
        assert!(source.upgrade().is_none());
    }

    /// Exact integer cone updates agree with an independent separating-axis oracle
    /// across every four-leg sequence in the routing posture, including reversals.
    #[test]
    fn constant_work_cone_matches_separating_axis_oracle() {
        let directions = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
        for code in 0..4096 {
            let mut n = code;
            let mut sequence = Vec::new();
            let p = Point { x_nm: 0, y_nm: 0 };
            let mut cone = None;
            for _ in 0..4 {
                let (x, y) = directions[n % 8];
                n /= 8;
                sequence.push((x, y));
                let end = Point { x_nm: x, y_nm: y };
                match &mut cone {
                    None => cone = Some(Cone::new(p, end)),
                    Some(cone) => cone.extend(p, end),
                }
                let axes: Vec<_> = directions
                    .iter()
                    .copied()
                    .chain((0..8).map(|i| (directions[i].0 + directions[(i + 1) % 8].0, directions[i].1 + directions[(i + 1) % 8].1)))
                    .collect();
                let expected = axes.iter().any(|(x, y)| sequence.iter().all(|(a, b)| x * a + y * b > 0));
                assert_eq!(cone.as_ref().unwrap().corner, expected, "{sequence:?}");
            }
        }
    }
}
