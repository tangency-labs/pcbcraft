//! Shared exact circular arc geometry for copper contact, connectivity and short detection.
//!
//! All coordinates use board nanometers (`Point`, X right, Y down, bounded to ±1 m) and the
//! validation conventions in [`super::ArcTrack`]. No chord substitution or tessellation is used:
//! every contact predicate evaluates closed-form distances to the circular sweep, including
//! its projected end, and the authored round endpoints.
//!
//! ## Contact tolerance
//! At the ±1 m (±10⁹ nm) coordinate limit, IEEE-754 `f64` has ~2.2×10⁻⁷ nm per ULP.
//! [`CONTACT_TOLERANCE_NM`] (`1e-6` nm) absorbs floating-point rounding at exact tangencies and
//! intersections; gaps `<= 1e-6` nm count as contact (whereas the linear [`Contour::gap`]
//! predicate uses no epsilon).
use super::{ArcTrack, Direction};
use crate::{
    board::{
        geometry::Contour,
        track::{MAX_VIA_DIAMETER_NM, MAX_WIDTH_NM, MIN_VIA_DRILL_NM, MIN_WIDTH_NM},
    },
    schematic::Point,
};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2, TAU};

/// Documented floating-point tolerance for exact arc contact predicates, in board nanometers.
/// Gaps `<= 1e-6` nm count as contact; linear [`Contour::gap`] predicates use no epsilon.
pub const CONTACT_TOLERANCE_NM: f64 = 1e-6;

/// Validated circular arc centerline with round copper expansion in board nanometers
/// (X right, Y down). Always constructed through [`ArcGeometry::from_track`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcGeometry {
    center: (f64, f64),
    radius: f64,
    start: (f64, f64),
    end: (f64, f64),
    /// Point on the start circle in the direction of the authored end.
    circular_end: (f64, f64),
    start_point: Point,
    end_point: Point,
    start_angle: f64,
    /// Positive angular span in `(0, TAU)` traversed in `direction` from `start` to `end`.
    sweep: f64,
    direction: Direction,
    half_width: f64,
}

impl ArcGeometry {
    /// Derive exact circular geometry from a validated [`ArcTrack`].
    /// Refuses degenerate radii, full circles (equal ends), zero angular sweeps, out-of-range
    /// coordinates/widths, and endpoints more than 1 nm off the start circle.
    pub fn from_track(arc: &ArcTrack) -> Result<Self, String> {
        arc.validate()?;
        let center = (arc.center.x_nm as f64, arc.center.y_nm as f64);
        let start = (arc.start.x_nm as f64, arc.start.y_nm as f64);
        let end = (arc.end.x_nm as f64, arc.end.y_nm as f64);
        let radius = (start.0 - center.0).hypot(start.1 - center.1);
        if !(radius > 0.0 && radius.is_finite()) {
            return Err("an arc needs a nonzero finite radius".into());
        }
        let start_angle = (start.1 - center.1).atan2(start.0 - center.0);
        let (ex, ey) = (end.0 - center.0, end.1 - center.1);
        let end_angle = ey.atan2(ex);
        let scale = radius / ex.hypot(ey);
        let circular_end = (center.0 + ex * scale, center.1 + ey * scale);
        let mut sweep = match arc.direction {
            Direction::Clockwise => (end_angle - start_angle).rem_euclid(TAU),
            Direction::Counterclockwise => (start_angle - end_angle).rem_euclid(TAU),
        };
        // Near 0 or TAU, `atan2` subtraction can round to 0 or flip order across adjacent ULPs.
        // Decide tiny sweep vs near-full circle from exact integer cross and dot products.
        if sweep <= 1e-12 || sweep >= TAU - 1e-12 {
            let (sx, sy) = (arc.start.x_nm as i128 - arc.center.x_nm as i128, arc.start.y_nm as i128 - arc.center.y_nm as i128);
            let (ex, ey) = (arc.end.x_nm as i128 - arc.center.x_nm as i128, arc.end.y_nm as i128 - arc.center.y_nm as i128);
            let cross = sx * ey - sy * ex;
            let dot = sx * ex + sy * ey;
            if dot > 0 {
                let delta = (cross.unsigned_abs() as f64).atan2(dot as f64).max(f64::MIN_POSITIVE);
                let is_tiny = match arc.direction {
                    Direction::Clockwise => cross > 0,
                    Direction::Counterclockwise => cross < 0,
                };
                let below_tau = f64::from_bits(TAU.to_bits() - 1);
                sweep = if is_tiny { delta } else { (TAU - delta).min(below_tau) };
            }
        }
        Ok(Self {
            center,
            radius,
            start,
            end,
            circular_end,
            start_point: arc.start,
            end_point: arc.end,
            start_angle,
            sweep,
            direction: arc.direction,
            half_width: arc.width_nm as f64 / 2.0,
        })
    }

    /// Circle center `(x, y)` in board nanometers.
    pub fn center(&self) -> (f64, f64) {
        self.center
    }

    /// Radius derived from `center` and `start`, in board nanometers.
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Authored start point `(x, y)` in floating-point board nanometers.
    pub fn start(&self) -> (f64, f64) {
        self.start
    }

    /// Authored end point `(x, y)` in floating-point board nanometers.
    pub fn end(&self) -> (f64, f64) {
        self.end
    }

    /// Exact integer start point.
    pub fn start_point(&self) -> Point {
        self.start_point
    }

    /// Exact integer end point.
    pub fn end_point(&self) -> Point {
        self.end_point
    }

    /// Start angle in radians (`atan2(y - cy, x - cx)` in X-right, Y-down coordinates).
    pub fn start_angle(&self) -> f64 {
        self.start_angle
    }

    /// Positive angular span in `(0, TAU)` traversed in `self.direction()`.
    pub fn sweep(&self) -> f64 {
        self.sweep
    }

    /// Authored traversal direction in X-right, Y-down coordinates.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// Copper half-width (`width_nm / 2.0`) in board nanometers.
    pub fn half_width(&self) -> f64 {
        self.half_width
    }

    /// Whether `angle` (in radians, standard `atan2(y - cy, x - cx)` in X-right, Y-down) lies
    /// within the closed angular sweep `[0, sweep]` from `start_angle` in `self.direction`.
    pub fn contains_angle(&self, angle: f64) -> bool {
        let offset = match self.direction {
            Direction::Clockwise => (angle - self.start_angle).rem_euclid(TAU),
            Direction::Counterclockwise => (self.start_angle - angle).rem_euclid(TAU),
        };
        offset <= self.sweep
    }

    /// Point on the circular centerline at `angle` radians.
    pub fn point_at_angle(&self, angle: f64) -> (f64, f64) {
        (self.center.0 + self.radius * angle.cos(), self.center.1 + self.radius * angle.sin())
    }

    /// Representative midpoint along the circular centerline, rounded to the nearest nanometer.
    pub fn midpoint(&self) -> Point {
        let mid_angle = match self.direction {
            Direction::Clockwise => self.start_angle + self.sweep / 2.0,
            Direction::Counterclockwise => self.start_angle - self.sweep / 2.0,
        };
        let (x, y) = self.point_at_angle(mid_angle);
        Point { x_nm: x.round() as i64, y_nm: y.round() as i64 }
    }

    /// Conservative axis-aligned copper bounding box `[min_x, min_y, max_x, max_y]` in nm.
    pub fn bounds(&self) -> [f64; 4] {
        let mut min_x = self.start.0.min(self.end.0).min(self.circular_end.0);
        let mut min_y = self.start.1.min(self.end.1).min(self.circular_end.1);
        let mut max_x = self.start.0.max(self.end.0).max(self.circular_end.0);
        let mut max_y = self.start.1.max(self.end.1).max(self.circular_end.1);
        for angle in [0.0, FRAC_PI_2, PI, -FRAC_PI_2] {
            if self.contains_angle(angle) {
                let (x, y) = self.point_at_angle(angle);
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        let expand = self.half_width + 1.0;
        [min_x - expand, min_y - expand, max_x + expand, max_y + expand]
    }

    /// Conservative four-axis bounds `(min, max)` on `[x, y, x + y, x - y]` for broad-phase indexing.
    pub(crate) fn four_axis_bounds(&self) -> ([f64; 4], [f64; 4]) {
        let mut min = [f64::INFINITY; 4];
        let mut max = [f64::NEG_INFINITY; 4];
        let mut include = |(x, y): (f64, f64)| {
            let coords = [x, y, x + y, x - y];
            for ((lo, hi), value) in min.iter_mut().zip(max.iter_mut()).zip(coords) {
                *lo = lo.min(value);
                *hi = hi.max(value);
            }
        };
        include(self.start);
        include(self.end);
        include(self.circular_end);
        for step in 0..8 {
            let angle = step as f64 * FRAC_PI_4;
            if self.contains_angle(angle) {
                include(self.point_at_angle(angle));
            }
        }
        let xy = self.half_width + 1.0;
        let diag = self.half_width * SQRT_2 + 1.0;
        let pad = [xy, xy, diag, diag];
        (std::array::from_fn(|a| min[a] - pad[a]), std::array::from_fn(|a| max[a] + pad[a]))
    }

    /// Shortest Euclidean distance to the closed sweep or the authored end.
    pub fn distance_to_point(&self, point: (f64, f64)) -> f64 {
        let mut best = hypot_diff(point, self.start).min(hypot_diff(point, self.end)).min(hypot_diff(point, self.circular_end));
        let dx = point.0 - self.center.0;
        let dy = point.1 - self.center.1;
        let dist_c = dx.hypot(dy);
        if dist_c == 0.0 {
            best = best.min(self.radius);
        } else if self.contains_angle(dy.atan2(dx)) {
            best = best.min((dist_c - self.radius).abs());
        }
        best
    }

    /// Shortest Euclidean distance between a closed segment `(a, b)` and this arc's centerline.
    pub fn distance_to_segment(&self, a: Point, b: Point) -> f64 {
        self.distance_to_segment_f64((a.x_nm as f64, a.y_nm as f64), (b.x_nm as f64, b.y_nm as f64))
    }

    /// Closed-form centerline distance to a floating-point segment `(a, b)`.
    fn distance_to_segment_f64(&self, a: (f64, f64), b: (f64, f64)) -> f64 {
        let mut best = point_to_segment_distance(self.start, a, b)
            .min(point_to_segment_distance(self.end, a, b))
            .min(point_to_segment_distance(self.circular_end, a, b))
            .min(self.distance_to_point(a))
            .min(self.distance_to_point(b));
        let p0 = (a.0 - self.center.0, a.1 - self.center.1);
        let dir = (b.0 - a.0, b.1 - a.1);
        let len2 = dir.0 * dir.0 + dir.1 * dir.1;
        if len2 == 0.0 {
            return best;
        }
        let len = len2.sqrt();
        let t_closest = -(p0.0 * dir.0 + p0.1 * dir.1) / len2;
        let closest = (p0.0 + t_closest * dir.0, p0.1 + t_closest * dir.1);
        let h = closest.0.hypot(closest.1);

        // Interior circle-segment intersections.
        if h <= self.radius + CONTACT_TOLERANCE_NM {
            let half_chord = ((self.radius - h).max(0.0) * (self.radius + h)).sqrt() / len;
            for t in [t_closest - half_chord, t_closest + half_chord] {
                if (0.0..=1.0).contains(&t) {
                    let q = (p0.0 + t * dir.0, p0.1 + t * dir.1);
                    if self.contains_angle(q.1.atan2(q.0)) {
                        best = best.min((q.0.hypot(q.1) - self.radius).abs());
                    }
                }
            }
        }

        // Interior stationary point at the perpendicular foot from the circle center.
        if (0.0..=1.0).contains(&t_closest) {
            if h == 0.0 {
                best = best.min(self.radius);
            } else if self.contains_angle(closest.1.atan2(closest.0)) {
                best = best.min((h - self.radius).abs());
            }
        }
        best
    }

    /// Shortest Euclidean distance between two arc centerlines (including authored endpoints).
    pub fn distance_to_arc(&self, other: &Self) -> f64 {
        let mut best = self
            .distance_to_point(other.start)
            .min(self.distance_to_point(other.end))
            .min(self.distance_to_point(other.circular_end))
            .min(other.distance_to_point(self.start))
            .min(other.distance_to_point(self.end))
            .min(other.distance_to_point(self.circular_end));

        let dx = other.center.0 - self.center.0;
        let dy = other.center.1 - self.center.1;
        let dist_c = dx.hypot(dy);

        // Concentric or coincident circles.
        if dist_c <= CONTACT_TOLERANCE_NM {
            let shares_angle = self.contains_angle(other.start_angle)
                || self.contains_angle(other.end_angle())
                || other.contains_angle(self.start_angle)
                || other.contains_angle(self.end_angle());
            if shares_angle {
                best = best.min((self.radius - other.radius).abs());
            }
            return best;
        }

        // Circle-circle intersections.
        let r_diff = (self.radius - other.radius).abs();
        let r_sum = self.radius + other.radius;
        if dist_c >= r_diff - CONTACT_TOLERANCE_NM && dist_c <= r_sum + CONTACT_TOLERANCE_NM {
            let x = (dist_c + (self.radius - other.radius) * r_sum / dist_c) / 2.0;
            let y = ((self.radius - x).max(0.0) * (self.radius + x).max(0.0)).sqrt();
            let (ux, uy) = (dx / dist_c, dy / dist_c);
            for sign in [-1.0_f64, 1.0] {
                let p = (x * ux - sign * y * uy, x * uy + sign * y * ux);
                if self.contains_angle(p.1.atan2(p.0)) {
                    let q = (p.0 - dx, p.1 - dy);
                    if other.contains_angle(q.1.atan2(q.0)) {
                        best = best.min((q.0.hypot(q.1) - other.radius).abs());
                    }
                }
            }
        }

        // Non-intersecting interior stationary points lie on the line of centers.
        let theta_ab = dy.atan2(dx);
        let theta_ba = (-dy).atan2(-dx);
        for (ta, tb, dist) in [
            (theta_ab, theta_ba, (dist_c - r_sum).abs()),
            (theta_ab, theta_ab, (dist_c + other.radius - self.radius).abs()),
            (theta_ba, theta_ba, (dist_c + self.radius - other.radius).abs()),
        ] {
            if self.contains_angle(ta) && other.contains_angle(tb) {
                best = best.min(dist);
            }
        }
        best
    }

    /// Angle of the authored end point relative to `self.center`.
    fn end_angle(&self) -> f64 {
        (self.end.1 - self.center.1).atan2(self.end.0 - self.center.0)
    }

    /// Exact copper contact test: arc vs arc (including tangency, endpoint touch and coincidence).
    pub fn touches_arc(&self, other: &Self) -> bool {
        self.distance_to_arc(other) <= self.half_width + other.half_width + CONTACT_TOLERANCE_NM
    }

    /// Exact copper contact test: arc vs round-ended straight segment with floating radius.
    pub fn touches_segment_radius(&self, start: Point, end: Point, radius: f64) -> bool {
        self.distance_to_segment(start, end) <= self.half_width + radius + CONTACT_TOLERANCE_NM
    }

    /// Exact copper contact test: arc vs validated straight track segment `(start, end, width_nm)`.
    pub fn touches_segment(&self, start: Point, end: Point, width_nm: i64) -> Result<bool, String> {
        if !crate::wire::valid_point(&start) || !crate::wire::valid_point(&end) || !(MIN_WIDTH_NM..=MAX_WIDTH_NM).contains(&width_nm) {
            return Err("bounded coordinates and a supported copper width are required".into());
        }
        Ok(self.touches_segment_radius(start, end, width_nm as f64 / 2.0))
    }

    /// Exact copper contact test: arc vs circular pad or via with floating radius.
    pub fn touches_point_radius(&self, at: Point, radius: f64) -> bool {
        self.distance_to_point((at.x_nm as f64, at.y_nm as f64)) <= self.half_width + radius + CONTACT_TOLERANCE_NM
    }

    /// Exact copper contact test: arc vs validated via `(at, diameter_nm)`.
    pub fn touches_via(&self, at: Point, diameter_nm: i64) -> Result<bool, String> {
        if !crate::wire::valid_point(&at) || !(MIN_VIA_DRILL_NM..=MAX_VIA_DIAMETER_NM).contains(&diameter_nm) {
            return Err("bounded coordinates and a supported via diameter are required".into());
        }
        Ok(self.touches_point_radius(at, diameter_nm as f64 / 2.0))
    }

    /// Exact copper contact test: arc vs pad (or linear copper) [`Contour`].
    /// Handles rectangle, rounded-rectangle, circle and oval pads, including full containment.
    pub fn touches_contour(&self, contour: &Contour) -> bool {
        if contour.contains_core(self.start_point) || contour.contains_core(self.end_point) {
            return true;
        }
        let min_dist = contour.edges().map(|(a, b)| self.distance_to_segment(a, b)).fold(f64::INFINITY, f64::min);
        min_dist <= self.half_width + contour.radius + CONTACT_TOLERANCE_NM
    }
}

/// Euclidean distance between two floating-point points.
fn hypot_diff(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Shortest Euclidean distance from point `p` to closed segment `(a, b)`.
fn point_to_segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::definition::PadShape;
    use uuid::Uuid;

    fn pt(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    fn make_arc(center: Point, start: Point, end: Point, width_nm: i64, direction: Direction) -> ArcTrack {
        ArcTrack { id: Uuid::from_u128(1), layer: Uuid::from_u128(2), width_nm, center, start, end, direction }
    }

    fn q1_cw() -> ArcGeometry {
        // Center (0,0), radius 5_000_000, width 200_000 (half_width 100_000).
        // CW in X-right Y-down: (5_000_000, 0) -> (0, 5_000_000) (positive X, positive Y quadrant).
        ArcGeometry::from_track(&make_arc(pt(0, 0), pt(5_000_000, 0), pt(0, 5_000_000), 200_000, Direction::Clockwise)).unwrap()
    }

    /// Contacts just inside the projected cap survive a valid 1 nm endpoint discrepancy.
    #[test]
    fn off_circle_end_preserves_sweep_and_authored_cap_contacts() {
        let arc = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(5_000_000, 0), pt(0, 5_000_001), 200_001, Direction::Clockwise)).unwrap();
        let contact = pt(-120_000, 4_840_000);
        // The projected end is 200,000 nm away, the authored end about 200,000.8 nm.
        // The two copper radii sum to 200,000.5 nm.
        assert!(arc.touches_via(contact, 200_000).unwrap());
        assert!(arc.touches_segment(contact, pt(-220_000, 4_740_000), 200_000).unwrap());
        assert!(!arc.touches_via(pt(-120_002, 4_840_000), 200_000).unwrap());
        let other =
            ArcGeometry::from_track(&make_arc(pt(-120_000, 3_840_000), contact, pt(-1_120_000, 3_840_000), 200_000, Direction::Clockwise)).unwrap();
        assert!(arc.touches_arc(&other));
        assert!(other.touches_arc(&arc));

        assert_eq!(arc.end_point(), pt(0, 5_000_001));
        assert!(arc.touches_via(pt(-120_000, 5_160_001), 200_000).unwrap());
    }

    #[test]
    fn degenerate_radius_and_full_circle_limits() {
        // Zero radius (center == start or center == end) is refused.
        let zero_start = make_arc(pt(0, 0), pt(0, 0), pt(5_000_000, 0), 200_000, Direction::Clockwise);
        assert!(ArcGeometry::from_track(&zero_start).is_err());
        let zero_end = make_arc(pt(0, 0), pt(5_000_000, 0), pt(0, 0), 200_000, Direction::Clockwise);
        assert!(ArcGeometry::from_track(&zero_end).is_err());

        // Full circle (start == end) is refused.
        let full = make_arc(pt(0, 0), pt(5_000_000, 0), pt(5_000_000, 0), 200_000, Direction::Clockwise);
        assert!(ArcGeometry::from_track(&full).is_err());

        // Same-ray zero sweep is refused.
        let zero_sweep = make_arc(pt(0, 0), pt(5_000_000, 0), pt(5_000_001, 0), 200_000, Direction::Clockwise);
        assert!(ArcGeometry::from_track(&zero_sweep).is_err());

        // Near-full-circle limit: CW from (1_000_000, 0) to (1_000_000, -1) sweeps ~360 degrees.
        let near_full = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(1_000_000, 0), pt(1_000_000, -1), 20_000, Direction::Clockwise)).unwrap();
        assert!(near_full.sweep() > TAU - 1e-4 && near_full.sweep() < TAU);
        assert!(near_full.contains_angle(FRAC_PI_2));
        assert!(near_full.contains_angle(PI));
        assert!(near_full.contains_angle(-FRAC_PI_2));
        // Point on the opposite side of the circle (-1_000_000, 0) touches at exact outer tangency,
        // and misses by 1 nm outside.
        assert!(near_full.touches_point_radius(pt(-1_010_000, 0), 0.0));
        assert!(!near_full.touches_point_radius(pt(-1_010_001, 0), 0.0));

        // Complementary tiny CCW arc from (1_000_000, 0) to (1_000_000, -1) does NOT cover (-1_000_000, 0).
        let tiny_ccw =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(1_000_000, 0), pt(1_000_000, -1), 20_000, Direction::Counterclockwise)).unwrap();
        assert!(tiny_ccw.sweep() > 0.0 && tiny_ccw.sweep() < 1e-4);
        assert!(!tiny_ccw.contains_angle(PI));
        assert!(!tiny_ccw.touches_point_radius(pt(-1_000_000, 0), 0.0));

        // Sub-ULP atan2 collision: validate() accepts cross = -1 even when atan2 rounds equal.
        let tiny_sub_ulp =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(-100_000_000, 1), pt(-99_999_999, 1), 200_000, Direction::Counterclockwise)).unwrap();
        assert!(tiny_sub_ulp.sweep() > 0.0 && tiny_sub_ulp.sweep() < 1e-12);
        assert!(tiny_sub_ulp.touches_point_radius(pt(-100_000_000, 100_001), 0.0));
        assert!(!tiny_sub_ulp.touches_point_radius(pt(100_000_000, 0), 0.0));

        // Complementary CW direction on the same sub-ULP endpoints is a near-full ring.
        let full_sub_ulp =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(-100_000_000, 1), pt(-99_999_999, 1), 200_000, Direction::Clockwise)).unwrap();
        assert!(full_sub_ulp.sweep() > TAU - 1e-12 && full_sub_ulp.sweep() < TAU);
        assert!(full_sub_ulp.touches_point_radius(pt(100_000_000, 0), 0.0));

        // Radius strictly below half-width (e.g. R = 50_000 nm, half_width = 100_000 nm).
        let tight = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(50_000, 0), pt(0, 50_000), 200_000, Direction::Clockwise)).unwrap();
        assert!(tight.touches_point_radius(pt(0, 0), 0.0));
        assert!(tight.touches_point_radius(pt(150_000, 0), 0.0));
        assert!(!tight.touches_point_radius(pt(150_001, 0), 0.0));
    }

    #[test]
    fn arc_vs_arc_tangency_endpoint_near_miss_and_coincident() {
        let a = q1_cw(); // center (0,0), R=5_000_000, hw=100_000, Q1 (+x,+y)

        // External tangency along +Y at (0, 5_000_000):
        // Second arc centered at (0, 10_200_000), R=5_000_000, hw=100_000 -> center dist 10_200_000 = R1+R2+hw1+hw2.
        let ext_tangent = ArcGeometry::from_track(&make_arc(
            pt(0, 10_200_000),
            pt(5_000_000, 10_200_000),
            pt(-5_000_000, 10_200_000),
            200_000,
            Direction::Counterclockwise,
        ))
        .unwrap();
        assert!(a.touches_arc(&ext_tangent));
        assert!(ext_tangent.touches_arc(&a));

        // Near-miss by 1 nm at external tangency.
        let ext_miss = ArcGeometry::from_track(&make_arc(
            pt(0, 10_200_001),
            pt(5_000_000, 10_200_001),
            pt(-5_000_000, 10_200_001),
            200_000,
            Direction::Counterclockwise,
        ))
        .unwrap();
        assert!(!a.touches_arc(&ext_miss));
        assert!(!ext_miss.touches_arc(&a));

        // Interior tangency (not at endpoints): two arcs tangent along +X.
        // Arc A sweeps from -90 deg to +90 deg around (0,0), R=5_000_000, hw=100_000 -> covers (5_000_000, 0).
        let span_x = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(0, -5_000_000), pt(0, 5_000_000), 200_000, Direction::Clockwise)).unwrap();
        // Arc B centered at (10_200_000, 0), R=5_000_000, hw=100_000, covers (5_200_000, 0) in interior.
        let right_tangent = ArcGeometry::from_track(&make_arc(
            pt(10_200_000, 0),
            pt(10_200_000, 5_000_000),
            pt(10_200_000, -5_000_000),
            200_000,
            Direction::Clockwise,
        ))
        .unwrap();
        assert!(span_x.touches_arc(&right_tangent));
        let right_miss = ArcGeometry::from_track(&make_arc(
            pt(10_200_001, 0),
            pt(10_200_001, 5_000_000),
            pt(10_200_001, -5_000_000),
            200_000,
            Direction::Clockwise,
        ))
        .unwrap();
        assert!(!span_x.touches_arc(&right_miss));

        // Transversal mid-sweep crossing (y != 0) with all endpoints far apart:
        // Circle 1 centered at (0,0) R=5_000_000, Circle 2 centered at (6_000_000, 0) R=5_000_000
        // intersect at (3_000_000, ±4_000_000).
        let cross_b =
            ArcGeometry::from_track(&make_arc(pt(6_000_000, 0), pt(6_000_000, 5_000_000), pt(6_000_000, -5_000_000), 200_000, Direction::Clockwise))
                .unwrap();
        assert!(span_x.touches_arc(&cross_b));
        assert!(cross_b.touches_arc(&span_x));

        // Same circles, but Circle 2's arc sweeps only the right side (outside both intersections).
        let disjoint_circles =
            ArcGeometry::from_track(&make_arc(pt(6_000_000, 0), pt(6_000_000, -5_000_000), pt(6_000_000, 5_000_000), 200_000, Direction::Clockwise))
                .unwrap();
        assert!(!span_x.touches_arc(&disjoint_circles));

        // Internal concentric tangency: inner arc R=4_800_000, hw=100_000 inside R=5_000_000, hw=100_000 -> gap = 0.
        let inner_tangent = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(4_800_000, 0), pt(0, 4_800_000), 200_000, Direction::Clockwise)).unwrap();
        assert!(a.touches_arc(&inner_tangent));
        let inner_miss = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(4_799_999, 0), pt(0, 4_799_999), 200_000, Direction::Clockwise)).unwrap();
        assert!(!a.touches_arc(&inner_miss));

        // Endpoint touch: second arc starts at (0, 5_000_000) going into Q2 (-x, +y).
        let endpoint_touch =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(0, 5_000_000), pt(-5_000_000, 0), 200_000, Direction::Clockwise)).unwrap();
        assert!(a.touches_arc(&endpoint_touch));

        // Coincident arcs: identical, reversed direction, and disjoint on the same circle.
        assert!(a.touches_arc(&a));
        let reversed =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(0, 5_000_000), pt(5_000_000, 0), 200_000, Direction::Counterclockwise)).unwrap();
        assert!(a.touches_arc(&reversed));
        let opposite_quadrant =
            ArcGeometry::from_track(&make_arc(pt(0, 0), pt(-5_000_000, 0), pt(0, -5_000_000), 200_000, Direction::Clockwise)).unwrap();
        assert!(!a.touches_arc(&opposite_quadrant));

        // +/-1 m scale Pythagorean tangency (3-4-5 triangle scaled by 100_000_000 nm):
        // Center A = (-300_000_000, -400_000_000), Center B = (300_000_000, 400_000_000) -> dist = 1_000_000_000 nm.
        // R_A = R_B = 499_900_000 nm, hw_A = hw_B = 100_000 nm -> exact tangency at origin (0, 0).
        let large_a = ArcGeometry::from_track(&make_arc(
            pt(-300_000_000, -400_000_000),
            pt(-300_000_000 + 499_900_000, -400_000_000),
            pt(-300_000_000, -400_000_000 + 499_900_000),
            200_000,
            Direction::Clockwise,
        ))
        .unwrap();
        let large_b = ArcGeometry::from_track(&make_arc(
            pt(300_000_000, 400_000_000),
            pt(300_000_000 - 499_900_000, 400_000_000),
            pt(300_000_000, 400_000_000 - 499_900_000),
            200_000,
            Direction::Clockwise,
        ))
        .unwrap();
        assert!(large_a.touches_arc(&large_b));
        let large_b_miss = ArcGeometry::from_track(&make_arc(
            pt(300_000_001, 400_000_000),
            pt(300_000_001 - 499_900_000, 400_000_000),
            pt(300_000_001, 400_000_000 - 499_900_000),
            200_000,
            Direction::Clockwise,
        ))
        .unwrap();
        assert!(!large_a.touches_arc(&large_b_miss));
    }

    #[test]
    fn arc_vs_segment_tangency_endpoint_and_near_miss() {
        let a = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(0, -5_000_000), pt(0, 5_000_000), 200_000, Direction::Clockwise)).unwrap();
        // Vertical segment tangent to outer copper at x = 5_000_000 + 100_000 + 100_000 = 5_200_000.
        assert!(a.touches_segment(pt(5_200_000, -1_000_000), pt(5_200_000, 1_000_000), 200_000).unwrap());
        // Near-miss by 1 nm.
        assert!(!a.touches_segment(pt(5_200_001, -1_000_000), pt(5_200_001, 1_000_000), 200_000).unwrap());
        // Crossing segment through arc interior.
        assert!(a.touches_segment(pt(4_000_000, 0), pt(6_000_000, 0), 200_000).unwrap());
        // Endpoint touch at (0, 5_000_000).
        assert!(a.touches_segment(pt(0, 5_200_000), pt(0, 6_000_000), 200_000).unwrap());
        assert!(!a.touches_segment(pt(0, 5_200_001), pt(0, 6_000_000), 200_000).unwrap());
        // Segment on the uncovered left side (-5_000_000, 0) does NOT touch the right semicircle.
        assert!(!a.touches_segment(pt(-5_000_000, -1_000_000), pt(-5_000_000, 1_000_000), 200_000).unwrap());
    }

    #[test]
    fn arc_vs_via_tangency_endpoint_and_near_miss() {
        let a = q1_cw(); // R=5_000_000, hw=100_000, Q1 (+x,+y)
        // Via diameter 600_000 (radius 300_000): outer tangency at (5_400_000, 0) and (0, 5_400_000).
        assert!(a.touches_via(pt(5_400_000, 0), 600_000).unwrap());
        assert!(!a.touches_via(pt(5_400_001, 0), 600_000).unwrap());
        assert!(a.touches_via(pt(0, 5_400_000), 600_000).unwrap());
        assert!(!a.touches_via(pt(0, 5_400_001), 600_000).unwrap());
        // Inner tangency at (4_600_000, 0).
        assert!(a.touches_via(pt(4_600_000, 0), 600_000).unwrap());
        assert!(!a.touches_via(pt(4_599_999, 0), 600_000).unwrap());
        // Via on uncovered quadrant (-5_000_000, 0) does not touch.
        assert!(!a.touches_via(pt(-5_000_000, 0), 600_000).unwrap());
        // Odd width / diameter half-nanometer boundary: arc hw=100_000, via radius=300_000.5 -> touch at 5_400_000, miss at 5_400_001.
        assert!(a.touches_via(pt(5_400_000, 0), 600_001).unwrap());
        assert!(!a.touches_via(pt(5_400_001, 0), 600_001).unwrap());

        // Endpoint 1 nm off the start circle (the maximum validation tolerance in arc.rs):
        // contact at the authored end point uses the authored end position, missing by 1 nm beyond.
        let tol_arc = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(5_000_000, 0), pt(0, 5_000_001), 200_000, Direction::Clockwise)).unwrap();
        assert!(tol_arc.touches_point_radius(pt(0, 5_100_001), 0.0));
        assert!(!tol_arc.touches_point_radius(pt(0, 5_100_002), 0.0));
    }

    #[test]
    fn arc_vs_pad_contours_tangency_containment_and_near_miss() {
        let a = ArcGeometry::from_track(&make_arc(pt(0, 0), pt(0, -5_000_000), pt(0, 5_000_000), 200_000, Direction::Clockwise)).unwrap();
        let rect = |left: i64, top: i64, right: i64, bottom: i64| {
            Contour::pad(
                pt((left + right) / 2, (top + bottom) / 2),
                [pt(left, top), pt(right, top), pt(right, bottom), pt(left, bottom)],
                PadShape::Rectangle,
                0,
                (right - left, bottom - top),
            )
        };
        // Rectangle pad tangent to outer arc at x = 5_100_000.
        assert!(a.touches_contour(&rect(5_100_000, -500_000, 6_100_000, 500_000)));
        assert!(!a.touches_contour(&rect(5_100_001, -500_000, 6_100_001, 500_000)));
        // Arc completely contained inside a large rectangular pad.
        assert!(a.touches_contour(&rect(-10_000_000, -10_000_000, 10_000_000, 10_000_000)));

        // Circle pad tangent at x = 5_100_000 (center 5_600_000, diameter 1_000_000).
        let circle = |cx: i64| {
            Contour::pad(
                pt(cx, 0),
                [pt(cx - 500_000, -500_000), pt(cx + 500_000, -500_000), pt(cx + 500_000, 500_000), pt(cx - 500_000, 500_000)],
                PadShape::Circle,
                0,
                (1_000_000, 1_000_000),
            )
        };
        assert!(a.touches_contour(&circle(5_600_000)));
        assert!(!a.touches_contour(&circle(5_600_001)));

        // Rounded rectangle and oval pads.
        let rounded = |left: i64| {
            Contour::pad(
                pt(left + 500_000, 0),
                [pt(left, -500_000), pt(left + 1_000_000, -500_000), pt(left + 1_000_000, 500_000), pt(left, 500_000)],
                PadShape::RoundedRectangle,
                200_000,
                (1_000_000, 1_000_000),
            )
        };
        assert!(a.touches_contour(&rounded(5_100_000)));
        assert!(!a.touches_contour(&rounded(5_100_001)));

        let oval = |cx: i64| {
            Contour::pad(
                pt(cx, 0),
                [pt(cx - 500_000, -1_000_000), pt(cx + 500_000, -1_000_000), pt(cx + 500_000, 1_000_000), pt(cx - 500_000, 1_000_000)],
                PadShape::Oval,
                0,
                (1_000_000, 2_000_000),
            )
        };
        assert!(a.touches_contour(&oval(5_600_000)));
        assert!(!a.touches_contour(&oval(5_600_001)));
    }
}
