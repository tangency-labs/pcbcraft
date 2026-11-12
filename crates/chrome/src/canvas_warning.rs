//! Positioned canvas notices reuse the Craft semantic state badge.
use egui::{Align, Id, Layout, Pos2, Rect, Response, Ui, UiBuilder, vec2};

/// Position a compact warning inside the canvas without advancing the parent's layout.
/// The shared badge owns its semantic colours, type, spacing and rounded surface.
pub fn badge(ui: &mut Ui, id: Id, at: Pos2, canvas: Rect, label: &str) -> Response {
    let tokens = solidcraft_theme::Tokens::get();
    let font = solidcraft_ui_kit::type_face::semibold(tokens.font_small);
    let width = ui.painter().layout_no_wrap(label.to_owned(), font, tokens.pref_note).size().x + tokens.space_16;
    let size = vec2(width, tokens.space_16 + tokens.space_2);
    let at = at.clamp(canvas.min, (canvas.max - size).max(canvas.min));
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(Rect::from_min_size(at, size)).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(ui.clip_rect().intersect(canvas));
    solidcraft_ui_kit::settings_state_badge(&mut child, label, solidcraft_ui_kit::SettingsBadgeTone::Warning)
}
