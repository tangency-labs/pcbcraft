//! Walkaround over one captured board. The head goes toward the target in the route's posture;
//! where a leg would break clearance, it follows the blocking copper's hull round, in the
//! cheaper sense, until the posture elbow to the target no longer enters the hull. Copper met
//! while following a hull joins it. Every kept leg is confirmed by the exact clearance probe and
//! the board-outline predicates that DRC uses; where no way round exists the head stops at the
//! last legal point. A final pass replaces runs of corners with elbows where they are clear.
use super::{
    Outcome, Plan, Request,
    hull::Hull,
    posture::{Posture, along, elbow, lattice},
};
use crate::{
    CommandError,
    board::{clearance::ClearanceQuery, drc, geometry::Contour, track::MAX_PATH_POINTS},
    schematic::Point,
};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Most blocking obstacles (or merged clusters) one walk goes round.
const MAX_BLOCKS: usize = 24;
/// Most obstacles merged into one hull while following it.
const MAX_MERGES: usize = 6;
/// Corners one shortcut may skip.
const SHORTCUT_SPAN: usize = 10;
/// Hull growth beyond the exact requirement, covering whole-nanometre rounding of corners.
const MARGIN_NM: f64 = 2.;
/// A blocked leg shorter than this adds no copper: a sliver toward a wall is not progress.
const MIN_REACH_NM: i64 = 1_000;
/// A detour on the previous result's side is kept unless the other is this much shorter.
const HYSTERESIS: f64 = 0.1;

/// Why a walk stopped before finishing.
pub(super) enum Stop {
    /// The probe budget ran out.
    Exhausted,
    /// The caller's cancellation flag was raised.
    Canceled,
    /// The captured query refused a probe (an invalid layer or object index).
    Failed(CommandError),
}

/// The exact verdict for one leg.
struct Leg {
    legal: bool,
    /// Copper of other memberships that breaks clearance, with each pair's actual requirement.
    blockers: Vec<(usize, i64)>,
}

/// One way round a hull, from where the walk's elbow enters it.
struct Detour {
    /// The elbow leg the walk enters the hull on, and the whole-nanometre entry point.
    leg: usize,
    entry: Point,
    /// Hull corners followed after the entry.
    path: Vec<Point>,
    flip: bool,
    cost: f64,
}

/// Where a walk got to; `blocker` names the first copper that stopped a blocked walk.
struct Walked {
    points: Vec<Point>,
    reached: bool,
    blocker: Option<usize>,
}

/// One planning run: the captured query, the route's copper and membership, and its budget.
pub(super) struct Walker<'a> {
    query: &'a ClearanceQuery,
    layers: [Uuid; 1],
    half: f64,
    posture: Posture,
    head: Point,
    target: Point,
    /// A head contact with resolved membership, lending it to every probe.
    intended_from: Option<usize>,
    /// Islands the head touches; the new copper joins them, so DRC exempts their clearance.
    own_islands: BTreeSet<usize>,
    /// Resolved membership of the head's copper (empty for free space).
    route_nets: Vec<usize>,
    outline: drc::Outline,
    budget: usize,
    probes: usize,
    cancel: &'a dyn Fn() -> bool,
    /// Legs already confirmed legal, so repeated checks cost no budget.
    clear: BTreeSet<(i64, i64, i64, i64)>,
    /// The side of the head-to-target line the previous result ran on, if any.
    prefer_side: Option<i8>,
}

impl<'a> Walker<'a> {
    /// Capture the head's copper contacts and the board outline. The request was validated by
    /// the caller; an invalid layer still fails here before any probe is charged.
    pub(super) fn new(query: &'a ClearanceQuery, request: &Request, budget: usize, cancel: &'a dyn Fn() -> bool) -> Result<Self, CommandError> {
        let layers = [request.layer];
        let half = request.width_nm as f64 / 2.;
        let at_head = Contour::capsule(request.head, request.head, half);
        let objects = query.objects();
        let contacts: Vec<usize> = query.candidates(&at_head, &layers)?.into_iter().filter(|i| objects[*i].contour.gap(&at_head) <= 0.).collect();
        let route_nets: Vec<usize> =
            contacts.iter().flat_map(|i| objects[*i].intended_nets.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
        let view = query.authority().view().ok_or_else(|| CommandError::Unavailable("The captured board is no longer available.".into()))?;
        let mut walker = Self {
            query,
            layers,
            half,
            posture: request.posture,
            head: request.head,
            target: request.target,
            intended_from: contacts.iter().copied().find(|i| !objects[*i].intended_nets.is_empty()),
            own_islands: contacts.iter().map(|i| objects[*i].island).collect(),
            route_nets,
            outline: view.outline_edges(),
            budget,
            probes: 0,
            cancel,
            clear: BTreeSet::new(),
            prefer_side: None,
        };
        walker.prefer_side = walker.side(&request.previous);
        Ok(walker)
    }

    /// Probes charged so far; never more than the budget.
    pub(super) fn probes(&self) -> usize {
        self.probes
    }

    /// Plan the route. Errors are budget exhaustion, cancellation or a refused probe.
    pub(super) fn run(&mut self, request: &Request) -> Result<Plan, Stop> {
        let head = self.check(self.head, self.head)?;
        if !head.legal {
            return Ok(self.plan(vec![self.head], Outcome::Blocked { last_legal: self.head }, head.blockers.first().map(|b| b.0)));
        }
        if self.head == self.target {
            return Ok(self.plan(vec![self.head], Outcome::Reached, None));
        }
        if self.continues(&request.previous)? {
            return Ok(self.plan(request.previous.clone(), Outcome::Reached, None));
        }
        let walked = self.walk(self.head, self.target, request.flipped)?;
        if walked.points.len() > MAX_PATH_POINTS {
            return Ok(self.plan(vec![self.head], Outcome::Incomplete, None));
        }
        let last = walked.points.last().copied().unwrap_or(self.head);
        let outcome = if walked.reached { Outcome::Reached } else { Outcome::Blocked { last_legal: last } };
        Ok(self.plan(walked.points, outcome, walked.blocker))
    }

    /// Assemble a result, naming the blocking copper by its query address.
    fn plan(&self, points: Vec<Point>, outcome: Outcome, blocker: Option<usize>) -> Plan {
        let blocker =
            blocker.filter(|_| matches!(outcome, Outcome::Blocked { .. })).and_then(|i| self.query.objects().get(i)).map(|o| o.name.clone());
        Plan { points, outcome, blocker, probes: self.probes }
    }

    /// Whether the previous result still applies unchanged: same ends, posture legs only, and
    /// every leg still legal. Reusing it keeps a stationary pointer's route from oscillating.
    fn continues(&mut self, previous: &[Point]) -> Result<bool, Stop> {
        if previous.len() < 2
            || previous.len() > MAX_PATH_POINTS
            || previous.first() != Some(&self.head)
            || previous.last() != Some(&self.target)
            || !previous.windows(2).all(|leg| self.posture.allows(leg[0], leg[1]))
        {
            return Ok(false);
        }
        for leg in previous.windows(2) {
            if !self.check(leg[0], leg[1])?.legal {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Exactly check the leg `a`–`b` against captured copper and the board outline, charging
    /// one probe unless the same leg was already confirmed. Contact with the head's islands is
    /// exempt unless it joins distinct nets; copper of the route's own single net never becomes
    /// a hull, though it still makes a leg illegal when a short widens the leg's membership.
    fn check(&mut self, a: Point, b: Point) -> Result<Leg, Stop> {
        let key = (a.x_nm, a.y_nm, b.x_nm, b.y_nm);
        if self.clear.contains(&key) {
            return Ok(Leg { legal: true, blockers: Vec::new() });
        }
        if (self.cancel)() {
            return Err(Stop::Canceled);
        }
        if self.probes >= self.budget {
            return Err(Stop::Exhausted);
        }
        self.probes += 1;
        let contour = Contour::capsule(a, b, self.half);
        let results = self.query.probe(&contour, &self.layers, self.intended_from).map_err(Stop::Failed)?;
        let objects = self.query.objects();
        let mut legal = true;
        let mut blockers = Vec::new();
        for decision in results.decisions.iter().filter(|d| d.clearance_violation || d.conflicting_nets) {
            let object = &objects[decision.second];
            if !decision.conflicting_nets && self.own_islands.contains(&object.island) {
                continue;
            }
            legal = false;
            if !(self.route_nets.len() == 1 && object.intended_nets == self.route_nets) {
                blockers.push((decision.second, decision.clearance_nm));
            }
        }
        if !self.outline.polygons.is_empty() {
            let baseline = self.query.authority().rules().values;
            let edge = results.probe_rules.as_ref().map_or(baseline, |rules| rules.hard_values(baseline)).edge_clearance_nm;
            let (distance, crosses) = drc::edge_gap(&self.outline.edges, &contour);
            if !drc::contains(&self.outline.polygons, &contour) || crosses || distance < edge as f64 {
                legal = false;
            }
        }
        if legal {
            self.clear.insert(key);
        }
        Ok(Leg { legal, blockers })
    }

    /// The farthest lattice point of the leg `a`–`b` reached legally, by bisection. Each kept
    /// point was confirmed; `a` is returned when no step, or less than [`MIN_REACH_NM`], is legal.
    fn reach(&mut self, a: Point, b: Point) -> Result<Point, Stop> {
        let (steps, _) = lattice(a, b);
        let (mut lo, mut hi) = (0, steps);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.check(a, along(a, b, mid, steps))?.legal {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Ok(if lo < MIN_REACH_NM { a } else { along(a, b, lo, steps) })
    }

    /// The hull of source copper `index` for this route, grown by its pair requirement.
    fn hull(&self, index: usize, clearance_nm: i64) -> Option<Hull> {
        let object = self.query.objects().get(index)?;
        Hull::grown(&object.contour.core, object.contour.radius + self.half + clearance_nm as f64 + MARGIN_NM, self.posture)
    }

    /// Walk from `from` toward `to`, going round blocking hulls, then shorten and re-confirm.
    fn walk(&mut self, from: Point, to: Point, flipped: bool) -> Result<Walked, Stop> {
        let mut points = vec![from];
        let (mut cur, mut flip, mut reached, mut blocker) = (from, flipped, false, None);
        for _ in 0..MAX_BLOCKS {
            let legs = elbow(cur, to, self.posture, flip);
            if legs.len() < 2 {
                reached = true;
                break;
            }
            let mut failing = None;
            for (i, leg) in legs.windows(2).enumerate() {
                let verdict = self.check(leg[0], leg[1])?;
                if !verdict.legal {
                    failing = Some((i, verdict.blockers));
                    break;
                }
            }
            let Some((leg, blockers)) = failing else {
                points.extend(legs.iter().skip(1));
                reached = true;
                break;
            };
            blocker = blockers.first().map(|b| b.0).or(blocker);
            let (a, b) = (legs[leg], legs[leg + 1]);
            // The blocking hull the leg enters first; ties keep the lower source index.
            let first = blockers
                .iter()
                .filter_map(|&(index, clearance)| {
                    let hull = self.hull(index, clearance)?;
                    let (t, _, _) = hull.inside(a, b)?;
                    Some((t, index, hull))
                })
                .min_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            let Some((_, index, hull)) = first else {
                // A wall (the board edge) or a short with no way round: go as far as is legal.
                points.extend(legs[1..=leg].iter().copied());
                points.push(self.reach(a, b)?);
                break;
            };
            let mut detours = Vec::new();
            for ccw in [true, false] {
                if let Some(detour) = self.detour(&legs, hull.clone(), index, to, ccw, flip)? {
                    detours.push(detour);
                }
            }
            let Some(detour) = self.choose(detours) else {
                points.extend(legs[1..=leg].iter().copied());
                points.push(self.reach(a, b)?);
                break;
            };
            points.extend(legs[1..=detour.leg].iter().copied());
            points.push(detour.entry);
            points.extend(detour.path.iter().copied());
            cur = points.last().copied().unwrap_or(cur);
            flip = detour.flip;
        }
        let mut points = simplify(&points);
        if reached {
            points = self.shortcut(&points, flipped)?;
        }
        // Confirm every kept leg; keep the legal prefix should a hull entry have cut a corner.
        let mut confirmed = vec![from];
        for leg in points.windows(2) {
            if self.check(leg[0], leg[1])?.legal {
                confirmed.push(leg[1]);
            } else {
                confirmed.push(self.reach(leg[0], leg[1])?);
                reached = false;
                break;
            }
        }
        let points = simplify(&confirmed);
        let reached = reached && points.last() == Some(&to);
        Ok(Walked { points, reached, blocker: if reached { None } else { blocker } })
    }

    /// Prefer the detour on the previous result's side unless the other is markedly shorter;
    /// otherwise the shorter, counter-clockwise first on equal cost.
    fn choose(&self, detours: Vec<Detour>) -> Option<Detour> {
        let shortest = detours.iter().map(|d| d.cost).fold(f64::INFINITY, f64::min);
        let preferred = self.prefer_side.and_then(|side| {
            detours.iter().position(|d| {
                let mut path = vec![d.entry];
                path.extend(d.path.iter().copied());
                self.side(&path) == Some(side) && d.cost <= shortest * (1. + HYSTERESIS)
            })
        });
        let index = preferred.or_else(|| detours.iter().position(|d| d.cost <= shortest))?;
        detours.into_iter().nth(index)
    }

    /// The side of the head-to-target line a path runs on, by its summed signed offsets.
    fn side(&self, points: &[Point]) -> Option<i8> {
        let (dx, dy) = (i128::from(self.target.x_nm - self.head.x_nm), i128::from(self.target.y_nm - self.head.y_nm));
        let sum: i128 = points.iter().map(|p| dx * i128::from(p.y_nm - self.head.y_nm) - dy * i128::from(p.x_nm - self.head.x_nm)).sum();
        (sum != 0).then(|| sum.signum() as i8)
    }

    /// Follow `hull` round from where `legs` enter it until an elbow to `to` leaves it, merging
    /// copper met on the way. `None` when this sense meets a wall or no exit within bounds.
    fn detour(&mut self, legs: &[Point], mut hull: Hull, first: usize, to: Point, ccw: bool, flip: bool) -> Result<Option<Detour>, Stop> {
        let mut members = vec![first];
        let mut excluded = false;
        'merge: for _ in 0..=MAX_MERGES {
            let Some((leg, entry)) = entry(legs, &mut hull) else { return Ok(None) };
            if leg == 0 && legs.first() == Some(&entry) {
                // The head lies inside a hull corner's rounding while clear of the exact copper:
                // pull the hull back to it once.
                if excluded {
                    return Ok(None);
                }
                excluded = true;
                hull.exclude(entry);
                continue 'merge;
            }
            let mut path = Vec::new();
            let mut c = entry;
            let corners = hull.corners_from(entry, ccw);
            for v in corners.iter().copied().chain(std::iter::once(entry)) {
                if let Some(exit) = [flip, !flip].into_iter().find(|f| !hull.crossed_by(&elbow(c, to, self.posture, *f))) {
                    let mut walked = legs[..=leg].to_vec();
                    walked.push(entry);
                    walked.extend(path.iter().copied());
                    walked.extend(elbow(c, to, self.posture, exit).into_iter().skip(1));
                    return Ok(Some(Detour { leg, entry, path, flip: exit, cost: length(&walked) }));
                }
                if c == v {
                    continue;
                }
                let verdict = self.check(c, v)?;
                if !verdict.legal {
                    let met = verdict.blockers.into_iter().find(|(index, _)| !members.contains(index));
                    if let Some((index, clearance)) = met
                        && let Some(grown) = self.hull(index, clearance)
                    {
                        hull.merge(&grown);
                        members.push(index);
                        continue 'merge;
                    }
                    return Ok(None);
                }
                path.push(v);
                c = v;
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Replace runs of corners with elbows where those are legal. Ends never move.
    fn shortcut(&mut self, points: &[Point], flip: bool) -> Result<Vec<Point>, Stop> {
        let n = points.len();
        if n < 3 {
            return Ok(points.to_vec());
        }
        let mut out = vec![points[0]];
        let mut i = 0;
        while i + 1 < n {
            let mut next = i + 1;
            let mut via = None;
            'search: for j in (i + 2..=(i + SHORTCUT_SPAN).min(n - 1)).rev() {
                for f in [flip, !flip] {
                    let legs = elbow(points[i], points[j], self.posture, f);
                    if legs.len() >= 2 && self.legal(&legs)? {
                        via = Some(legs);
                        next = j;
                        break 'search;
                    }
                }
            }
            match via {
                Some(legs) => out.extend(legs.into_iter().skip(1)),
                None => out.push(points[next]),
            }
            i = next;
        }
        Ok(simplify(&out))
    }

    /// Whether every leg of the polyline is legal, stopping at the first that is not.
    fn legal(&mut self, points: &[Point]) -> Result<bool, Stop> {
        for leg in points.windows(2) {
            if !self.check(leg[0], leg[1])?.legal {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Where the polyline first enters `hull`: the leg and a whole-nanometre point on its lattice.
/// A diagonal leg meeting a perpendicular diagonal edge between lattice points widens that
/// edge by one unit so the entry is exact; otherwise the entry stays just outside.
fn entry(legs: &[Point], hull: &mut Hull) -> Option<(usize, Point)> {
    for attempt in 0..2 {
        let (i, a, b, through) =
            legs.windows(2).enumerate().find_map(|(i, leg)| hull.inside(leg[0], leg[1]).map(|(_, _, through)| (i, leg[0], leg[1], through)))?;
        let (steps, unit) = lattice(a, b);
        let Some(k) = through else { return Some((i, a)) };
        match hull.boundary_step(k, a, unit) {
            Some(m) if (0..=steps).contains(&m) => return Some((i, along(a, b, m, steps))),
            _ if attempt == 0 => hull.widen(k),
            _ => {
                let (t, _, _) = hull.inside(a, b)?;
                let m = ((t * steps as f64).floor() as i64).clamp(0, steps);
                return Some((i, along(a, b, m, steps)));
            }
        }
    }
    None
}

/// Total centre-line length in nanometres.
fn length(points: &[Point]) -> f64 {
    points.windows(2).map(|leg| ((leg[1].x_nm - leg[0].x_nm) as f64).hypot((leg[1].y_nm - leg[0].y_nm) as f64)).sum()
}

/// Drop repeated points and corners between collinear legs. Every dropped corner lies on legs
/// already kept, so the remaining legs cover only copper that was checked.
pub(super) fn simplify(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = i128::from(b.x_nm - a.x_nm) * i128::from(p.y_nm - b.y_nm) - i128::from(b.y_nm - a.y_nm) * i128::from(p.x_nm - b.x_nm);
            let forward = i128::from(b.x_nm - a.x_nm) * i128::from(p.x_nm - b.x_nm) + i128::from(b.y_nm - a.y_nm) * i128::from(p.y_nm - b.y_nm);
            if cross == 0 && forward > 0 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}
