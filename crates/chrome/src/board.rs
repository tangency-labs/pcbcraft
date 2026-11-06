//! Board inspection primitives; physical geometry and selection ownership stay with callers.
use crate::Tokens;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, vec2};

/// Return the shared palette's copper colors; `None` denotes an internal layer.
pub fn copper_color(front: Option<bool>) -> Color32 {
    let t = Tokens::get();
    match front {
        Some(true) => t.board_front,
        Some(false) => t.board_back,
        None => t.board_inner,
    }
}

/// Screen-space hit area for a placement's pad-center markers, with screen-space padding for reliable pointer selection.
/// These sizes are UI affordances, not pad dimensions or a manufactured footprint outline.
pub fn marker_bounds(pads: &[Pos2], center: Pos2) -> Rect {
    pads.iter().fold(Rect::from_pos(center), |bounds, point| bounds.union(Rect::from_pos(*point))).expand2(vec2(12.0, 18.0))
}

/// Paint pad centers and reference as an inspection proxy, with distinct local and linked
/// highlights. No pad diameter, hole, courtyard or fabrication geometry is inferred.
pub fn footprint(p: &Painter, pads: &[(String, Pos2)], reference: &str, center: Pos2, copper: Color32, selected: bool, highlighted: bool) {
    let t = Tokens::get();
    let ink = if selected {
        t.accent
    } else if highlighted {
        t.board_highlight
    } else {
        copper
    };
    let points: Vec<_> = pads.iter().map(|(_, point)| *point).collect();
    let bounds = marker_bounds(&points, center);
    if selected || highlighted {
        p.rect_stroke(bounds, t.radius_sm, Stroke::new(1.5, ink), egui::StrokeKind::Inside);
    }
    for (number, point) in pads {
        p.circle_stroke(*point, 5.0, Stroke::new(1.5, ink));
        p.line_segment([*point - vec2(2.0, 0.0), *point + vec2(2.0, 0.0)], Stroke::new(1.0, ink));
        p.line_segment([*point - vec2(0.0, 2.0), *point + vec2(0.0, 2.0)], Stroke::new(1.0, ink));
        p.text(*point + vec2(0.0, 9.0), Align2::CENTER_TOP, number, FontId::proportional(t.font_small), t.board_ink);
    }
    p.text(
        bounds.center_top() - vec2(0.0, t.space_2),
        Align2::CENTER_BOTTOM,
        reference,
        FontId::proportional(t.font_body),
        if selected || highlighted { ink } else { t.board_ink },
    );
}

/// Explain an empty board once in the canvas; normal populated views need no instructional copy.
pub fn empty(p: &Painter, rect: Rect) {
    let t = Tokens::get();
    p.text(rect.center(), Align2::CENTER_CENTER, "No footprints placed", FontId::proportional(t.font_body), t.board_ink);
}

/// Paint a layer-color key without depending on optional font glyph coverage.
pub fn layer_swatch(ui: &mut egui::Ui, color: Color32) {
    let size = Tokens::get().space_8;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

/// Reserve a bounded upper-left Browser above Layers, keeping its header visible in short areas.
pub fn browser_bottom(area: Rect) -> f32 {
    area.top() + (area.height() * 0.36).min(210.0).max(crate::workspace::floating_card_min_height()).min(area.height())
}

/// Paint the board's own canvas and subdued line grid, independently of schematic/background
/// theme choices. Opacity and zoom thinning use the common navigation preferences.
pub fn canvas(p: &Painter, rect: Rect, origin: Pos2, step: f32, opacity: u8) {
    let t = Tokens::get();
    p.rect_filled(rect, 0, t.board_canvas);
    let (Some(color), Some(every)) = (crate::grid::color(t.board_grid, opacity), crate::grid::shown_every(step)) else {
        return;
    };
    let step = step * every as f32;
    for index in ((rect.left() - origin.x) / step).ceil() as i32..=((rect.right() - origin.x) / step).floor() as i32 {
        p.vline(origin.x + index as f32 * step, rect.y_range(), Stroke::new(1.0, color));
    }
    for index in ((rect.top() - origin.y) / step).ceil() as i32..=((rect.bottom() - origin.y) / step).floor() as i32 {
        p.hline(rect.x_range(), origin.y + index as f32 * step, Stroke::new(1.0, color));
    }
}
