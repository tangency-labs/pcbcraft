//! Route postures in whole nanometres. An elbow joins two points with at most two legs; every
//! leg it produces runs horizontally, vertically or, for the 45° posture, exactly diagonally.
use crate::schematic::Point;
use serde::{Deserialize, Serialize};

/// How a route bends on its way to the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Posture {
    /// One straight leg and one exact 45° leg (the route tool's 45° mode).
    #[default]
    Diagonal,
    /// One horizontal and one vertical leg.
    Orthogonal,
}

impl Posture {
    /// The stable spellings accepted by command clients.
    pub const NAMES: [&'static str; 2] = ["diagonal", "orthogonal"];

    /// Whether the leg from `a` to `b` follows this posture. A zero-length leg does not.
    pub fn allows(self, a: Point, b: Point) -> bool {
        let (dx, dy) = (b.x_nm - a.x_nm, b.y_nm - a.y_nm);
        if dx == 0 && dy == 0 {
            return false;
        }
        dx == 0 || dy == 0 || (self == Self::Diagonal && dx.abs() == dy.abs())
    }
}

/// The corners of an elbow from `from` to `to`, both included. The 45° elbow runs straight first
/// and diagonally second (diagonal first when `flipped`); the 90° elbow runs horizontally first
/// (vertically first when `flipped`). A target on an axis or diagonal needs one leg, and a target
/// equal to `from` gives `[from]`. Coordinates are bounded board points, so no step overflows.
pub(crate) fn elbow(from: Point, to: Point, posture: Posture, flipped: bool) -> Vec<Point> {
    let (dx, dy) = (to.x_nm - from.x_nm, to.y_nm - from.y_nm);
    let corner = match posture {
        Posture::Orthogonal if flipped => Point { x_nm: from.x_nm, y_nm: to.y_nm },
        Posture::Orthogonal => Point { x_nm: to.x_nm, y_nm: from.y_nm },
        Posture::Diagonal => {
            let run = dx.abs().min(dy.abs());
            let (sx, sy) = (run * dx.signum(), run * dy.signum());
            if flipped { Point { x_nm: from.x_nm + sx, y_nm: from.y_nm + sy } } else { Point { x_nm: to.x_nm - sx, y_nm: to.y_nm - sy } }
        }
    };
    let mut points = vec![from];
    if corner != from && corner != to {
        points.push(corner);
    }
    if to != from {
        points.push(to);
    }
    points
}

/// Whole-nanometre steps along a posture leg, and the unit lattice step. A leg that follows no
/// posture (never produced by the solver itself) is measured by its longer axis.
pub(super) fn lattice(a: Point, b: Point) -> (i64, Point) {
    let (dx, dy) = (b.x_nm - a.x_nm, b.y_nm - a.y_nm);
    let steps = dx.abs().max(dy.abs());
    (steps, Point { x_nm: dx.signum(), y_nm: dy.signum() })
}

/// The point `k` of `steps` along the leg from `a` to `b`. Posture legs stay on their exact
/// lattice line; other legs round to the nearest nanometre.
pub(super) fn along(a: Point, b: Point, k: i64, steps: i64) -> Point {
    if steps <= 0 {
        return a;
    }
    let scale = |from: i64, to: i64| -> i64 {
        let span = i128::from(to - from) * i128::from(k);
        let half = i128::from(steps) / 2;
        let rounded = if span >= 0 { (span + half) / i128::from(steps) } else { (span - half) / i128::from(steps) };
        from + i64::try_from(rounded).unwrap_or(0)
    };
    Point { x_nm: scale(a.x_nm, b.x_nm), y_nm: scale(a.y_nm, b.y_nm) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    /// Both 45° senses reach the target with one straight and one exact diagonal leg in every
    /// quadrant; 90° elbows switch which axis comes first.
    #[test]
    fn elbows_keep_their_posture_in_every_quadrant() {
        for to in [p(5_000, 2_000), p(-4_000, 1_000), p(1_000, -4_000), p(-3_000, -7_000)] {
            for flipped in [false, true] {
                for posture in [Posture::Diagonal, Posture::Orthogonal] {
                    let points = elbow(p(0, 0), to, posture, flipped);
                    assert_eq!(points.len(), 3);
                    assert_eq!(points.last(), Some(&to));
                    assert!(points.windows(2).all(|leg| posture.allows(leg[0], leg[1])), "{points:?}");
                }
            }
        }
        assert_eq!(elbow(p(0, 0), p(5, 2), Posture::Diagonal, false)[1], p(3, 0));
        assert_eq!(elbow(p(0, 0), p(5, 2), Posture::Diagonal, true)[1], p(2, 2));
        assert_eq!(elbow(p(0, 0), p(3, 1), Posture::Orthogonal, true)[1], p(0, 1));
        assert_eq!(elbow(p(0, 0), p(3, 3), Posture::Diagonal, false).len(), 2);
        assert_eq!(elbow(p(0, 0), p(0, 0), Posture::Diagonal, false).len(), 1);
    }

    /// Lattice points stay on the exact leg, including a diagonal's odd step count.
    #[test]
    fn lattice_points_stay_on_their_leg() {
        let (a, b) = (p(0, 0), p(7, -7));
        let (steps, unit) = lattice(a, b);
        assert_eq!((steps, unit), (7, p(1, -1)));
        assert_eq!(along(a, b, 3, steps), p(3, -3));
        assert_eq!(along(a, b, 0, steps), a);
        assert_eq!(along(a, b, steps, steps), b);
    }
}
