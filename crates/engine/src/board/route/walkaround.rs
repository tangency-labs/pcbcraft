//! Walkaround over one captured board. The head goes toward the target in the route's posture;
//! where a leg would break clearance, it follows the blocking copper's hull round, in the
//! cheaper sense, until the posture elbow to the target no longer enters the hull. Copper met
//! while following a hull joins it. Every kept leg is confirmed by the exact clearance probe and
//! the board-outline predicates that DRC uses; where no way round exists the head stops at the
//! last legal point. A final pass replaces runs of corners with elbows where they are clear.
mod alternatives;
mod approach;
mod portals;
mod tidy;

use super::{
    Outcome, Plan, Request,
    hull::Hull,
    posture::{Posture, along, elbow, lattice},
};
use crate::{
    CommandError,
    board::{clearance::RouteObstacles, drc, geometry::Contour, track::MAX_PATH_POINTS},
    schematic::Point,
};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Most blocking obstacles (or merged clusters) one walk goes round.
const MAX_BLOCKS: usize = 24;
/// Most times a blocked walk returns to an earlier obstacle to go round its other side.
const MAX_RETRIES: usize = 8;
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
    /// The probe budget or a search bound ran out.
    Exhausted,
    /// The captured query refused a probe (an invalid layer or object index).
    Failed(CommandError),
}

/// The exact verdict for one leg.
struct Leg {
    legal: bool,
    edge_blocked: bool,
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

/// The other way round an earlier obstacle: the walked points before it, the points that go
/// round it and the elbow's flip after it. A walk that blocks further on resumes from here.
struct Branch {
    points: Vec<Point>,
    round: Vec<Point>,
    flip: bool,
    blocks: usize,
}

/// Where a walk got to; `blocker` names the first copper that stopped a blocked walk.
struct Walked {
    points: Vec<Point>,
    reached: bool,
    blocker: Option<usize>,
}

/// One planning run: the captured query, the route's copper and membership, and its budget.
pub(super) struct Walker<'a> {
    query: RouteObstacles<'a>,
    layers: [Uuid; 1],
    half: f64,
    posture: Posture,
    head: Point,
    target: Point,
    /// An original gesture contact with resolved membership, lending it to every probe.
    intended_from: Option<usize>,
    /// Islands the head touches; the new copper joins them, so DRC exempts their clearance.
    own_islands: BTreeSet<usize>,
    /// Resolved membership of the original contact (empty for unassigned/free copper).
    route_nets: Vec<usize>,
    /// Displacement permits the original segment's direct contacts, not every member of its net.
    displacement_contacts: Option<BTreeSet<String>>,
    /// Source-line half-plane selected by the incoming push, not the cheapest walk direction.
    displacement_side: Option<(Point, Point, i128)>,
    /// Complete net membership during final connected-path validation only. Search probes
    /// must not inherit contacts from speculative detours, retries or discarded previous plans.
    path_membership: Option<Vec<usize>>,
    outline: drc::Outline,
    budget: usize,
    probes: usize,
    progress: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Legs already confirmed legal, so repeated checks cost no budget.
    clear: BTreeSet<(i64, i64, i64, i64)>,
    /// The side of the head-to-target line the previous result ran on, if any.
    prefer_side: Option<i8>,
}

impl<'a> Walker<'a> {
    /// Capture the head's copper contacts and the board outline. The request was validated by
    /// the caller; an invalid layer still fails here before any probe is charged.
    pub(super) fn new(
        query: impl Into<RouteObstacles<'a>>,
        request: &Request,
        budget: usize,
        progress: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Self, CommandError> {
        let query = query.into();
        let layers = [request.layer];
        let half = request.width_nm as f64 / 2.;
        let at_head = Contour::capsule(request.head, request.head, half);
        let objects = query;
        let head_candidates = query.candidates(&at_head, &layers)?;
        let (origin, origin_layer, origin_width) = if request.mode == super::Mode::Shove {
            super::shove::pickup_geometry(request)
        } else {
            let (origin, layer) = request.origin.unwrap_or((request.head, request.layer));
            (origin, layer, request.width_nm)
        };
        let at_origin = Contour::capsule(origin, origin, origin_width as f64 / 2.);
        let candidates = if (origin, origin_layer, origin_width) == (request.head, request.layer, request.width_nm) {
            head_candidates
        } else {
            query.candidates(&at_origin, &[origin_layer])?
        };
        let contacts: Vec<usize> = candidates.into_iter().filter(|i| !objects[*i].mechanical && objects[*i].contour.gap(&at_origin) <= 0.).collect();
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
            displacement_contacts: None,
            displacement_side: None,
            path_membership: None,
            outline: view.outline_edges(),
            budget,
            probes: progress.load(std::sync::atomic::Ordering::Relaxed),
            progress,
            clear: BTreeSet::new(),
            prefer_side: None,
        };
        walker.prefer_side = walker.side(&request.previous);
        Ok(walker)
    }

    /// Displaced fragments retain their original intent/island even while their source is
    /// excluded from the speculative obstacle projection. Final topology is validated separately.
    pub(super) fn inherit(&mut self, object: &crate::board::clearance::Object, contacts: Option<BTreeSet<String>>) {
        self.route_nets = object.intended_nets.clone();
        self.displacement_contacts = contacts;
        self.own_islands = [object.island].into_iter().collect();
        self.intended_from = None;
    }

    /// Restrict displacement to the side ahead of the incoming gesture. A parallel approach
    /// pushes away from its final position; a collinear approach imposes no arbitrary side.
    pub(super) fn push_forward(&mut self, start: Point, end: Point, from: Point, toward: Point) {
        let cross = |x: i128, y: i128| (end.x_nm as i128 - start.x_nm as i128) * y - (end.y_nm as i128 - start.y_nm as i128) * x;
        let motion = cross(toward.x_nm as i128 - from.x_nm as i128, toward.y_nm as i128 - from.y_nm as i128);
        let side = if motion != 0 {
            motion.signum()
        } else {
            -cross(toward.x_nm as i128 - start.x_nm as i128, toward.y_nm as i128 - start.y_nm as i128).signum()
        };
        self.displacement_side = (side != 0).then_some((start, end, side));
    }

    /// Test a soft trace bend with the same inherited membership/contact policy as
    /// its eventual displacement. An obstructed hint may move; protected contacts may not.
    pub(super) async fn displacement_anchor(&mut self, point: Point) -> Result<bool, Stop> {
        Ok(self.check(point, point).await?.legal)
    }

    /// Probes charged so far; never more than the budget.
    pub(super) fn probes(&self) -> usize {
        self.probes
    }

    /// Plan the route, yielding between work units. Errors are budget exhaustion or a refused probe; the owner cancels by dropping the continuation.
    pub(super) async fn run(&mut self, request: &Request) -> Result<Plan, Stop> {
        let head = self.check(self.head, self.head).await?;
        if !head.legal {
            let mut plan = self.plan(vec![self.head], Outcome::Blocked { last_legal: self.head }, head.blockers.first().map(|b| b.0));
            plan.start_blocked = true;
            return Ok(plan);
        }
        if self.head == self.target {
            return Ok(self.plan(vec![self.head], Outcome::Reached, None));
        }
        if self.continues(&request.previous).await? {
            return Ok(self.plan(request.previous.clone(), Outcome::Reached, None));
        }
        let walked = self.walk(self.head, self.target, request.flipped).await?;
        if walked.points.len() > MAX_PATH_POINTS {
            return Ok(self.plan(vec![self.head], Outcome::Incomplete, None));
        }
        let last = walked.points.last().copied().unwrap_or(self.head);
        let outcome = if walked.reached { Outcome::Reached } else { Outcome::Blocked { last_legal: last } };
        Ok(self.plan(walked.points, outcome, walked.blocker))
    }

    /// Resolve an exact target, retry legal concave pockets from their other end, or approach
    /// an obstructed walkaround cursor's clearance boundary. All searches retain the original
    /// pickup intent and global budget; a nearby boundary stays explicitly blocked.
    pub(super) async fn run_target(&mut self, request: &Request) -> Result<Plan, Stop> {
        let first = match self.run(request).await {
            Ok(plan) if plan.outcome == Outcome::Reached || plan.start_blocked => return Ok(plan),
            Err(Stop::Failed(error)) => return Err(Stop::Failed(error)),
            result => result,
        };
        if self.probes >= self.budget {
            return first;
        }
        let goal = match self.check(request.target, request.target).await {
            Ok(goal) => goal,
            Err(Stop::Exhausted) if request.mode == super::Mode::Walkaround => return self.refinement_exhausted(first),
            Err(stop) => return Err(stop),
        };
        if !goal.legal {
            if request.mode == super::Mode::Walkaround {
                match self.approach_target(request, &goal).await {
                    Ok(Some(plan)) => return Ok(plan),
                    Ok(None) => {}
                    Err(Stop::Exhausted) => return self.refinement_exhausted(first),
                    Err(stop) => return Err(stop),
                }
            }
            return first.map(|mut plan| {
                // Shove can try a board-edge prefix on its overlay. Placement still
                // requires successful propagation, and a copper blocker stays blocked.
                if request.mode == super::Mode::Shove
                    && goal.edge_blocked
                    && plan.blocker.is_none()
                    && plan.points.len() >= 2
                    && let Outcome::Blocked { last_legal } = plan.outcome
                {
                    plan.adjusted_target = Some(last_legal);
                }
                plan.probes = self.probes;
                plan
            });
        }
        let first = match first {
            Ok(plan) => plan,
            Err(Stop::Exhausted) if request.mode == super::Mode::Walkaround => self.plan(vec![request.head], Outcome::Incomplete, None),
            Err(stop) => return Err(stop),
        };
        self.finish_target(request, first).await
    }

    /// Preserve only a usable certified prefix when optional work spends the remaining budget.
    fn refinement_exhausted(&self, first: Result<Plan, Stop>) -> Result<Plan, Stop> {
        let mut first = first?;
        if first.points.len() < 2 {
            return Err(Stop::Exhausted);
        }
        first.probes = self.probes;
        Ok(first)
    }

    /// Refine an exact legal goal without discarding a certified fallback on a local
    /// search limit. All attempts share the captured intent and global probe budget.
    async fn finish_target(&mut self, request: &Request, first: Plan) -> Result<Plan, Stop> {
        let mut fallback = match self.reverse_target(request, first.clone()).await {
            Ok(plan) if plan.outcome == Outcome::Reached => return Ok(plan),
            Ok(plan) => plan,
            Err(Stop::Exhausted) if request.mode == super::Mode::Walkaround && self.probes < self.budget => first,
            Err(Stop::Exhausted) if request.mode == super::Mode::Walkaround => return self.refinement_exhausted(Ok(first)),
            Err(stop) => return Err(stop),
        };
        if request.mode == super::Mode::Walkaround {
            match self.portal_target(request).await {
                Ok(Some(plan)) => return Ok(plan),
                Ok(None) => {}
                Err(Stop::Exhausted) => return self.refinement_exhausted(Ok(fallback)),
                Err(stop) => return Err(stop),
            }
        }
        fallback.probes = self.probes;
        Ok(fallback)
    }

    /// Approach an already-legal goal from its other end, retaining the original contact
    /// authority and only adopting a completely reached, exact-confirmed reverse path.
    async fn reverse_target(&mut self, request: &Request, mut first: Plan) -> Result<Plan, Stop> {
        let mut reverse_request = request.clone();
        reverse_request.head = request.target;
        reverse_request.target = request.head;
        reverse_request.origin = Some(request.origin.unwrap_or((request.head, request.layer)));
        reverse_request.flipped = !request.flipped;
        reverse_request.previous.reverse();
        let mut reverse = Self::new(self.query, &reverse_request, self.budget, self.progress.clone()).map_err(Stop::Failed)?;
        let result = reverse.run(&reverse_request).await;
        self.probes = reverse.probes();
        match result {
            Ok(mut plan) if plan.outcome == Outcome::Reached => {
                plan.points.reverse();
                Ok(self.plan(plan.points, Outcome::Reached, None))
            }
            Ok(_) => {
                first.probes = self.probes;
                Ok(first)
            }
            Err(stop) => Err(stop),
        }
    }

    /// Assemble a result, naming the blocking copper by its query address.
    fn plan(&self, points: Vec<Point>, outcome: Outcome, blocker: Option<usize>) -> Plan {
        let blocker = blocker.filter(|_| matches!(outcome, Outcome::Blocked { .. })).and_then(|i| self.query.get(i)).map(|o| o.name.clone());
        Plan { adjusted_target: None, start_blocked: false, replacements: Vec::new(), shove: None, points, outcome, blocker, probes: self.probes }
    }

    /// Certify an incoming shove candidate against the same geometry and membership as
    /// walkaround. A guide gains no authority if any leg clips a fixed object or edge.
    pub(super) async fn certify_candidate(&mut self, points: &[Point]) -> Result<Option<Plan>, Stop> {
        Ok(self.continues(points).await?.then(|| self.plan(points.to_vec(), Outcome::Reached, None)))
    }

    /// Whether the previous result still applies unchanged: same ends, posture legs only, and
    /// every leg still legal. Reusing it keeps a stationary pointer's route from oscillating.
    async fn continues(&mut self, previous: &[Point]) -> Result<bool, Stop> {
        if previous.len() < 2
            || previous.len() > MAX_PATH_POINTS
            || previous.first() != Some(&self.head)
            || previous.last() != Some(&self.target)
            || !previous.windows(2).all(|leg| self.posture.allows(leg[0], leg[1]))
        {
            return Ok(false);
        }
        Ok(self.confirm(previous.to_vec()).await?.points == previous)
    }

    /// Exactly check the leg `a`–`b` against captured copper and the board outline, charging
    /// one probe unless the same leg was already confirmed. Contact with the head's islands is
    /// exempt unless it joins distinct nets; copper of the route's own single net never becomes
    /// a hull, though it still makes a leg illegal when a short widens the leg's membership.
    async fn check(&mut self, a: Point, b: Point) -> Result<Leg, Stop> {
        let key = (a.x_nm, a.y_nm, b.x_nm, b.y_nm);
        if self.clear.contains(&key) {
            return Ok(Leg { legal: true, edge_blocked: false, blockers: Vec::new() });
        }
        super::search::yield_unit().await;
        if self.probes >= self.budget {
            return Err(Stop::Exhausted);
        }
        self.probes += 1;
        self.progress.store(self.probes, std::sync::atomic::Ordering::Relaxed);
        let contour = Contour::capsule(a, b, self.half);
        let results = match &self.path_membership {
            Some(nets) => self.query.probe_membership(&contour, &self.layers, nets, None),
            None => self.query.probe_membership(&contour, &self.layers, &self.route_nets, self.intended_from),
        }
        .map_err(Stop::Failed)?;
        let objects = self.query;
        let mut legal = self.displacement_side.is_none_or(|(start, end, side)| {
            [a, b].iter().all(|p| {
                side * ((end.x_nm as i128 - start.x_nm as i128) * (p.y_nm as i128 - start.y_nm as i128)
                    - (end.y_nm as i128 - start.y_nm as i128) * (p.x_nm as i128 - start.x_nm as i128))
                    >= 0
            })
        });
        let mut blockers = Vec::new();
        for decision in &results.decisions {
            let object = &objects[decision.second];
            // Only the starting contact's resolved net grants a clearance exemption. A probe combines
            // contacted nets for short/class checks; that union cannot assign this route a
            // foreign net merely by crossing it, including during previous-path confirmation.
            let same_net = self.displacement_contacts.is_none() && self.route_nets.len() == 1 && object.intended_nets == self.route_nets;
            let clearance_violation =
                if object.mechanical { decision.clearance_violation } else { !same_net && decision.gap_nm < decision.clearance_nm as f64 };
            if !clearance_violation && !decision.conflicting_nets {
                continue;
            }
            let permitted_contact = self
                .displacement_contacts
                .as_ref()
                .map_or_else(|| self.own_islands.contains(&object.island), |contacts| contacts.contains(&object.name));
            if !object.mechanical && !decision.conflicting_nets && permitted_contact {
                continue;
            }
            legal = false;
            if !same_net {
                blockers.push((decision.second, decision.clearance_nm));
            }
        }
        let baseline = self.query.authority().rules().values;
        let rules = results.probe_rules.as_ref().map_or(baseline, |rules| rules.hard_values(baseline));
        if self.half * 2. < rules.min_track_width_nm as f64 {
            legal = false;
        }
        let edge_blocked = !self.outline.allows_copper(&contour, rules.edge_clearance_nm);
        legal &= !edge_blocked;
        if legal {
            self.clear.insert(key);
        }
        Ok(Leg { legal, edge_blocked, blockers })
    }

    /// The farthest lattice point of the leg `a`–`b` reached legally, by bisection. Each kept
    /// point was confirmed; `a` is returned when no step, or less than [`MIN_REACH_NM`], is legal.
    async fn reach(&mut self, a: Point, b: Point) -> Result<Point, Stop> {
        let (steps, _) = lattice(a, b);
        let (mut lo, mut hi) = (0, steps);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.check(a, along(a, b, mid, steps)).await?.legal {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Ok(if lo < MIN_REACH_NM { a } else { along(a, b, lo, steps) })
    }

    /// The hull of source copper `index` for this route, grown by its pair requirement.
    fn hull(&self, index: usize, clearance_nm: i64) -> Option<Hull> {
        let object = self.query.get(index)?;
        let mut hull = Hull::grown(&object.contour.core, object.contour.radius + self.half + clearance_nm as f64 + MARGIN_NM, self.posture)?;
        if self.displacement_side.is_some()
            && let Some(group) = self.query.guide_group(index)
        {
            for sibling in group.filter(|i| *i != index) {
                let other = &self.query[sibling];
                if other.layers.iter().any(|l| self.layers.contains(l))
                    && let Some(grown) =
                        Hull::grown(&other.contour.core, other.contour.radius + self.half + clearance_nm as f64 + MARGIN_NM, self.posture)
                {
                    hull.merge(&grown);
                }
            }
        }
        Some(hull)
    }

    /// Walk from `from` toward `to`, going round blocking hulls, then shorten and re-confirm.
    /// Each obstacle is gone round on the preferred side first. Should the walk then block,
    /// it resumes from the most recent obstacle's other side, at most [`MAX_RETRIES`] times;
    /// when every try blocks, the first try's legal prefix and blocker are reported.
    async fn walk(&mut self, from: Point, to: Point, flipped: bool) -> Result<Walked, Stop> {
        let mut points = vec![from];
        let (mut cur, mut flip, mut reached, mut blocker) = (from, flipped, false, None);
        let (mut blocks, mut retries) = (0, 0);
        let mut branches: Vec<Branch> = Vec::new();
        let mut first_block: Option<(Vec<Point>, Option<usize>)> = None;
        loop {
            let ended = if blocks >= MAX_BLOCKS {
                return Err(Stop::Exhausted);
            } else {
                blocks += 1;
                let legs = elbow(cur, to, self.posture, flip);
                if legs.len() < 2 {
                    reached = true;
                    break;
                }
                let mut failing = None;
                for (i, leg) in legs.windows(2).enumerate() {
                    let verdict = self.check(leg[0], leg[1]).await?;
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
                let mut detours = Vec::new();
                if let Some((_, index, hull)) = first {
                    for ccw in [true, false] {
                        if let Some(detour) = self.detour(&legs, hull.clone(), index, to, ccw, flip).await? {
                            detours.push(detour);
                        }
                    }
                }
                let mut ordered = self.order(detours).into_iter().map(|detour| {
                    let mut round = legs[1..=detour.leg].to_vec();
                    round.push(detour.entry);
                    round.extend(detour.path.iter().copied());
                    (round, detour.flip)
                });
                match ordered.next() {
                    Some((round, exit)) => {
                        if let Some((other, other_flip)) = ordered.next() {
                            branches.push(Branch { points: points.clone(), round: other, flip: other_flip, blocks });
                        }
                        points.extend(round);
                        cur = points.last().copied().unwrap_or(cur);
                        flip = exit;
                        false
                    }
                    None => {
                        // A wall (the board edge), a short or copper with no way round: go as
                        // far as is legal.
                        points.extend(legs[1..=leg].iter().copied());
                        points.push(self.reach(a, b).await?);
                        true
                    }
                }
            };
            if !ended {
                continue;
            }
            if first_block.is_none() {
                first_block = Some((points.clone(), blocker));
            }
            if !branches.is_empty() && retries >= MAX_RETRIES {
                return Err(Stop::Exhausted);
            }
            let Some(branch) = branches.pop() else { break };
            retries += 1;
            points = branch.points;
            points.extend(branch.round);
            cur = points.last().copied().unwrap_or(cur);
            flip = branch.flip;
            blocks = branch.blocks;
        }
        if !reached && let Some((first, first_blocker)) = first_block {
            points = first;
            blocker = first_blocker;
        }
        let mut points = simplify(&points);
        if reached {
            points = self.shortcut(&points, flipped).await?;
        }
        let mut confirmed = self.confirm(points).await?;
        confirmed.reached &= reached && confirmed.points.last() == Some(&to);
        if !confirmed.reached && confirmed.blocker.is_none() {
            confirmed.blocker = blocker;
        }
        Ok(confirmed)
    }

    /// Revalidate a complete connected path, including earlier legs, with the union of every
    /// physical contact's net membership. Only head-contact islands are exempt from clearance;
    /// crossing another island does not authorize joining it. If a leg fails, shorten to its legal prefix
    /// and recompute membership: discarded contacts must not authorize that prefix. Speculative
    /// search/reuse checks never retain this context or its cached clear legs.
    async fn confirm(&mut self, points: Vec<Point>) -> Result<Walked, Stop> {
        let result = self.confirm_path(points).await;
        self.path_membership = None;
        self.clear.clear();
        result
    }

    /// Trim failing legs until the entire prefix is legal under its own complete membership.
    /// Each pass discards a segment or shortens its last leg; the explicit bound also covers
    /// pathological changes in net-class rules while contacts disappear.
    async fn confirm_path(&mut self, mut points: Vec<Point>) -> Result<Walked, Stop> {
        let mut reached = true;
        let mut blocker = None;
        for _ in 0..MAX_PATH_POINTS * 2 {
            if points.len() > MAX_PATH_POINTS {
                return Err(Stop::Exhausted);
            }
            let mut contacts = BTreeSet::new();
            for leg in points.windows(2) {
                super::search::yield_unit().await;
                let contour = Contour::capsule(leg[0], leg[1], self.half);
                for index in self.query.candidates(&contour, &self.layers).map_err(Stop::Failed)? {
                    if self.query[index].contour.gap(&contour) <= 0. {
                        contacts.insert(index);
                    }
                }
            }
            let objects = self.query;
            let nets = self
                .route_nets
                .iter()
                .copied()
                .chain(contacts.iter().flat_map(|i| objects[*i].intended_nets.iter().copied()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            self.path_membership = Some(nets);
            self.clear.clear();
            let mut trimmed = None;
            for (index, leg) in points.windows(2).enumerate() {
                let verdict = self.check(leg[0], leg[1]).await?;
                if !verdict.legal {
                    blocker = verdict.blockers.first().map(|b| b.0).or(blocker);
                    let last = self.reach(leg[0], leg[1]).await?;
                    trimmed = Some((index, last));
                    break;
                }
            }
            let Some((index, last)) = trimmed else {
                return Ok(Walked { points, reached, blocker });
            };
            reached = false;
            points.truncate(index + 1);
            if points.last() != Some(&last) {
                points.push(last);
            }
            points = simplify(&points);
        }
        Err(Stop::Exhausted)
    }

    /// Order the detours to try: the one on the previous result's side first unless the other
    /// is markedly shorter; otherwise the shorter, counter-clockwise first on equal cost.
    fn order(&self, mut detours: Vec<Detour>) -> Vec<Detour> {
        let shortest = detours.iter().map(|d| d.cost).fold(f64::INFINITY, f64::min);
        let preferred = self.prefer_side.and_then(|side| {
            detours.iter().position(|d| {
                let mut path = vec![d.entry];
                path.extend(d.path.iter().copied());
                self.side(&path) == Some(side) && d.cost <= shortest * (1. + HYSTERESIS)
            })
        });
        if let Some(index) = preferred.or_else(|| detours.iter().position(|d| d.cost <= shortest)) {
            let chosen = detours.remove(index);
            detours.insert(0, chosen);
        }
        detours
    }

    /// The side of the head-to-target line a path runs on, by its summed signed offsets.
    fn side(&self, points: &[Point]) -> Option<i8> {
        let (dx, dy) = (i128::from(self.target.x_nm - self.head.x_nm), i128::from(self.target.y_nm - self.head.y_nm));
        let sum: i128 = points.iter().map(|p| dx * i128::from(p.y_nm - self.head.y_nm) - dy * i128::from(p.x_nm - self.head.x_nm)).sum();
        (sum != 0).then(|| sum.signum() as i8)
    }

    /// Follow `hull` round from where `legs` enter it until an elbow to `to` leaves it, merging
    /// copper met on the way. `None` when this sense meets a wall or has no exit; exhausted merge bounds propagate.
    async fn detour(&mut self, legs: &[Point], mut hull: Hull, first: usize, to: Point, ccw: bool, flip: bool) -> Result<Option<Detour>, Stop> {
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
                let verdict = self.check(c, v).await?;
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
        Err(Stop::Exhausted)
    }

    /// Replace runs of corners with elbows where those are legal. Ends never move.
    async fn shortcut(&mut self, points: &[Point], flip: bool) -> Result<Vec<Point>, Stop> {
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
                    if legs.len() >= 2 && self.legal(&legs).await? {
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
    async fn legal(&mut self, points: &[Point]) -> Result<bool, Stop> {
        for leg in points.windows(2) {
            if !self.check(leg[0], leg[1]).await?.legal {
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
