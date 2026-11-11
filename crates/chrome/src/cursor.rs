//! Original canvas cursors, drawn in logical points for the current interface scale.

use crate::{Tokens, theme};
use egui::{Color32, Painter, Pos2, Stroke, vec2};

/// Draw the wiring crosshair at the live `pointer` and mark `target`, where a click lands.
/// The crosshair follows every pointer movement, so the cursor never waits for the next grid
/// point. `connected` rings an existing connection target at `target`. Otherwise a small dot marks
/// the snapped free point, and it sits in the crosshair gap when the two coincide. The canvas
/// color under-stroke keeps both readable over geometry in either theme.
pub fn wire(painter: &Painter, pointer: Pos2, target: Pos2, connected: bool) {
    let t = Tokens::get();
    drawing(painter, pointer, target, connected, t.schematic_wire, t.canvas);
}

/// Draw the live routing crosshair and distinguish a copper contact from a free click target.
pub fn track(painter: &Painter, pointer: Pos2, target: Pos2, connected: bool) {
    let t = Tokens::get();
    drawing(painter, pointer, target, connected, t.accent, t.board_canvas);
}

/// Paint the shared crosshair at the pointer and the ring or dot at the resolved target.
fn drawing(painter: &Painter, pointer: Pos2, target: Pos2, connected: bool, ink: Color32, canvas: Color32) {
    let arm = theme::WIRE_CURSOR_ARM;
    let gap = theme::WIRE_CURSOR_GAP;
    for stroke in [Stroke::new(3.5, canvas), Stroke::new(1.5, ink)] {
        for (a, b) in [
            (vec2(-arm, 0.0), vec2(-gap, 0.0)),
            (vec2(gap, 0.0), vec2(arm, 0.0)),
            (vec2(0.0, -arm), vec2(0.0, -gap)),
            (vec2(0.0, gap), vec2(0.0, arm)),
        ] {
            painter.line_segment([pointer + a, pointer + b], stroke);
        }
    }
    if connected {
        painter.circle_stroke(target, theme::WIRE_CURSOR_TARGET_RADIUS, Stroke::new(1.5, ink));
    } else {
        painter.circle(target, theme::WIRE_CURSOR_SNAP_RADIUS, ink, Stroke::new(1.0, canvas));
    }
}
