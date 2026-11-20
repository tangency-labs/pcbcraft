//! Bounded parallel displacement alternatives when a local hull walk cannot find a channel.
use super::*;
use std::collections::BTreeMap;

/// Retained parallel guide intervals, independent of source-fragment count.
const MAX_BANDS: usize = 64;
/// Optional compacting work, charged inside the gesture's global probe budget.
const MAX_LOCAL_PROBES: usize = 128;

/// A parallel run bounded by its first and last source-tangent projections.
struct Band {
    first: (i128, Point),
    last: (i128, Point),
}

impl Walker<'_> {
    /// Prefer a compact, exactly certified displacement when the first hull walk
    /// spreads far along the source. This optional search shares the gesture budget,
    /// uses at most 128 additional probes and preserves its certified fallback on
    /// exhaustion. Query failures still propagate to the caller.
    pub(in crate::board::route) async fn localize_displacement(&mut self, plan: Plan, request: &Request) -> Result<Plan, CommandError> {
        if plan.points.len() < 3 {
            return Ok(plan);
        }
        let budget = self.budget;
        self.budget = budget.min(self.probes.saturating_add(MAX_LOCAL_PROBES));
        let candidate = self.parallel_alternatives(request).await;
        self.budget = budget;
        let selected = match candidate {
            Ok(Some(other))
                if displacement_area(request.head, request.target, &other.points) < displacement_area(request.head, request.target, &plan.points) =>
            {
                other
            }
            Err(Stop::Failed(error)) => return Err(error),
            _ => plan,
        };
        self.tidy_displacement(selected, request.flipped).await
    }

    /// Try parallel runs through individual guide-hull corners without merging a whole channel
    /// into one convex obstacle. Every candidate uses the same intent, exact checks and global
    /// budget as the first walk; dropping this continuation cancels it without authored changes.
    pub(in crate::board::route) async fn parallel_alternatives(&mut self, request: &Request) -> Result<Option<Plan>, Stop> {
        let Some((start, end, side)) = self.displacement_side else { return Ok(None) };
        let (dx, dy) = ((end.x_nm - start.x_nm) as i128, (end.y_nm - start.y_nm) as i128);
        let span2 = dx * dx + dy * dy;
        if span2 == 0 {
            return Ok(None);
        }
        self.path_membership = None;
        self.clear.clear();
        super::super::search::yield_unit().await;
        let region = Contour::capsule(start, end, (span2 as f64).sqrt() + self.half);
        let candidates = self.query.candidates(&region, &self.layers).map_err(Stop::Failed)?;
        let mut bands = BTreeMap::<i128, Vec<Band>>::new();
        let mut count = 0usize;
        let mut excess = false;
        for index in candidates {
            let object = &self.query.objects()[index];
            let exempt = self.displacement_contacts.as_ref().map_or_else(
                || self.own_islands.contains(&object.island) || (self.route_nets.len() == 1 && object.intended_nets == self.route_nets),
                |contacts| contacts.contains(&object.name),
            );
            if exempt {
                continue;
            }
            super::super::search::yield_unit().await;
            let clearance = self.query.membership_clearance(&self.route_nets, &object.intended_nets);
            let Some(hull) = Hull::grown(&object.contour.core, object.contour.radius + self.half + clearance as f64 + MARGIN_NM, self.posture) else {
                continue;
            };
            let mut local = BTreeMap::<i128, Band>::new();
            for point in hull.corners_from(start, true) {
                let (x, y) = ((point.x_nm - start.x_nm) as i128, (point.y_nm - start.y_nm) as i128);
                let (offset, tangent) = (side * (dx * y - dy * x), dx * x + dy * y);
                if offset < 0 || !(0..=span2).contains(&tangent) || !crate::wire::valid_point(&point) {
                    continue;
                }
                local
                    .entry(offset)
                    .and_modify(|band| {
                        if tangent < band.first.0 {
                            band.first = (tangent, point);
                        }
                        if tangent > band.last.0 {
                            band.last = (tangent, point);
                        }
                    })
                    .or_insert(Band { first: (tangent, point), last: (tangent, point) });
            }
            // Coalesce overlapping intervals only. Distant bodies at the same
            // offset must not force a plateau across the space between them.
            for (offset, mut band) in local {
                let group = bands.entry(offset).or_default();
                let mut i = 0;
                while i < group.len() {
                    if band.first.0 <= group[i].last.0 && group[i].first.0 <= band.last.0 {
                        let other = group.remove(i);
                        count -= 1;
                        if other.first.0 < band.first.0 {
                            band.first = other.first;
                        }
                        if other.last.0 > band.last.0 {
                            band.last = other.last;
                        }
                        i = 0;
                    } else {
                        i += 1;
                    }
                }
                group.push(band);
                group.sort_by_key(|b| (b.first.0, b.last.0));
                count += 1;
                // Stream the captured neighbors while keeping only the nearest
                // bounded guide set. Every body remains an exact obstacle.
                if count > MAX_BANDS {
                    excess = true;
                    if let Some(mut far) = bands.last_entry() {
                        far.get_mut().pop();
                        if far.get().is_empty() {
                            far.remove_entry();
                        }
                        count -= 1;
                    }
                }
            }
        }
        let mut paths = Vec::with_capacity(MAX_BANDS * 4);
        for band in bands.into_values().flatten() {
            for first_flip in [false, true] {
                for last_flip in [false, true] {
                    let mut path = elbow(request.head, band.first.1, self.posture, first_flip);
                    if path.last() != Some(&band.last.1) {
                        path.push(band.last.1);
                    }
                    path.extend(elbow(band.last.1, request.target, self.posture, last_flip).into_iter().skip(1));
                    let path = simplify(&path);
                    if path.len() >= 2 && path.windows(2).all(|p| self.posture.allows(p[0], p[1])) {
                        paths.push(path);
                    }
                }
            }
        }
        // Return to the original source line promptly when several elbows fit.
        // A long displaced plateau needlessly propagates into distant neighbors.
        paths.sort_by(|a, b| {
            displacement_area(start, end, a).total_cmp(&displacement_area(start, end, b)).then_with(|| length(a).total_cmp(&length(b)))
        });
        paths.dedup();
        for path in paths {
            if !self.legal(&path).await? {
                continue;
            }
            let confirmed = self.confirm(path).await?;
            if confirmed.reached && confirmed.points.last() == Some(&request.target) {
                return Ok(Some(self.plan(confirmed.points, Outcome::Reached, None)));
            }
        }
        if excess { Err(Stop::Exhausted) } else { Ok(None) }
    }
}

/// Rank elbow choices by their swept distance from the original source centerline.
/// The omitted common normalization factor does not affect ordering; exact probes, not
/// this geometric preference, decide whether a candidate is legal.
fn displacement_area(start: Point, end: Point, points: &[Point]) -> f64 {
    let cross = |p: Point| {
        ((end.x_nm as i128 - start.x_nm as i128) * (p.y_nm as i128 - start.y_nm as i128)
            - (end.y_nm as i128 - start.y_nm as i128) * (p.x_nm as i128 - start.x_nm as i128))
            .abs() as f64
    };
    points.windows(2).map(|p| (cross(p[0]) + cross(p[1])) * ((p[1].x_nm - p[0].x_nm) as f64).hypot((p[1].y_nm - p[0].y_nm) as f64)).sum()
}
