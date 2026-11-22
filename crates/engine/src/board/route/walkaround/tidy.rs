//! Optional local hull shortcuts for certified displaced paths with long return bends.
use super::*;

/// At most two local return windows and 128 charged probes, within the gesture's budget.
const MAX_WINDOWS: usize = 2;
const MAX_PROBES: usize = 128;

impl Walker<'_> {
    /// Improve a reached source interval without discarding its certified fallback. Local
    /// walks retain its membership and forward half-plane; only a shorter complete path with
    /// lower length/backtracking cost can replace it. Optimization exhaustion keeps the
    /// certified path; query errors propagate to the owner.
    pub(in crate::board::route) async fn tidy_displacement(&mut self, mut plan: Plan, flipped: bool) -> Result<Plan, CommandError> {
        if plan.outcome != Outcome::Reached || self.displacement_side.is_none() || plan.points.len() < 3 {
            return Ok(plan);
        }
        let axis = ((self.target.x_nm - self.head.x_nm) as f64, (self.target.y_nm - self.head.y_nm) as f64);
        let span = axis.0.hypot(axis.1);
        if span == 0. {
            return Ok(plan);
        }
        let axis = (axis.0 / span, axis.1 / span);
        let budget = self.budget;
        self.budget = budget.min(self.probes.saturating_add(MAX_PROBES));
        let result = self.tidy_windows(&mut plan, axis, flipped).await;
        self.budget = budget;
        match result {
            Ok(()) | Err(Stop::Exhausted) => {
                plan.probes = self.probes;
                Ok(plan)
            }
            Err(Stop::Failed(error)) => Err(error),
        }
    }

    /// Try the largest projected retreats first. Each local search omits this cleanup, so
    /// windows cannot recurse; both the window count and exact probes have explicit bounds.
    async fn tidy_windows(&mut self, plan: &mut Plan, axis: (f64, f64), flipped: bool) -> Result<(), Stop> {
        let mut tried = BTreeSet::new();
        for _ in 0..MAX_WINDOWS {
            let window = plan
                .points
                .windows(2)
                .enumerate()
                .filter_map(|(index, leg)| {
                    let retreat = retreat(leg, axis);
                    let (i, j) = (index.saturating_sub(2), (index + 2).min(plan.points.len() - 1));
                    let (a, b) = (plan.points[i], plan.points[j]);
                    let key = (a.x_nm, a.y_nm, b.x_nm, b.y_nm);
                    (retreat > self.half * 2. && j > i + 1 && !tried.contains(&key)).then_some((retreat, i, j, key))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
            let Some((_, i, j, key)) = window else { break };
            tried.insert(key);
            self.path_membership = None;
            self.clear.clear();
            let local = self.walk(plan.points[i], plan.points[j], flipped).await?;
            if !local.reached || local.points.last() != Some(&plan.points[j]) {
                continue;
            }
            let mut points = plan.points[..i].to_vec();
            points.extend(local.points);
            points.extend_from_slice(&plan.points[j + 1..]);
            let points = simplify(&points);
            if points.len() > MAX_PATH_POINTS
                || length(&points) + MIN_REACH_NM as f64 >= length(&plan.points)
                || cost(&points, axis) + MIN_REACH_NM as f64 >= cost(&plan.points, axis)
            {
                continue;
            }
            let confirmed = self.confirm(points).await?;
            if confirmed.reached && confirmed.points.first() == Some(&self.head) && confirmed.points.last() == Some(&self.target) {
                plan.points = confirmed.points;
            }
        }
        Ok(())
    }
}

/// Distance a leg travels backward along the original source interval's unit tangent.
fn retreat(leg: &[Point], axis: (f64, f64)) -> f64 {
    (-((leg[1].x_nm - leg[0].x_nm) as f64 * axis.0 + (leg[1].y_nm - leg[0].y_nm) as f64 * axis.1)).max(0.)
}

/// Prefer short paths while charging twice for travel back toward a source endpoint.
fn cost(points: &[Point], axis: (f64, f64)) -> f64 {
    length(points) + 2. * points.windows(2).map(|leg| retreat(leg, axis)).sum::<f64>()
}
