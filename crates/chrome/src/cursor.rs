//! Original canvas cursors, drawn in logical points for the current interface scale.

use crate::{Tokens, theme};
use egui::{Painter, Pos2, Stroke, vec2};

/// Draw a wiring crosshair and orthogonal-wire badge at the effective snapped position.
/// A ring distinguishes an existing connection target from a free grid point. The canvas
/// color under-stroke keeps the cursor readable over geometry in either theme.
pub fn wire(painter: &Painter, at: Pos2, connected: bool) {
    let t = Tokens::get();
    let arm = theme::WIRE_CURSOR_ARM;
    let gap = theme::WIRE_CURSOR_GAP;
    for stroke in [Stroke::new(3.5, t.canvas), Stroke::new(1.5, t.accent)] {
        for (a, b) in [
            (vec2(-arm, 0.0), vec2(-gap, 0.0)),
            (vec2(gap, 0.0), vec2(arm, 0.0)),
            (vec2(0.0, -arm), vec2(0.0, -gap)),
            (vec2(0.0, gap), vec2(0.0, arm)),
            (vec2(10.0, 10.0), vec2(10.0, 18.0)),
            (vec2(10.0, 18.0), vec2(19.0, 18.0)),
        ] {
            painter.line_segment([at + a, at + b], stroke);
        }
    }
    if connected {
        painter.circle_stroke(at, theme::WIRE_CURSOR_TARGET_RADIUS, Stroke::new(1.5, t.accent));
    }
}
