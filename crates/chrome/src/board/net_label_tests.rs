use super::*;
use egui::pos2;

/// Capture actual text geometry through egui's font/layout path, without mocking fit decisions.
fn text_shapes(paint: impl Fn(&Painter)) -> Vec<egui::epaint::TextShape> {
    text_shapes_in(vec2(800., 600.), paint)
}

/// Capture a chosen viewport so repetition can be checked independently of off-screen copper.
fn text_shapes_in(size: egui::Vec2, paint: impl Fn(&Painter)) -> Vec<egui::epaint::TextShape> {
    let ctx = egui::Context::default();
    crate::install(&ctx);
    let mut out =
        ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() }, |ui| paint(ui.painter()));
    out.textures_delta.clear();
    out.shapes
        .into_iter()
        .filter_map(|shape| match shape.shape {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

/// A wide viewport needs a few well-separated names, regardless of the zoomed copper width.
#[test]
fn track_names_repeat_sparsely_at_readable_zooms() {
    for width in [24., 32., 55., 120.] {
        let text = text_shapes_in(vec2(1400., 600.), |p| track_net_label(p, [pos2(40., 300.), pos2(1360., 300.)], width, "+3V3", Color32::RED));
        assert_eq!(text.len(), 3);
        for pair in text.windows(2) {
            assert!(pair[1].pos.x - pair[0].pos.x >= 400.);
        }
    }
}

/// A later T junction, crossing, via or pad removes a covered word in full while leaving the
/// two other repetitions readable. Shapes follow the same paint order as the board canvas.
#[test]
fn later_copper_never_leaves_a_partial_track_name() {
    for cover in 0..5 {
        let text = text_shapes_in(vec2(1400., 600.), |p| {
            let mut labels = TrackLabels::new(p.clone());
            let ends = [pos2(40., 300.), pos2(1360., 300.)];
            track(p, ends, 55., Color32::RED, false, false);
            labels.track(ends, 55., "+3V3", Color32::RED);
            match cover {
                0..=2 => {
                    let ends = match cover {
                        0 => [pos2(700., 50.), pos2(700., 300.)],
                        1 => [pos2(700., 50.), pos2(700., 550.)],
                        _ => [pos2(450., 50.), pos2(950., 550.)],
                    };
                    labels.cover_track(ends, 55.);
                    track(p, ends, 55., Color32::BLUE, false, false);
                }
                3 => {
                    labels.cover_disc(pos2(700., 300.), 40.);
                    p.circle_filled(pos2(700., 300.), 40., Color32::YELLOW);
                }
                _ => {
                    let bounds = Rect::from_center_size(pos2(700., 300.), vec2(80., 60.));
                    labels.cover_rect(bounds);
                    p.rect_filled(bounds, 0., Color32::YELLOW);
                }
            }
        });
        assert_eq!(text.len(), 2, "cover case {cover}");
        for label in text {
            assert_eq!(label.galley.job.text, "+3V3");
            assert!(!label.visual_bounding_rect().intersects(Rect::from_center_size(pos2(700., 300.), vec2(80., 60.))));
        }
    }
}

/// A diagonal's bounding box must not erase text far from its actual projected copper.
#[test]
fn nearby_copper_preserves_uncovered_names() {
    let text = text_shapes(|p| {
        let mut labels = TrackLabels::new(p.clone());
        labels.track([pos2(80., 80.), pos2(700., 80.)], 24., "COL_A", Color32::BLUE);
        labels.cover_track([pos2(100., 100.), pos2(700., 500.)], 24.);
        labels.cover_disc(pos2(400., 150.), 15.);
        labels.cover_rect(Rect::from_min_max(pos2(500., 150.), pos2(600., 200.)));
    });
    assert_eq!(text.len(), 1);
}

/// Labels disappear before they become smaller than readable text or exceed segment copper;
/// rotated/reversed tracks stay upright and long visible segments repeat bounded labels.
#[test]
fn track_names_fit_visible_copper_and_remain_upright() {
    for ends in [
        [pos2(80., 80.), pos2(700., 80.)],
        [pos2(700., 80.), pos2(80., 80.)],
        [pos2(80., 80.), pos2(600., 500.)],
        [pos2(400., 500.), pos2(400., 80.)],
    ] {
        let text = text_shapes(|p| track_net_label(p, ends, 24., "U3.OUT", Color32::RED));
        assert!(!text.is_empty());
        for label in text {
            assert!(label.angle.abs() <= std::f32::consts::FRAC_PI_2);
            assert!(label.galley.size().y < 24.);
        }
    }
    for (ends, width) in
        [([pos2(80., 80.), pos2(85., 80.)], 24.), ([pos2(80., 80.), pos2(700., 80.)], 5.), ([pos2(-500., -80.), pos2(-100., -80.)], 24.)]
    {
        assert!(text_shapes(|p| track_net_label(p, ends, width, "U3.OUT", Color32::RED)).is_empty());
    }
    let text = text_shapes(|p| track_net_label(p, [pos2(-10000., 80.), pos2(10000., 80.)], 24., "U3.OUT", Color32::RED));
    assert!(!text.is_empty() && text.len() < 8, "preparation is bounded by the visible span");
}

/// Via text stays in the annular band and never covers an ordinary small via or its drill.
#[test]
fn via_names_need_legible_annular_space() {
    assert!(text_shapes(|p| via_net_label(p, pos2(200., 200.), 16., 8., "U3.OUT")).is_empty());
    let text = text_shapes(|p| via_net_label(p, pos2(200., 200.), 180., 40., "U3.OUT"));
    assert_eq!(text.len(), 1);
    let label = &text[0];
    let bounds = label.galley.rect.translate(label.pos.to_vec2());
    assert!(bounds.bottom() < 180.);
    for corner in [bounds.left_top(), bounds.right_top(), bounds.left_bottom(), bounds.right_bottom()] {
        assert!(corner.distance(pos2(200., 200.)) < 90.);
    }
}

/// Both lines stay inside the actual circular copper, rather than merely its bounding box.
#[test]
fn circular_pad_names_stay_inside_the_land() {
    let center = pos2(200., 200.);
    let corners = [pos2(160., 160.), pos2(240., 160.), pos2(240., 240.), pos2(160., 240.)];
    let pad = PadDrawing {
        number: "2".into(),
        center,
        copper: Some(corners),
        color: None,
        contour: PadContour::Circle,
        drill_radius: Some(10.),
        through_hole: true,
        net: Some("R1.2".into()),
    };
    let labels = text_shapes(|p| pad_label(p, &pad, corners, Color32::RED));
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].galley.job.text, "2\nR1.2");
    let bounds = labels[0].galley.rect.translate(labels[0].pos.to_vec2());
    for corner in [bounds.left_top(), bounds.right_top(), bounds.left_bottom(), bounds.right_bottom()] {
        assert!(corner.distance(center) < 40.);
    }
}
