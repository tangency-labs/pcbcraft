//! Conservative bounds for pruning analytic profile operations.
use solidcore_record::{Loop2, Seg2, Vec2};

/// An axis-aligned envelope in the units of its source.
pub(super) type Bounds = [f64; 4];

/// Enclose all supplied positions, or the empty set when there are none.
pub(super) fn points(points: &[Vec2]) -> Bounds {
    points.iter().fold(empty(), |b, p| join(b, [p.x, p.y, p.x, p.y]))
}

pub(super) fn empty() -> Bounds {
    [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]
}

pub(super) fn join(a: Bounds, b: Bounds) -> Bounds {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Include arc endpoints and the axis extrema within the sweep. Positive-weight curves
/// stay in their control hull. Envelope queries use an outward numerical reserve.
pub(super) fn segment(seg: &Seg2) -> Bounds {
    match *seg {
        Seg2::Line { a, b } => points(&[a, b]),
        Seg2::Arc { center, radius, start, sweep } => {
            use std::f64::consts::{FRAC_PI_2, PI, TAU};
            let on_arc = |angle: f64| {
                let delta = ((angle - start) * sweep.signum()).rem_euclid(TAU);
                delta <= sweep.abs() + 1e-12 || delta >= TAU - 1e-12
            };
            let at = |angle: f64| center + Vec2::new(angle.cos() * radius, angle.sin() * radius);
            let mut bounds = points(&[at(start), at(start + sweep)]);
            for (angle, point) in [
                (0., Vec2::new(center.x + radius, center.y)),
                (FRAC_PI_2, Vec2::new(center.x, center.y + radius)),
                (PI, Vec2::new(center.x - radius, center.y)),
                (3. * FRAC_PI_2, Vec2::new(center.x, center.y - radius)),
            ] {
                if on_arc(angle) {
                    bounds = join(bounds, points(&[point]));
                }
            }
            bounds
        }
        Seg2::Cubic { p0, p1, p2, p3 } => points(&[p0, p1, p2, p3]),
        Seg2::Conic { a, apex, b, .. } => points(&[a, apex, b]),
        Seg2::Bezier { n, ref p } => points(p.get(..=n as usize).unwrap_or_default()),
    }
}

pub(super) fn loop_bounds(profile: &Loop2) -> Bounds {
    profile.segs.iter().map(segment).fold(empty(), join)
}

/// Inclusive overlap with an outward reserve so touching or nearly touching paths fall
/// back to SolidCore instead of being treated as disjoint.
pub(super) fn near(a: Bounds, b: Bounds, reserve: f64) -> bool {
    a[0] <= b[2] + reserve && a[2] + reserve >= b[0] && a[1] <= b[3] + reserve && a[3] + reserve >= b[1]
}
