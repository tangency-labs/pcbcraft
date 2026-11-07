//! Selection inspectors and detailed forms share the Craft field rhythm and hierarchy.
use crate::{Tokens, fonts};
use egui::{Align, Layout, RichText, Ui, vec2};

/// Render a compact selected-object heading with a secondary value and no instructional copy.
pub fn identity(ui: &mut Ui, reference: &str, value: &str) {
    let t = Tokens::get();
    ui.label(RichText::new(reference).font(fonts::interface_heading(ui, t.font_intro)));
    ui.add(egui::Label::new(RichText::new(value).color(t.text_dim)).truncate());
    ui.add_space(t.space_8);
}

/// Align a short label with a flexible field; stacked forms use the same control height.
pub fn field<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.add_sized(vec2(72.0, t.control_height), egui::Label::new(label).halign(Align::Min));
        ui.with_layout(Layout::left_to_right(Align::Center), add).inner
    })
    .inner
}

/// A single-line editable field with a stable accessible name, independent of label placement.
pub fn text(ui: &mut Ui, label: &str, value: &mut String) -> egui::Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new(label).halign(Align::Min));
        ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width())).labelled_by(label.id)
    })
    .inner
}

/// A labelled choice field keeps stored enum names separate from readable option labels and
/// supplies an accessible name even when the selected value contains no field identity.
pub fn choice(ui: &mut Ui, label: &str, id: egui::Id, value: &mut String, options: &[(&str, String)]) -> egui::Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new(label).halign(Align::Min));
        let selected = options.iter().find(|(key, _)| *key == value.as_str()).map(|(_, text)| text.as_str()).unwrap_or(value.as_str()).to_owned();
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (key, text) in options {
                    ui.selectable_value(value, (*key).into(), text);
                }
            })
            .response
            .labelled_by(label.id)
    })
    .inner
}

/// A quarter-turn field shares its label and alignment in compact and expanded properties.
pub fn rotation(ui: &mut Ui, degrees: &mut u16) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new("Rotation").halign(Align::Min));
        egui::ComboBox::from_id_salt("property_rotation")
            .selected_text(format!("{degrees}°"))
            .show_ui(ui, |ui| {
                for rotation in [0, 90, 180, 270] {
                    ui.selectable_value(degrees, rotation, format!("{rotation}°"));
                }
            })
            .response
            .labelled_by(label.id);
    });
}

/// Preserve a practical field width in compact hosts, bounded by the actual window edges.
pub fn inspector(ui: &Ui, id: egui::Id, area: egui::Rect, collapsed: bool, add: impl FnOnce(&mut Ui)) -> (egui::Rect, bool) {
    let width = if area.width() < 600.0 { area.width() } else { 260.0 };
    crate::workspace::floating_card_width(ui, id, "Properties", area, collapsed, true, width, add)
}

/// Stack the inspector below Browser on compact hosts instead of overlapping two edge cards.
pub fn inspector_area(mut area: egui::Rect, browser: egui::Rect) -> egui::Rect {
    if area.width() < 600.0 && browser.is_positive() {
        area.min.y = browser.bottom() + Tokens::get().space_8;
    }
    area
}

/// Keep room for the stacked inspector while retaining Browser's header and scrolling tree.
pub fn browser_area(mut area: egui::Rect) -> egui::Rect {
    if area.width() < 600.0 {
        area.max.y = area.top() + area.height() * 0.4;
    }
    area
}

/// Label a quiet field group using the shared heading weight and standard vertical gap.
pub fn section(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    ui.add_space(t.space_8);
    ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_heading)));
    ui.add_space(t.space_4);
}

/// Use two columns only when each can retain a practical field width.
pub fn columns(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    if ui.available_width() >= 600.0 {
        ui.columns(2, |cols| {
            left(&mut cols[0]);
            right(&mut cols[1]);
        });
    } else {
        left(ui);
        ui.separator();
        right(ui);
    }
}

/// Render related sections sequentially in two responsive columns, allowing one shared draft
/// to flow from a chooser into its details without overlapping mutable borrows.
pub fn split(ui: &mut Ui, mut section: impl FnMut(&mut Ui, usize)) {
    if ui.available_width() >= 600.0 {
        ui.columns(2, |columns| {
            section(&mut columns[0], 0);
            section(&mut columns[1], 1);
        });
    } else {
        section(ui, 0);
        ui.separator();
        section(ui, 1);
    }
}

/// Give definition previews a bounded canvas with the board palette and normal card insets.
pub fn preview(ui: &mut Ui, paint: impl FnOnce(&egui::Painter, egui::Rect)) {
    let t = Tokens::get();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.file_preview_max_height), egui::Sense::hover());
    ui.painter().rect_filled(rect, t.radius_md, t.board_canvas);
    paint(&ui.painter().with_clip_rect(rect), rect.shrink(t.space_16));
}

/// Keep a compact inspector's footprint stable across selection changes so a new selection
/// cannot move the canvas's navigation target. The body scrolls in short windows.
pub fn inspector_body(ui: &mut Ui) {
    ui.set_min_height(300.0_f32.min(ui.available_height()));
}
