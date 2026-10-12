//! Small, bounded planar geometry. Units are millimetres.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.x.abs() <= 100_000.0
            && self.y.abs() <= 100_000.0
    }
    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
    pub fn offset(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
    pub fn rotate(self, degrees: f64) -> Self {
        let (s, c) = degrees.to_radians().sin_cos();
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }
}

pub fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= 10_000.0
}

pub fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = dx * dx + dy * dy;
    if length < 1e-20 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

pub fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ab_c * ab_d < 0.0 && cd_a * cd_b < 0.0 {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crossing_and_collinear() {
        assert_eq!(
            segment_distance(
                Point::new(0.0, 0.0),
                Point::new(5.0, 5.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 0.0)
            ),
            0.0
        );
        assert_eq!(
            segment_distance(
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(3.0, 0.0),
                Point::new(4.0, 0.0)
            ),
            2.0
        );
    }
}
