//! Canvas selection feedback, following PCBcraft Next's contextual marquee styling.

use crate::Tokens;
use egui::{Align2, Color32, FontId, Painter, Rect, Stroke, StrokeKind, vec2};

/// Paint the selection window and hit count. Crossing uses a dashed border; containment is solid.
/// The caller supplies a painter clipped to its canvas, so the rectangle never covers chrome.
pub fn marquee(painter: &Painter, rect: Rect, crossing: bool, count: usize) {
    paint(painter, rect, crossing, count, Tokens::get().text);
}

/// Paint a marquee with the PCB canvas ink so its count remains readable in either shell theme.
pub fn board_marquee(painter: &Painter, rect: Rect, crossing: bool, count: usize) {
    paint(painter, rect, crossing, count, Tokens::get().board_ink);
}

/// Share the directional border and fill while allowing each canvas to choose legible text ink.
fn paint(painter: &Painter, rect: Rect, crossing: bool, count: usize, ink: Color32) {
    let t = Tokens::get();
    painter.rect_filled(rect, 0, t.selected_bg.gamma_multiply(0.25));
    let stroke = Stroke::new(1.0, t.accent);
    if crossing {
        for edge in [
            [rect.left_top(), rect.right_top()],
            [rect.right_top(), rect.right_bottom()],
            [rect.right_bottom(), rect.left_bottom()],
            [rect.left_bottom(), rect.left_top()],
        ] {
            let length = edge[0].distance(edge[1]);
            let mut offset = 0.0;
            while offset < length {
                painter.line_segment([edge[0].lerp(edge[1], offset / length), edge[0].lerp(edge[1], ((offset + 5.0) / length).min(1.0))], stroke);
                offset += 9.0;
            }
        }
    } else {
        painter.rect_stroke(rect, 0, stroke, StrokeKind::Inside);
    }
    painter.text(
        rect.left_top() - vec2(0.0, 5.0),
        Align2::LEFT_BOTTOM,
        format!("{} · {} items", if crossing { "Crossing" } else { "Window" }, count),
        FontId::proportional(t.font_small),
        ink,
    );
}
