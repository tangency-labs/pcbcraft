use crate::Graphic;
use pcbcraft_geom::Point;
use std::f64::consts::TAU;

impl Graphic {
    /// Shared display approximation in local y-down coordinates. Invalid
    /// definitions produce an empty path. Curves never exceed 129 points;
    /// original polylines retain their validated (at most 10,000) vertices.
    /// Text is handled separately by the renderer.
    pub fn path_points(&self) -> Vec<Point> {
        if self.validate().is_err() {
            return Vec::new();
        }
        match self {
            Self::Line { start, end, .. } => vec![*start, *end],
            Self::Rectangle { start, end, .. } => vec![
                *start,
                Point::new(end.x, start.y),
                *end,
                Point::new(start.x, end.y),
                *start,
            ],
            Self::Polyline { points, .. } => points.clone(),
            Self::Circle { center, radius, .. } => {
                let mut points = (0..128)
                    .map(|i| {
                        let angle = TAU * (i as f64) / 128.0;
                        Point::new(
                            center.x + radius * angle.cos(),
                            center.y + radius * angle.sin(),
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
                points
            }
            Self::Arc {
                start, mid, end, ..
            } => arc_points(*start, *mid, *end),
            Self::Text { .. } => Vec::new(),
        }
    }
}

fn arc_points(start: Point, mid: Point, end: Point) -> Vec<Point> {
    // Translate to the start before the circumcircle calculation, to limit
    // cancellation for objects located far from the document origin.
    let b = Point::new(mid.x - start.x, mid.y - start.y);
    let c = Point::new(end.x - start.x, end.y - start.y);
    let cross = b.x * c.y - b.y * c.x;
    let scale = b.x.hypot(b.y) * c.x.hypot(c.y);
    if scale == 0.0 || cross.abs() <= scale * 1e-10 {
        return vec![start, mid, end];
    }
    let b2 = b.x * b.x + b.y * b.y;
    let c2 = c.x * c.x + c.y * c.y;
    let offset = Point::new(
        (b2 * c.y - c2 * b.y) / (2.0 * cross),
        (b.x * c2 - c.x * b2) / (2.0 * cross),
    );
    let center = start.offset(offset);
    let radius = offset.x.hypot(offset.y);
    if !radius.is_finite() || radius > 1e12 {
        return vec![start, mid, end];
    }
    let a = (start.y - center.y).atan2(start.x - center.x);
    let m = (mid.y - center.y).atan2(mid.x - center.x);
    let e = (end.y - center.y).atan2(end.x - center.x);
    let dm = (m - a).rem_euclid(TAU);
    let de = (e - a).rem_euclid(TAU);
    let (first, second) = if dm <= de {
        (dm, de - dm)
    } else {
        (dm - TAU, de - dm)
    };
    let mut points = Vec::with_capacity(129);
    points.push(start);
    // Two halves preserve the exact given midpoint, including major arcs.
    for (from, sweep, last) in [(a, first, mid), (m, second, end)] {
        for i in 1..64 {
            let angle = from + sweep * (i as f64) / 64.0;
            points.push(Point::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }
        points.push(last);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circle_has_exact_extents_and_closed_bounded_path() {
        let g = Graphic::Circle {
            center: Point::new(2.0, 3.0),
            radius: 4.0,
            width: 0.1,
            fill: "none".into(),
            layer: String::new(),
        };
        let points = g.path_points();
        assert_eq!(points.len(), 129);
        assert_eq!(points.first(), points.last());
        assert!(points.iter().any(|p| (p.x + 2.0).abs() < 1e-9));
        assert!(points.iter().any(|p| (p.y - 7.0).abs() < 1e-9));
    }
    #[test]
    fn semicircle_passes_through_mid_in_correct_direction() {
        for y in [-1.0, 1.0] {
            let mid = Point::new(0.0, y);
            let g = Graphic::Arc {
                start: Point::new(-1.0, 0.0),
                mid,
                end: Point::new(1.0, 0.0),
                width: 0.1,
                layer: String::new(),
            };
            let points = g.path_points();
            assert_eq!(points.len(), 129);
            assert_eq!(points[64], mid);
            assert!(points.iter().all(|p| p.y * y >= -1e-9));
            assert!(points.iter().all(|p| (p.x.hypot(p.y) - 1.0).abs() < 1e-9));
        }
    }
    #[test]
    fn degenerate_arc_and_invalid_geometry_are_safe() {
        let g = Graphic::Arc {
            start: Point::new(0.0, 0.0),
            mid: Point::new(1.0, 0.0),
            end: Point::new(2.0, 0.0),
            width: 0.1,
            layer: String::new(),
        };
        assert_eq!(g.path_points().len(), 3);
        let bad = Graphic::Circle {
            center: Point::default(),
            radius: f64::NAN,
            width: 0.1,
            fill: String::new(),
            layer: String::new(),
        };
        assert!(bad.path_points().is_empty());
    }

    #[test]
    fn major_arc_uses_the_long_sweep_through_midpoint() {
        let g = Graphic::Arc {
            start: Point::new(1.0, 0.0),
            mid: Point::new(-1.0, 0.0),
            end: Point::new(0.0, 1.0),
            width: 0.1,
            layer: String::new(),
        };
        let points = g.path_points();
        assert_eq!(points[64], Point::new(-1.0, 0.0));
        assert!(points.iter().any(|p| p.y < -0.9));
        assert_eq!(points.last(), Some(&Point::new(0.0, 1.0)));
    }
}
