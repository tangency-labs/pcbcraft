//! Editor pane layout and focus presentation. The host owns views, focus and document state.

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2};

use crate::Tokens;

/// Lay out two panes with a draggable divider. Narrow windows stack them vertically so each
/// retains usable width. The ratio is presentation state; dragging clamps it to 25–75 percent.
pub fn split(ui: &mut Ui, ratio: &mut f32) -> [Rect; 2] {
    let t = Tokens::get();
    let rect = ui.available_rect_before_wrap();
    let horizontal = rect.width() >= 900.0;
    let extent = if horizontal { rect.width() } else { rect.height() };
    let gap = t.space_6.min(extent);
    let position = |ratio: f32| (extent - gap) * ratio;
    let at = position(*ratio);
    let divider = if horizontal {
        Rect::from_min_max(pos2(rect.left() + at, rect.top()), pos2(rect.left() + at + gap, rect.bottom()))
    } else {
        Rect::from_min_max(pos2(rect.left(), rect.top() + at), pos2(rect.right(), rect.top() + at + gap))
    };
    let response = ui.interact(divider, ui.id().with("editor_divider"), Sense::drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Resize editor panes"));
    if response.dragged() && extent > gap {
        let delta = if horizontal { response.drag_delta().x } else { response.drag_delta().y };
        *ratio = (*ratio + delta / (extent - gap)).clamp(0.25, 0.75);
    }
    response.on_hover_cursor(if horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
    let at = position(*ratio);
    if horizontal {
        let x = rect.left() + at;
        ui.painter().rect_filled(Rect::from_min_max(pos2(x, rect.top()), pos2(x + gap, rect.bottom())), 0, t.border);
        [Rect::from_min_max(rect.min, pos2(x, rect.bottom())), Rect::from_min_max(pos2(x + gap, rect.top()), rect.max)]
    } else {
        let y = rect.top() + at;
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + gap)), 0, t.border);
        [Rect::from_min_max(rect.min, pos2(rect.right(), y)), Rect::from_min_max(pos2(rect.left(), y + gap), rect.max)]
    }
}

/// Identify a pane and mark keyboard ownership. Clicking its title requests focus without
/// changing the view's selection or camera. All colors and dimensions follow the chrome theme.
pub fn header(ui: &mut Ui, title: &str, active: bool) -> bool {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_24 + t.space_6), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, active, title));
    ui.painter().rect_filled(rect, 0, if active { t.accent_soft } else { t.panel });
    ui.painter().text(rect.left_center() + vec2(t.space_8, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(t.font_label), t.text);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(if active { 2.0 } else { 1.0 }, if active { t.accent } else { t.border }),
    );
    response.clicked()
}
