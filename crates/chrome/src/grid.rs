//! The canvas grid: its dots, how they thin out when zoomed out, and how strongly they show.
//!
//! Opacity is purely a matter of drawing. It never changes the grid's spacing or what the pointer
//! snaps to, so a hidden grid still places parts on the same points.

use std::ops::RangeInclusive;

use egui::{Color32, Painter, Pos2, Rect, Ui, pos2};

use crate::{Tokens, theme};

/// Grid opacity when nothing has been chosen, in percent: the grid at its full token colour.
pub const OPACITY_DEFAULT: u8 = 100;
/// The opacities a person can choose, in percent. Zero hides the grid.
pub const OPACITY_RANGE: RangeInclusive<u8> = 0..=100;
/// The slider moves in steps of this many percent.
pub const OPACITY_STEP: u8 = 5;

/// The colour to paint grid dots at `opacity_percent` of `base`, or `None` when the grid is
/// hidden. Values above 100% paint at full strength.
pub fn color(base: Color32, opacity_percent: u8) -> Option<Color32> {
    match opacity_percent.min(*OPACITY_RANGE.end()) {
        0 => None,
        100 => Some(base),
        percent => Some(base.gamma_multiply(f32::from(percent) / 100.0)),
    }
}

/// Paint the grid dots that fall inside `rect`, with the grid's zero at `origin` and `step` points
/// between placement grid points. When zoomed out past [`theme::SCHEMATIC_MIN_GRID_SPACING`] only
/// every fifth (then 25th, …) point is drawn. Nothing is painted at zero opacity or for a step
/// that isn't a positive finite number.
pub fn paint(painter: &Painter, rect: Rect, origin: Pos2, mut step: f32, opacity_percent: u8) {
    let Some(color) = color(Tokens::get().canvas_grid, opacity_percent) else {
        return;
    };
    if !step.is_finite() || step <= 0.0 {
        return;
    }
    while step < theme::SCHEMATIC_MIN_GRID_SPACING {
        step *= 5.0;
    }
    let first_x = ((rect.left() - origin.x) / step).ceil() as i32;
    let last_x = ((rect.right() - origin.x) / step).floor() as i32;
    let first_y = ((rect.top() - origin.y) / step).ceil() as i32;
    let last_y = ((rect.bottom() - origin.y) / step).floor() as i32;
    for x in first_x..=last_x {
        for y in first_y..=last_y {
            let center = pos2(origin.x + x as f32 * step, origin.y + y as f32 * step);
            painter.circle_filled(center, theme::SCHEMATIC_GRID_DOT_RADIUS, color);
        }
    }
}

/// A percentage slider for grid opacity and a Reset button that returns it to
/// [`OPACITY_DEFAULT`]. Reset is disabled while the value is already the default. Returns true
/// when the value changed this frame.
pub fn opacity_control(ui: &mut Ui, opacity_percent: &mut u8) -> bool {
    let t = Tokens::get();
    let before = *opacity_percent;
    let slider = egui::Slider::new(opacity_percent, OPACITY_RANGE).step_by(f64::from(OPACITY_STEP)).suffix("%");
    ui.add(slider);
    let reset = egui::Button::new(egui::RichText::new("Reset").size(t.font_label));
    if ui.add_enabled(*opacity_percent != OPACITY_DEFAULT, reset).on_hover_text(format!("Back to {OPACITY_DEFAULT}%")).clicked() {
        *opacity_percent = OPACITY_DEFAULT;
    }
    *opacity_percent != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_scales_the_grid_colour_and_zero_hides_it() {
        let base = Color32::from_rgb(194, 201, 211);
        assert_eq!(color(base, 0), None);
        assert_eq!(color(base, 100), Some(base));
        assert_eq!(color(base, 250), Some(base));
        let half = color(base, 50).unwrap_or(base);
        assert_eq!(half, base.gamma_multiply(0.5));
        assert!(half.a() < base.a());
    }

    /// Count the dots painted over a canvas, by drawing into a frame and reading its shapes.
    fn dots(rect: Rect, step: f32, opacity_percent: u8) -> usize {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            paint(&painter, rect, rect.center(), step, opacity_percent);
        });
        output.textures_delta.clear();
        output.shapes.len()
    }

    #[test]
    fn painting_follows_opacity_without_changing_the_points() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0));
        // A 10 point step over 100 points gives 11 dots on each axis.
        assert_eq!(dots(rect, 10.0, 100), 121);
        assert_eq!(dots(rect, 10.0, 35), 121);
        assert_eq!(dots(rect, 10.0, 0), 0);
        // Zoomed out, every fifth point is drawn, whatever the opacity.
        assert_eq!(dots(rect, 2.0, 100), 121);
        assert_eq!(dots(rect, 2.0, 5), 121);
        assert_eq!(dots(rect, 0.0, 100), 0);
        assert_eq!(dots(rect, f32::NAN, 100), 0);
    }
}
