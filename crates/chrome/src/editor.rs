//! Editor pane layout and focus presentation. The host owns views, focus and document state.

use egui::{FontId, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2};

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

/// Height of a document tab strip, also used to keep workspace cards below the first strip.
pub fn tab_height() -> f32 {
    let t = Tokens::get();
    t.space_24 + t.space_8
}

/// Show the sheet path on a document tab. Focus changes its underline, leaving the tab and
/// surrounding workspace stationary. Long paths elide from the left to retain the sheet name;
/// accessibility and hover text always contain the complete path. The host supplies stable IDs.
pub fn sheet_tab(ui: &mut Ui, path: &str, active: bool) -> bool {
    let t = Tokens::get();
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), tab_height()), Sense::hover());
    let font = FontId::proportional(t.font_label);
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), t.text);
    let available = (strip.width() - 2.0 * t.space_12).max(0.0);
    let mut label = path.to_owned();
    if measure(&label).size().x > available {
        for (index, _) in path.char_indices().skip(1) {
            label = format!("…{}", &path[index..]);
            if measure(&label).size().x <= available {
                break;
            }
        }
    }
    let galley = measure(&label);
    let width = (galley.size().x + 2.0 * t.space_12).min(strip.width());
    let tab = Rect::from_min_size(strip.min, vec2(width, strip.height()));
    let response = ui.interact(tab, ui.id().with("sheet_tab"), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, active, path));
    ui.painter().rect_filled(strip, 0, t.panel);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    let painter = ui.painter_at(tab);
    painter.rect_filled(tab, 0, t.canvas);
    painter.galley(pos2(tab.left() + t.space_12, tab.center().y - galley.size().y / 2.0), galley, t.text);
    painter.line_segment([tab.left_bottom(), tab.right_bottom()], Stroke::new(2.0, if active { t.accent } else { t.border }));
    response.on_hover_text(path).clicked()
}
