//! Supported copper contours shared by connectivity, clearance and board-edge checks.
//! A convex integer core expanded by a floating radius preserves the existing nanometer
//! projection, including rotated pad rounding and half-nanometer track/via radii.
use crate::schematic::Point;

/// The pad's inner rectangle, segment or point, expanded by a round radius.
#[derive(Clone, Debug)]
pub struct Contour {
    pub(crate) core: [Point; 4],
    pub(crate) radius: f64,
}

impl Contour {
    /// Resolve a trusted pad through the same contour used for copper connectivity.
    pub(super) fn pad(center: Point, corners: [Point; 4], shape: super::definition::PadShape, corner_radius_nm: i64) -> Self {
        use super::definition::PadShape;
        if shape == PadShape::Rectangle {
            return Self { core: corners, radius: 0. };
        }
        let c = &corners;
        let (ux, uy) = ((c[1].x_nm - c[0].x_nm) as f64, (c[1].y_nm - c[0].y_nm) as f64);
        let (vx, vy) = ((c[3].x_nm - c[0].x_nm) as f64, (c[3].y_nm - c[0].y_nm) as f64);
        let (w, h) = (ux.hypot(uy), vx.hypot(vy));
        let radius = match shape {
            PadShape::Rectangle => 0.,
            PadShape::RoundedRectangle => (corner_radius_nm as f64).min(w.min(h) / 2.),
            PadShape::Circle | PadShape::Oval => w.min(h).round() / 2.,
        };
        let (x, y) = if shape == PadShape::RoundedRectangle {
            ((c[0].x_nm as f64 + c[2].x_nm as f64) / 2., (c[0].y_nm as f64 + c[2].y_nm as f64) / 2.)
        } else {
            (center.x_nm as f64, center.y_nm as f64)
        };
        let (iw, ih) = match shape {
            PadShape::Circle => (0., 0.),
            PadShape::Oval => ((w.round() / 2. - radius).max(0.), (h.round() / 2. - radius).max(0.)),
            _ => ((w / 2. - radius).max(0.), (h / 2. - radius).max(0.)),
        };
        let core = [(-iw, -ih), (iw, -ih), (iw, ih), (-iw, ih)].map(|(a, b)| Point {
            x_nm: (x + a * ux / w.max(1.) + b * vx / h.max(1.)).round() as i64,
            y_nm: (y + a * uy / w.max(1.) + b * vy / h.max(1.)).round() as i64,
        });
        Self { core, radius }
    }

    /// A round-ended segment in board nanometers. A zero-length skeleton is supported for
    /// preview endpoints and vias; width and coordinates obey the existing track limits.
    pub fn segment(start: Point, end: Point, width_nm: i64) -> Result<Self, crate::CommandError> {
        if !crate::wire::valid_point(&start)
            || !crate::wire::valid_point(&end)
            || !(super::track::MIN_WIDTH_NM..=super::track::MAX_WIDTH_NM).contains(&width_nm)
        {
            return Err(crate::CommandError::InvalidParams("bounded coordinates and a supported copper width are required".into()));
        }
        Ok(Self::capsule(start, end, width_nm as f64 / 2.))
    }

    /// Construct a trusted native segment or circle, preserving fractional radii.
    pub(crate) fn capsule(start: Point, end: Point, radius: f64) -> Self {
        Self { core: [start, end, end, start], radius }
    }

    /// Only straight, nondegenerate capsule cores describe track centerlines.
    pub(super) fn track_centerline(&self) -> Option<[Point; 2]> {
        let [a, b, c, d] = self.core;
        (a == d && b == c && a != b).then_some([a, b])
    }

    /// Interval where a track would be too close to this convex copper core.
    /// A line through the core meets it at edges or starts/ends inside it.
    pub(super) fn track_proximity(&self, leg: [Point; 2], expansion: f64) -> Option<(f64, f64)> {
        self.edges()
            .filter_map(|(a, b)| proximity_interval(leg, [a, b], self.radius + expansion))
            .chain(self.contains_core(leg[0]).then_some((0., 0.)))
            .chain(self.contains_core(leg[1]).then_some((1., 1.)))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// Copper separation in nanometers. Zero or negative means contact; containment returns
    /// zero, not a penetration depth. Equality with a required clearance passes. Distances use
    /// the existing floating predicates and rounded integer pad cores, without a new epsilon.
    pub fn gap(&self, other: &Self) -> f64 {
        if self.core.iter().any(|p| other.contains_core(*p)) || other.core.iter().any(|p| self.contains_core(*p)) {
            return 0.;
        }
        self.edges()
            .flat_map(|(a, b)| other.edges().map(move |(c, d)| segment_distance(a, b, c, d) - self.radius - other.radius))
            .fold(f64::INFINITY, f64::min)
    }

    /// Closed copper contact, including tangency and degenerate segment skeletons.
    pub fn touches(&self, other: &Self) -> bool {
        self.gap(other) <= 0.
    }

    /// Core edges, collapsing the repeated capsule skeleton to its one closed segment.
    /// Point and oval degeneracies retain at least one edge for distance evaluation.
    pub(crate) fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.core
            .iter()
            .copied()
            .zip(self.core.iter().copied().cycle().skip(1))
            .take(if self.core[0] == self.core[3] && self.core[1] == self.core[2] { 1 } else { 4 })
    }

    /// Check containment in a core with area. Line and point cores are handled by edge distance.
    pub(crate) fn contains_core(&self, p: Point) -> bool {
        let [a, b, _, d] = self.core;
        let area = (b.x_nm - a.x_nm) as i128 * (d.y_nm - a.y_nm) as i128 - (b.y_nm - a.y_nm) as i128 * (d.x_nm - a.x_nm) as i128;
        area != 0 && inside(p, &self.core)
    }

    /// Test a round-ended segment against the pad, with a zero-length segment for an endpoint or via.
    pub(crate) fn overlaps_segment(&self, start: Point, end: Point, radius: f64) -> bool {
        self.touches(&Self::capsule(start, end, radius))
    }

    /// Test whether an unexpanded board edge enters copper's interior. Exact tangency is
    /// distinct from penetration, including point/line cores and a zero rounding radius.
    pub(crate) fn penetrates_segment(&self, start: Point, end: Point) -> bool {
        if self.radius > 0. {
            return self.contains_core(start)
                || self.contains_core(end)
                || self.edges().any(|(a, b)| segment_distance(start, end, a, b) < self.radius);
        }
        self.segment_enters_core(start, end)
    }

    /// Clip a segment to the open convex core. Boundary-only contact has no interior interval;
    /// crossing through vertices still has one. Integer cross products retain the existing
    /// orientation test, with floating parameters only for the intersection interval.
    fn segment_enters_core(&self, start: Point, end: Point) -> bool {
        let cross = |a: Point, b: Point, p: Point| {
            (b.x_nm - a.x_nm) as i128 * (p.y_nm - a.y_nm) as i128 - (b.y_nm - a.y_nm) as i128 * (p.x_nm - a.x_nm) as i128
        };
        let orientation = cross(self.core[0], self.core[1], self.core[3]).signum();
        if orientation == 0 {
            return false;
        }
        let (mut enter, mut leave) = (0.0_f64, 1.0_f64);
        for (a, b) in self.edges() {
            let from = orientation * cross(a, b, start);
            let to = orientation * cross(a, b, end);
            if from <= 0 && to <= 0 {
                return false;
            }
            if from <= 0 {
                enter = enter.max(from as f64 / (from - to) as f64);
            } else if to <= 0 {
                leave = leave.min(from as f64 / (from - to) as f64);
            }
        }
        enter < leave
    }

    /// Test copper contact between two pads, including containment and rounded edges.
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.touches(other)
    }

    /// Bounds for pruning pad contact and clearance candidates before testing their contours.
    pub fn bounds(&self) -> [f64; 4] {
        let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in self.core {
            let (x, y) = (p.x_nm as f64, p.y_nm as f64);
            bounds = [bounds[0].min(x - self.radius), bounds[1].min(y - self.radius), bounds[2].max(x + self.radius), bounds[3].max(y + self.radius)];
        }
        bounds
    }
}

/// Point-in-convex-polygon by consistent cross-product sign; boundary points count as inside.
pub(super) fn inside(point: Point, corners: &[Point; 4]) -> bool {
    let mut sign = 0i8;
    for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let cross = (b.x_nm - a.x_nm) as i128 * (point.y_nm - a.y_nm) as i128 - (b.y_nm - a.y_nm) as i128 * (point.x_nm - a.x_nm) as i128;
        let side = cross.signum() as i8;
        if side != 0 {
            if sign != 0 && side != sign {
                return false;
            }
            sign = side;
        }
    }
    true
}

/// Shortest distance between two closed segments in nanometers; zero when they cross.
pub(super) fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let cross =
        |o: Point, p: Point, q: Point| (p.x_nm - o.x_nm) as i128 * (q.y_nm - o.y_nm) as i128 - (p.y_nm - o.y_nm) as i128 * (q.x_nm - o.x_nm) as i128;
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if d1.signum() * d2.signum() < 0 && d3.signum() * d4.signum() < 0 {
        return 0.;
    }
    [point_distance(a, c, d), point_distance(b, c, d), point_distance(c, a, b), point_distance(d, a, b)].into_iter().fold(f64::INFINITY, f64::min)
}

/// Distance from a point to a closed segment; a degenerate segment is a point.
pub(super) fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let (px, py, ax, ay, bx, by) = (p.x_nm as f64, p.y_nm as f64, a.x_nm as f64, a.y_nm as f64, b.x_nm as f64, b.y_nm as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { (((px - ax) * dx + (py - ay) * dy) / length).clamp(0., 1.) } else { 0. };
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Locate closest approach on the first closed segment. For a tied parallel
/// overlap, prefer the station towards the incoming leg's tip.
pub(super) fn closest_station([a, b]: [Point; 2], [c, d]: [Point; 2]) -> f64 {
    let (u, v, w) = (delta(a, b), delta(c, d), delta(a, c));
    let denominator = cross(u, v);
    if denominator != 0. {
        let (t, s) = (cross(w, v) / denominator, cross(w, u) / denominator);
        if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&s) {
            return t;
        }
    }
    let project = |p| (dot(delta(a, p), u) / dot(u, u)).clamp(0., 1.);
    [(point_distance(d, a, b), project(d)), (point_distance(c, a, b), project(c)), (point_distance(a, c, d), 0.), (point_distance(b, c, d), 1.)]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(0., |(_, station)| station)
}

/// The parameter interval where a straight leg enters an expanded capsule.
/// End discs and the central strip cover the same capsule as the exact gap check.
pub(super) fn proximity_interval([a, b]: [Point; 2], [c, d]: [Point; 2], radius: f64) -> Option<(f64, f64)> {
    let (u, v, w) = (delta(a, b), delta(c, d), delta(c, a));
    let interval = |offset: f64, slope: f64, low: f64, high: f64| {
        if slope == 0. {
            return (low <= offset && offset <= high).then_some((0., 1.));
        }
        let (x, y) = ((low - offset) / slope, (high - offset) / slope);
        let (start, end) = (x.min(y).max(0.), x.max(y).min(1.));
        (start <= end).then_some((start, end))
    };
    let disc = |at| {
        let w = delta(at, a);
        let length2 = dot(u, u);
        let depth = radius * radius - cross(u, w).powi(2) / length2;
        (depth >= 0.).then(|| depth.sqrt() * length2.sqrt()).and_then(|r| interval(dot(w, u), length2, -r, r))
    };
    let strip = || {
        let length2 = dot(v, v);
        if length2 == 0. {
            return None;
        }
        let height = radius * length2.sqrt();
        let (x0, x1) = interval(dot(w, v), dot(u, v), 0., length2)?;
        let (y0, y1) = interval(cross(v, w), cross(v, u), -height, height)?;
        let (start, end) = (x0.max(y0), x1.min(y1));
        (start <= end).then_some((start, end))
    };
    [disc(c), disc(d), strip()].into_iter().flatten().reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

fn delta(a: Point, b: Point) -> (f64, f64) {
    ((b.x_nm - a.x_nm) as f64, (b.y_nm - a.y_nm) as f64)
}
fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn dot(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
