//! Bounded approaches to legal clearance boundaries near an obstructed walkaround cursor.
use super::*;

const MAX_BLOCKERS: usize = 4;
const MAX_CANDIDATES: usize = 4;
const MAX_CHECKS: usize = 64;

impl Walker<'_> {
    /// Keep a blocked endpoint near the cursor by trying nearby grown-hull projections.
    /// Every candidate and its complete route are exact-checked with the original pickup
    /// intent. The cursor remains blocked; `adjusted_target` identifies the certified endpoint
    /// clients may fix without claiming a connection to copper under the cursor.
    pub(super) async fn approach_target(&mut self, request: &Request, goal: &Leg) -> Result<Option<Plan>, Stop> {
        let mut candidates = Vec::new();
        let mut edges = Vec::new();
        for &(index, clearance) in goal.blockers.iter().take(MAX_BLOCKERS) {
            super::super::search::yield_unit().await;
            let Some(hull) = self.hull(index, clearance) else { continue };
            let corners = hull.corners_from(request.target, true);
            for (i, &a) in corners.iter().enumerate() {
                let b = corners[(i + 1) % corners.len()];
                edges.push((a, b, index));
                let point = project(request.target, a, b);
                if crate::wire::valid_point(&point) {
                    candidates.push((point, index));
                }
            }
        }
        // A cursor inside two clearance bands may project onto an edge that is still
        // obstructed by the other body. Their boundary intersections supply the legal corner.
        for (i, &(a, b, index)) in edges.iter().enumerate() {
            for &(c, d, other) in &edges[i + 1..] {
                if index != other
                    && let Some(point) = intersection(a, b, c, d)
                    && crate::wire::valid_point(&point)
                {
                    candidates.push((point, index));
                }
            }
        }
        let previous = request.previous.last().copied().unwrap_or(request.head);
        let continuing = !request.previous.is_empty() || request.origin.is_some_and(|(point, _)| point != request.head);
        candidates.sort_by_key(|(point, index)| {
            let approach = if continuing {
                posture_distance(previous, *point, self.posture) + posture_distance(*point, request.target, self.posture)
            } else {
                distance(*point, request.target)
            };
            (approach, distance(*point, request.target), distance(*point, previous), point.x_nm, point.y_nm, *index)
        });
        candidates.retain(|(point, _)| *point != request.head);
        candidates.dedup_by_key(|(point, _)| (point.x_nm, point.y_nm));
        let mut searches = 0;
        for (point, blocker) in candidates.into_iter().take(MAX_CHECKS) {
            if !self.check(point, point).await?.legal {
                continue;
            }
            if searches >= MAX_CANDIDATES {
                break;
            }
            searches += 1;
            let mut nearby = request.clone();
            nearby.target = point;
            nearby.origin = Some(request.origin.unwrap_or((request.head, request.layer)));
            let mut walker = Self::new(self.query, &nearby, self.budget, self.progress.clone()).map_err(Stop::Failed)?;
            let first = walker.run(&nearby).await;
            self.probes = walker.probes();
            let result = match first {
                Ok(plan) if plan.outcome == Outcome::Reached => Ok(plan),
                Ok(plan) if !plan.start_blocked => walker.finish_target(&nearby, plan).await,
                Ok(_) => continue,
                Err(Stop::Exhausted) if walker.probes < walker.budget => {
                    let fallback = walker.plan(vec![nearby.head], Outcome::Incomplete, None);
                    walker.finish_target(&nearby, fallback).await
                }
                Err(error) => Err(error),
            };
            self.probes = walker.probes();
            let mut plan = match result {
                Ok(plan) => plan,
                Err(Stop::Exhausted) if self.probes < self.budget => continue,
                Err(stop) => return Err(stop),
            };
            if plan.outcome == Outcome::Reached {
                plan.outcome = Outcome::Blocked { last_legal: point };
                plan.adjusted_target = Some(point);
                plan.blocker = Some(self.query.objects()[blocker].name.clone());
                return Ok(Some(plan));
            }
        }
        Ok(None)
    }
}

/// Nearest whole-nanometre lattice point on a posture-aligned grown-hull edge.
fn project(point: Point, a: Point, b: Point) -> Point {
    let (steps, unit) = lattice(a, b);
    if steps == 0 {
        return a;
    }
    let dot = i128::from(point.x_nm - a.x_nm) * i128::from(unit.x_nm) + i128::from(point.y_nm - a.y_nm) * i128::from(unit.y_nm);
    let norm = i128::from(unit.x_nm).pow(2) + i128::from(unit.y_nm).pow(2);
    let step = (dot + norm / 2).div_euclid(norm).clamp(0, i128::from(steps)) as i64;
    along(a, b, step, steps)
}

/// Stable squared distance for ranking bounded board coordinates without square roots.
fn distance(a: Point, b: Point) -> i128 {
    i128::from(a.x_nm - b.x_nm).pow(2) + i128::from(a.y_nm - b.y_nm).pow(2)
}

/// Intersect two finite posture edges on the whole-nanometre lattice. Unit directions
/// bound the determinant to two, avoiding large-coordinate cross-product multiplication.
fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let (steps, u) = lattice(a, b);
    let (other_steps, v) = lattice(c, d);
    let cross = |x: Point, y: Point| i128::from(x.x_nm) * i128::from(y.y_nm) - i128::from(x.y_nm) * i128::from(y.x_nm);
    let mut denominator = cross(u, v);
    if steps == 0 || other_steps == 0 || denominator == 0 {
        return None;
    }
    let delta = Point { x_nm: c.x_nm - a.x_nm, y_nm: c.y_nm - a.y_nm };
    let (mut t, mut s) = (cross(delta, v), cross(delta, u));
    if denominator < 0 {
        (denominator, t, s) = (-denominator, -t, -s);
    }
    if !(0..=i128::from(steps) * denominator).contains(&t) || !(0..=i128::from(other_steps) * denominator).contains(&s) {
        return None;
    }
    let x = i128::from(a.x_nm) * denominator + i128::from(u.x_nm) * t;
    let y = i128::from(a.y_nm) * denominator + i128::from(u.y_nm) * t;
    if x % denominator != 0 || y % denominator != 0 {
        return None;
    }
    Some(Point { x_nm: i64::try_from(x / denominator).ok()?, y_nm: i64::try_from(y / denominator).ok()? })
}

/// Fixed-point octile/Manhattan distance ranks approaches by the route through the candidate,
/// so a nearby face across an obstacle does not automatically outrank the reachable inside corner.
fn posture_distance(a: Point, b: Point, posture: Posture) -> i128 {
    let (x, y) = (i128::from(a.x_nm - b.x_nm).abs(), i128::from(a.y_nm - b.y_nm).abs());
    match posture {
        Posture::Diagonal => x.max(y) * 1_000_000 + x.min(y) * 414_214,
        Posture::Orthogonal => (x + y) * 1_000_000,
    }
}
