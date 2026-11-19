//! Small exact visibility search for concave pockets the local hull walk cannot resolve.
use super::*;

const MAX_OBJECTS: usize = 32;
const MAX_PORTALS: usize = 32;

impl Walker<'_> {
    /// Search bounded hull-corner portals, using posture elbows as exact-checked edges.
    /// Only a complete, connected-path-confirmed result is returned. The original pickup
    /// remains authoritative throughout; cancellation and the shared probe budget apply.
    pub(super) async fn portal_target(&mut self, request: &Request) -> Result<Option<Plan>, Stop> {
        let span = ((request.target.x_nm - request.head.x_nm) as f64).hypot((request.target.y_nm - request.head.y_nm) as f64);
        let region = Contour::capsule(request.head, request.target, span + self.half);
        let nearby = self.query.candidates(&region, &self.layers).map_err(Stop::Failed)?;
        let objects = self.query.objects();
        let exempt = |index: usize| {
            let object = &objects[index];
            self.own_islands.contains(&object.island) || (self.route_nets.len() == 1 && object.intended_nets == self.route_nets)
        };
        let mut guides = nearby.into_iter().filter(|i| !exempt(*i)).collect::<BTreeSet<_>>();
        if guides.len() > MAX_OBJECTS {
            return Ok(None);
        }
        // Include one ring of neighbors along each blocking body's full extent. A nearby
        // long trace can terminate in a pad outside the head/goal region; its pad corners
        // must guide the detour too. This is one bounded expansion, never a board-wide flood.
        for index in guides.iter().copied().collect::<Vec<_>>() {
            super::super::search::yield_unit().await;
            for neighbor in self.query.candidates(&objects[index].contour, &self.layers).map_err(Stop::Failed)? {
                if !exempt(neighbor) {
                    guides.insert(neighbor);
                    if guides.len() > MAX_OBJECTS {
                        return Ok(None);
                    }
                }
            }
        }
        let mut portals = Vec::new();
        for index in guides {
            super::super::search::yield_unit().await;
            let object = &objects[index];
            if self.own_islands.contains(&object.island) || (self.route_nets.len() == 1 && object.intended_nets == self.route_nets) {
                continue;
            }
            let clearance = self.query.membership_clearance(&self.route_nets, &object.intended_nets);
            if let Some(hull) = self.hull(index, clearance) {
                portals.extend(hull.corners_from(request.head, true).into_iter().filter(crate::wire::valid_point));
            }
        }
        portals.sort_by(|a, b| {
            let cost = |p| segment_length(request.head, p) + segment_length(p, request.target);
            cost(*a).total_cmp(&cost(*b)).then((a.x_nm, a.y_nm).cmp(&(b.x_nm, b.y_nm)))
        });
        portals.dedup();
        portals.truncate(MAX_PORTALS);
        portals.retain(|p| *p != request.head && *p != request.target);
        let mut points = vec![request.head];
        points.extend(portals);
        let mut costs = vec![f64::INFINITY; points.len()];
        let mut visited = vec![false; points.len()];
        let mut parents: Vec<Option<(usize, Vec<Point>)>> = vec![None; points.len()];
        costs[0] = 0.;
        // Each portal is expanded at most once. Every tested edge consumes the same
        // exact-probe budget as the hull walk, including edges to the requested endpoint.
        for _ in 0..points.len() {
            let Some(current) = (0..points.len()).filter(|i| !visited[*i] && costs[*i].is_finite()).min_by(|a, b| {
                let estimate = |i: usize| costs[i] + segment_length(points[i], request.target);
                estimate(*a).total_cmp(&estimate(*b)).then(a.cmp(b))
            }) else {
                break;
            };
            visited[current] = true;
            for flipped in [request.flipped, !request.flipped] {
                let tail = elbow(points[current], request.target, self.posture, flipped);
                if self.legal(&tail).await? {
                    let mut parts = vec![tail];
                    let mut cursor = current;
                    while let Some((parent, edge)) = &parents[cursor] {
                        parts.push(edge.clone());
                        cursor = *parent;
                    }
                    let mut path = vec![request.head];
                    for part in parts.into_iter().rev() {
                        path.extend(part.into_iter().skip(1));
                    }
                    let confirmed = self.confirm(simplify(&path)).await?;
                    if confirmed.reached && confirmed.points.last() == Some(&request.target) {
                        return Ok(Some(self.plan(confirmed.points, Outcome::Reached, None)));
                    }
                }
            }
            for next in 1..points.len() {
                if visited[next] || costs[current] + segment_length(points[current], points[next]) >= costs[next] {
                    continue;
                }
                for flipped in [request.flipped, !request.flipped] {
                    let edge = elbow(points[current], points[next], self.posture, flipped);
                    let cost = costs[current] + length(&edge);
                    if cost < costs[next] && self.legal(&edge).await? {
                        costs[next] = cost;
                        parents[next] = Some((current, edge));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Centre-line lower bound for deterministic geometric portal ranking.
fn segment_length(a: Point, b: Point) -> f64 {
    ((b.x_nm - a.x_nm) as f64).hypot((b.y_nm - a.y_nm) as f64)
}
