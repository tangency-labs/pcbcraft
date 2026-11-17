//! Boundary hatch geometry is original PCBcraft presentation, independent of copper fill.
use super::*;

/// A large rectangle leaves room to distinguish short edge marks from whole-area scan lines.
fn rectangle() -> Vec<Pos2> {
    vec![egui::pos2(0., 0.), egui::pos2(400., 0.), egui::pos2(400., 300.), egui::pos2(0., 300.)]
}

/// Diagonal stubs stay at real edges and leave a large central region completely unhatched.
#[test]
fn short_diagonals_leave_the_interior_clear() {
    let points = rectangle();
    let mut segments = Vec::new();
    zone_hatch_segments(&points, Rect::from_min_max(egui::pos2(-10., -10.), egui::pos2(410., 310.)), |s| segments.push(s));
    assert!(segments.len() > 20);
    for [a, b] in segments {
        let delta = b - a;
        assert!((delta.x.abs() - delta.y.abs()).abs() < 0.001);
        assert!(delta.x * delta.y <= 0.);
        assert!(delta.length() <= 36.001);
        let on_edge = |p: Pos2| p.x == 0. || p.y == 0. || p.x == 400. || p.y == 300.;
        assert!(on_edge(a) || on_edge(b));
        assert!(!Rect::from_min_max(egui::pos2(40., 40.), egui::pos2(360., 260.)).contains((a + b.to_vec2()) / 2.));
    }
    let mut count = 0;
    zone_hatch_segments(&points, Rect::from_min_max(egui::pos2(100., 100.), egui::pos2(300., 200.)), |_| count += 1);
    assert_eq!(count, 0, "viewport edges must not become authored hatch boundaries");
}

/// Concave notches and reversed winding keep every diagonal inside the authored polygon.
#[test]
fn concavity_winding_and_narrow_regions_clip_strokes() {
    let mut points = vec![
        egui::pos2(0., 0.),
        egui::pos2(240., 0.),
        egui::pos2(240., 240.),
        egui::pos2(160., 240.),
        egui::pos2(160., 80.),
        egui::pos2(80., 80.),
        egui::pos2(80., 240.),
        egui::pos2(0., 240.),
    ];
    for _ in 0..2 {
        let mut count = 0;
        zone_hatch_segments(&points, Rect::from_min_max(egui::pos2(-1., -1.), egui::pos2(241., 241.)), |[a, b]| {
            count += 1;
            for step in 0..=8 {
                let p = a + (b - a) * (step as f32 / 8.);
                assert!(p.x >= -0.001 && p.x <= 240.001 && p.y >= -0.001 && p.y <= 240.001);
                assert!(!(p.x > 80.001 && p.x < 159.999 && p.y > 80.001), "hatch crossed the notch at {p:?}");
            }
        });
        assert!(count > 20);
        points.reverse();
    }
    let narrow = [egui::pos2(0., 0.), egui::pos2(8., 0.), egui::pos2(8., 200.), egui::pos2(0., 200.)];
    zone_hatch_segments(&narrow, Rect::EVERYTHING, |[a, b]| {
        assert!(a.x >= 0. && b.x <= 8.);
        assert!(a.distance(b) <= 12.);
    });
}
