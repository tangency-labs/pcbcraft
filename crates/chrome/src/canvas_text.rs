//! Continuously scaled canvas text over a bounded set of shared-font raster sizes.
//! Interface controls retain the toolkit's ordinary pixel-aligned text rendering.

use egui::{Align, Align2, Color32, FontId, Galley, Painter, Pos2, Rect, epaint::TextShape};
use std::sync::Arc;

// Ordinary overview/working text shares one raster size. Separate small-size buckets would
// reintroduce cold-font stalls as a smooth zoom crossed each of them; PCB pad text caps at 28 pt.
const RASTER_SIZES: [f32; 3] = [32., 64., 128.];

/// Lay out noneditable single-line canvas text at its requested physical screen size.
/// Glyphs come from a bounded raster-size bucket; mesh positions and ink bounds scale
/// continuously, so camera animation does not rasterize every glyph at each intermediate size.
/// Invalid sizes yield empty text. The painter owns DPI-sensitive font/atlas caching.
pub fn layout_no_wrap(painter: &Painter, text: String, font: FontId, color: Color32) -> Arc<Galley> {
    layout(painter, text, font, color, Align::Min)
}

/// Lay out canvas text with explicit line alignment, including multiline pad annotations.
/// This is for painted labels and their visual hit bounds, not editable text/caret geometry.
/// Beyond the largest raster bucket the mesh continues scaling without unbounded font sizes.
/// The galley's job retains its raster font size; use its scaled mesh/bounds for visual size.
pub fn layout(painter: &Painter, mut text: String, mut font: FontId, color: Color32, align: Align) -> Arc<Galley> {
    let requested = if font.size.is_finite() && font.size > 0. {
        font.size
    } else {
        text.clear();
        1.
    };
    let raster = RASTER_SIZES.into_iter().find(|size| *size >= requested).unwrap_or(128.);
    font.size = raster;
    let mut job = egui::text::LayoutJob::simple(text, font, color, f32::INFINITY);
    job.halign = align;
    let galley = painter.layout_job(job);
    let scale = requested / raster;
    if scale == 1. {
        return galley;
    }
    let mut shape = TextShape::new(Pos2::ZERO, galley, color);
    shape.transform(egui::emath::TSTransform::from_scaling(scale));
    shape.galley
}

/// Paint an anchored upright label using the same scaled layout that picking measures.
pub fn paint(painter: &Painter, at: Pos2, anchor: Align2, text: String, font: FontId, color: Color32) -> Rect {
    let galley = layout_no_wrap(painter, text, font, color);
    let bounds = anchor.anchor_size(at, galley.size());
    painter.galley(bounds.min, galley, color);
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Many intermediate camera scales must share warmed glyphs rather than uploading a new
    /// raster atlas each frame. Their layout and visual bounds still scale continuously.
    #[test]
    fn continuous_canvas_sizes_reuse_warm_font_rasters() {
        for ppp in [1., 2.] {
            let ctx = egui::Context::default();
            crate::install(&ctx);
            ctx.set_pixels_per_point(ppp);
            let mut first = None;
            for frame in 0..48 {
                let size = 8.1 + frame as f32 / 16.;
                let mut measured = None;
                let mut output = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))), ..Default::default() },
                    |ui| {
                        let galley = layout_no_wrap(ui.painter(), "R17 10 kΩ SENSOR_INPUT".into(), FontId::proportional(size), Color32::WHITE);
                        measured = Some(galley.size());
                        ui.painter().galley(Pos2::new(30., 30.), galley, Color32::WHITE);
                    },
                );
                if frame > 1 {
                    assert!(output.textures_delta.set.is_empty(), "frame {frame}, dpi {ppp}");
                }
                let bounds = measured.unwrap();
                let normalized = bounds / size;
                if let Some(first) = first {
                    assert!((normalized - first).length() < 0.001);
                } else {
                    first = Some(normalized);
                }
                output.textures_delta.clear();
            }
        }
    }

    /// Centered multiline pad text retains the same scaled line and rotated ink geometry
    /// used for picking, rather than snapping its displayed size to the raster bucket.
    #[test]
    fn multiline_alignment_and_rotated_bounds_follow_requested_scale() {
        let ctx = egui::Context::default();
        crate::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let small = layout(ui.painter(), "12\nGND".into(), FontId::monospace(8.5), Color32::WHITE, Align::Center);
            let large = layout(ui.painter(), "12\nGND".into(), FontId::monospace(10.5), Color32::WHITE, Align::Center);
            let ratio = 10.5 / 8.5;
            assert!((large.size() - small.size() * ratio).length() < 0.001);
            assert_eq!(small.rows.len(), 2);
            assert!((large.rows[1].pos - small.rows[1].pos * ratio).length() < 0.001);
            let small = TextShape::new(Pos2::ZERO, small, Color32::WHITE).with_angle(std::f32::consts::FRAC_PI_2).visual_bounding_rect();
            let large = TextShape::new(Pos2::ZERO, large, Color32::WHITE).with_angle(std::f32::consts::FRAC_PI_2).visual_bounding_rect();
            assert!((large.size() - small.size() * ratio).length() < 0.001);
            for size in [f32::NAN, f32::INFINITY, 0., -1.] {
                let hidden = layout_no_wrap(ui.painter(), "hidden".into(), FontId::proportional(size), Color32::WHITE);
                assert!(hidden.job.text.is_empty());
                assert_eq!(hidden.num_vertices, 0);
            }
        });
        output.textures_delta.clear();
    }
}
