//! Bounded parallel displacement alternatives when a local hull walk cannot find a channel.
use super::*;
use std::collections::BTreeMap;

const MAX_GUIDE_OBJECTS: usize = 32;
const MAX_BANDS: usize = 64;

/// A parallel run bounded by its first and last source-tangent projections.
struct Band {
    first: (i128, Point),
    last: (i128, Point),
}

impl Walker<'_> {
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
        let mut bands = BTreeMap::<i128, Band>::new();
        let mut guides = 0;
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
            let mut counted = false;
            for point in hull.corners_from(start, true) {
                let (x, y) = ((point.x_nm - start.x_nm) as i128, (point.y_nm - start.y_nm) as i128);
                let (offset, tangent) = (side * (dx * y - dy * x), dx * x + dy * y);
                if offset < 0 || !(0..=span2).contains(&tangent) || !crate::wire::valid_point(&point) {
                    continue;
                }
                // Only bodies contributing an eligible forward portal consume the
                // guide quota. Every body remains in the exact collision/path checks.
                if !counted {
                    guides += 1;
                    if guides > MAX_GUIDE_OBJECTS {
                        return Err(Stop::Exhausted);
                    }
                    counted = true;
                }
                bands
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
        }
        let excess = bands.len() > MAX_BANDS;
        for band in bands.into_values().take(MAX_BANDS) {
            for first_flip in [false, true] {
                for last_flip in [false, true] {
                    let mut path = elbow(request.head, band.first.1, self.posture, first_flip);
                    if path.last() != Some(&band.last.1) {
                        path.push(band.last.1);
                    }
                    path.extend(elbow(band.last.1, request.target, self.posture, last_flip).into_iter().skip(1));
                    let path = simplify(&path);
                    if path.len() < 2 || !path.windows(2).all(|p| self.posture.allows(p[0], p[1])) || !self.legal(&path).await? {
                        continue;
                    }
                    let confirmed = self.confirm(path).await?;
                    if confirmed.reached && confirmed.points.last() == Some(&request.target) {
                        return Ok(Some(self.plan(confirmed.points, Outcome::Reached, None)));
                    }
                }
            }
        }
        if excess { Err(Stop::Exhausted) } else { Ok(None) }
    }
}
