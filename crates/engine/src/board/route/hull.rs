//! Obstacle hulls with whole-nanometre corners and edges in fixed directions: every 45° for the
//! 45° posture, axes only for 90°. A hull is the intersection of half-planes `d·p <= s(d)` over
//! integer direction vectors `d`, where `s` supports an obstacle's copper core grown by its
//! radius, the route's half-width and the pair clearance, rounded outward. A centre line outside
//! the hull therefore clears that obstacle, and one running along its edges hugs it in the
//! posture's directions. Hulls only guide the walk: every kept leg is confirmed by the exact
//! clearance query.
use super::posture::Posture;
use crate::schematic::Point;

/// Integer direction vectors at k·45°, counter-clockwise from +x. Diagonals are unnormalized,
/// so their supports are `√2` times the Euclidean distance along the diagonal.
const DIRS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// Dot product of a direction with a point, widened so bounded coordinates never overflow.
fn dot(k: usize, p: Point) -> i128 {
    let (dx, dy) = DIRS[k];
    i128::from(dx) * i128::from(p.x_nm) + i128::from(dy) * i128::from(p.y_nm)
}

/// A convex polygon with whole-nanometre corners and edges in its posture's directions.
#[derive(Clone, Debug)]
pub(super) struct Hull {
    support: [i128; 8],
    posture: Posture,
    /// Counter-clockwise corners without consecutive duplicates.
    corners: Vec<Point>,
}

impl Hull {
    /// Grow a convex core (a copper contour's corners) by `radius` nanometres, rounding every
    /// support outward. Returns `None` for an empty core or non-finite radius.
    pub(super) fn grown(core: &[Point], radius: f64, posture: Posture) -> Option<Self> {
        if core.is_empty() || !radius.is_finite() || radius < 0. {
            return None;
        }
        let mut support = [0i128; 8];
        for (k, value) in support.iter_mut().enumerate() {
            let length = if k.is_multiple_of(2) { 1. } else { std::f64::consts::SQRT_2 };
            let reach = (radius * length).ceil() as i128;
            *value = core.iter().map(|p| dot(k, *p)).max()? + reach;
        }
        Self::from_support(support, posture)
    }

    /// Normalize diagonal supports to even values (an outward move of at most one unit), so
    /// every corner, including one where two diagonals meet, is a whole-nanometre point.
    fn from_support(mut support: [i128; 8], posture: Posture) -> Option<Self> {
        for k in [1, 3, 5, 7] {
            support[k] += support[k].rem_euclid(2);
        }
        let mut hull = Self { support, posture, corners: Vec::new() };
        hull.rebuild();
        (hull.corners.len() >= 3).then_some(hull)
    }

    /// Whether the hull uses direction `k`: diagonals only for the 45° posture.
    fn uses(&self, k: usize) -> bool {
        k.is_multiple_of(2) || self.posture == Posture::Diagonal
    }

    /// Recompute the corners: the axis box, clipped by each used diagonal half-plane.
    fn rebuild(&mut self) {
        let s = self.support;
        let point = |x: i128, y: i128| Point { x_nm: clamp(x), y_nm: clamp(y) };
        let mut polygon = vec![point(-s[4], -s[6]), point(s[0], -s[6]), point(s[0], s[2]), point(-s[4], s[2])];
        for k in [1, 3, 5, 7] {
            if self.uses(k) {
                polygon = clip(&polygon, k, s[k]);
            }
        }
        polygon.dedup();
        while polygon.len() > 1 && polygon.first() == polygon.last() {
            polygon.pop();
        }
        self.corners = polygon;
    }

    /// The union's hull: the larger support in every direction.
    pub(super) fn merge(&mut self, other: &Hull) {
        for (mine, theirs) in self.support.iter_mut().zip(other.support) {
            *mine = (*mine).max(theirs);
        }
        self.rebuild();
    }

    /// Move the boundary out by one unit across direction `k`. Used when a diagonal leg meets a
    /// perpendicular diagonal edge halfway between lattice points, so the meeting is exact.
    pub(super) fn widen(&mut self, k: usize) {
        if let Some(value) = self.support.get_mut(k) {
            *value += 1;
            self.rebuild();
        }
    }

    /// Pull the hull back to `p` across the edge nearest it, so a point legal against the exact
    /// copper but inside a corner's outward rounding lies on the boundary.
    pub(super) fn exclude(&mut self, p: Point) {
        let nearest = (0..8).filter(|k| self.uses(*k)).max_by_key(|k| (dot(*k, p) - self.support[*k], std::cmp::Reverse(*k)));
        if let Some(k) = nearest {
            self.support[k] = dot(k, p);
            self.rebuild();
        }
    }

    /// Counter-clockwise corners.
    #[cfg(test)]
    pub(super) fn corners(&self) -> &[Point] {
        &self.corners
    }

    /// The parameter interval of the leg `a`–`b` strictly inside the hull, and the direction
    /// whose edge it enters through (absent when it starts inside). `None` when the leg stays
    /// out or only touches the boundary. Exact integer numerators keep the comparison stable.
    pub(super) fn inside(&self, a: Point, b: Point) -> Option<(f64, f64, Option<usize>)> {
        let (mut enter, mut leave, mut through) = (0.0f64, 1.0f64, None);
        for k in (0..8).filter(|k| self.uses(*k)) {
            let start = dot(k, a) - self.support[k];
            let rate = dot(k, b) - dot(k, a);
            if rate == 0 {
                if start >= 0 {
                    return None;
                }
                continue;
            }
            let t = -(start as f64) / rate as f64;
            if rate > 0 {
                leave = leave.min(t);
            } else if t > enter || (t == enter && start >= 0 && through.is_none()) {
                enter = t;
                through = (start >= 0).then_some(k);
            }
            if enter >= leave {
                return None;
            }
        }
        (leave > enter).then_some((enter, leave, through))
    }

    /// The lattice step `m` at which `a + m·unit` meets the edge in direction `k`, when whole.
    pub(super) fn boundary_step(&self, k: usize, a: Point, unit: Point) -> Option<i64> {
        let start = dot(k, a) - *self.support.get(k)?;
        let rate = dot(k, unit);
        if rate == 0 || start % rate != 0 {
            return None;
        }
        i64::try_from(-start / rate).ok()
    }

    /// Whether any leg of the polyline enters the hull's interior.
    pub(super) fn crossed_by(&self, points: &[Point]) -> bool {
        points.windows(2).any(|leg| self.inside(leg[0], leg[1]).is_some())
    }

    /// Whether `p` lies on the closed edge from corner `i` to the next corner.
    fn on_edge(&self, i: usize, p: Point) -> bool {
        let n = self.corners.len();
        let (a, b) = (self.corners[i % n], self.corners[(i + 1) % n]);
        let cross = i128::from(b.x_nm - a.x_nm) * i128::from(p.y_nm - a.y_nm) - i128::from(b.y_nm - a.y_nm) * i128::from(p.x_nm - a.x_nm);
        cross == 0 && p.x_nm >= a.x_nm.min(b.x_nm) && p.x_nm <= a.x_nm.max(b.x_nm) && p.y_nm >= a.y_nm.min(b.y_nm) && p.y_nm <= a.y_nm.max(b.y_nm)
    }

    /// The corners met walking the boundary from `from` (on it), counter-clockwise when `ccw`,
    /// at most one full turn. A point off the boundary starts at the nearest corner's edge.
    pub(super) fn corners_from(&self, from: Point, ccw: bool) -> Vec<Point> {
        let n = self.corners.len();
        if n == 0 {
            return Vec::new();
        }
        let edge = (0..n).find(|i| self.on_edge(*i, from)).unwrap_or_else(|| self.nearest_corner(from));
        (0..n).map(|step| if ccw { self.corners[(edge + 1 + step) % n] } else { self.corners[(edge + n - step) % n] }).collect()
    }

    /// The corner nearest `p`, with the lowest index on ties.
    fn nearest_corner(&self, p: Point) -> usize {
        let distance = |c: &Point| {
            let (dx, dy) = (i128::from(c.x_nm - p.x_nm), i128::from(c.y_nm - p.y_nm));
            dx * dx + dy * dy
        };
        self.corners.iter().enumerate().min_by_key(|(i, c)| (distance(c), *i)).map_or(0, |(i, _)| i)
    }
}

/// Saturate a support-derived coordinate to the representable range; bounded inputs never reach it.
fn clamp(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// Divide rounding to the nearest integer, halves upward; `denominator` is never zero here.
fn div_round(numerator: i128, denominator: i128) -> i128 {
    let (n, d) = if denominator < 0 { (-numerator, -denominator) } else { (numerator, denominator) };
    if d == 0 {
        return 0;
    }
    (2 * n + d).div_euclid(2 * d)
}

/// Sutherland–Hodgman: the part of convex `polygon` with `d_k·p <= s`. Intersections of axis and
/// diagonal lines with even diagonal supports are whole points; any other rounds to the nearest.
fn clip(polygon: &[Point], k: usize, s: i128) -> Vec<Point> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (fa, fb) = (dot(k, a) - s, dot(k, b) - s);
        if fa <= 0 {
            out.push(a);
        }
        if (fa < 0 && fb > 0) || (fa > 0 && fb < 0) {
            let at = |from: i64, to: i64| clamp(i128::from(from) + div_round(i128::from(to - from) * fa, fa - fb));
            out.push(Point { x_nm: at(a.x_nm, b.x_nm), y_nm: at(a.y_nm, b.y_nm) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    /// A grown point is an octagon (or a square for 90°) with whole corners and posture edges;
    /// points nearer than the growth are inside and points beyond its outer bound are not.
    #[test]
    fn grown_hulls_contain_the_grown_core_with_posture_edges() {
        let hull = Hull::grown(&[p(0, 0)], 1_000., Posture::Diagonal).unwrap();
        assert_eq!(hull.corners().len(), 8);
        let n = hull.corners().len();
        for i in 0..n {
            assert!(Posture::Diagonal.allows(hull.corners()[i], hull.corners()[(i + 1) % n]), "{:?}", hull.corners());
        }
        for step in 0..64 {
            let angle = f64::from(step) / 64. * std::f64::consts::TAU;
            let inner = p((999. * angle.cos()) as i64, (999. * angle.sin()) as i64);
            assert!(hull.inside(p(0, 0), inner).is_some());
        }
        assert!(hull.inside(p(-3_000, 500), p(3_000, 500)).is_some());
        assert!(hull.inside(p(-3_000, 1_001), p(3_000, 1_001)).is_none());
        let square = Hull::grown(&[p(0, 0), p(4_000, 0), p(4_000, 0), p(0, 0)], 1_000., Posture::Orthogonal).unwrap();
        assert_eq!(square.corners(), &[p(-1_000, -1_000), p(5_000, -1_000), p(5_000, 1_000), p(-1_000, 1_000)]);
    }

    /// Entry reports the crossed edge and the boundary walk visits corners in either sense.
    #[test]
    fn entries_and_boundary_walks_are_exact() {
        let square = Hull::grown(&[p(0, 0)], 1_000., Posture::Orthogonal).unwrap();
        let (enter, _, through) = square.inside(p(-3_000, 0), p(3_000, 0)).unwrap();
        assert_eq!((enter, through), (2_000. / 6_000., Some(4)));
        assert_eq!(square.corners_from(p(0, -1_000), true)[..2], [p(1_000, -1_000), p(1_000, 1_000)]);
        assert_eq!(square.corners_from(p(0, -1_000), false)[..2], [p(-1_000, -1_000), p(-1_000, 1_000)]);
        let mut grown = square.clone();
        grown.merge(&Hull::grown(&[p(5_000, 0)], 1_000., Posture::Orthogonal).unwrap());
        assert_eq!(grown.corners()[1], p(6_000, -1_000));
        let mut pulled = square.clone();
        pulled.exclude(p(900, 0));
        assert!(pulled.inside(p(900, 0), p(3_000, 0)).is_none());
    }
}
