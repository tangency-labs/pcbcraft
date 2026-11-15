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

/// Shared single-line input padding and height keep table cells and labelled forms aligned.
pub fn input(value: &mut String) -> egui::TextEdit<'_> {
    let t = Tokens::get();
    egui::TextEdit::singleline(value)
        .margin(egui::Margin::symmetric(t.space_8 as i8, t.space_4 as i8))
        .min_size(vec2(0., t.control_height))
        .align(egui::Align2::LEFT_CENTER)
}

/// A single-line editable field with a stable accessible name, independent of label placement.
pub fn text(ui: &mut Ui, label: &str, value: &mut String) -> egui::Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label_response = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new(label).halign(Align::Min));
        let response = crate::controls::text(ui, value).labelled_by(label_response.id);
        crate::automation::publish(ui, &response, format!("field:{label}"));
        response
    })
    .inner
}

/// Keep a labelled value and its trailing action inside the form, reserving the action's measured width.
/// The returned flag reports only the action; its accessible name includes the field label.
pub fn text_action(ui: &mut Ui, label: &str, value: &mut String, action: &str) -> bool {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label_response = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new(label).halign(Align::Min));
        let action_width =
            egui::WidgetText::from(action).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button).size().x
                + ui.spacing().button_padding.x * 2.0
                + ui.spacing().item_spacing.x;
        let field_width = (ui.available_width() - action_width).max(0.0);
        ui.allocate_ui_with_layout(vec2(field_width, t.control_height), Layout::left_to_right(Align::Center), |ui| {
            crate::controls::text(ui, value).labelled_by(label_response.id);
        });
        let response = crate::controls::bounded_button(ui, label, action, action_width - ui.spacing().item_spacing.x, true);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{action} {label}")));
        response.clicked()
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
        solidcraft_ui_kit::dropdown_menu(ui, id, &selected, |ui| {
            for (key, text) in options {
                ui.selectable_value(value, (*key).into(), text);
            }
        })
        .labelled_by(label.id)
    })
    .inner
}

/// Compose a value selector with one creation action using the shared dropdown and aligned
/// menu rows. The host owns the selected key and handles creation after the popup dismisses.
pub fn choice_with_action(ui: &mut Ui, label: &str, id: egui::Id, value: &mut String, options: &[(&str, String)], action: &str) -> bool {
    let selected = options.iter().find(|(key, _)| *key == value.as_str()).map(|(_, name)| name.as_str()).unwrap_or("Choose library…");
    let mut create = false;
    field(ui, label, |ui| {
        crate::controls::Choice::from_id_salt(id).selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
            for (key, name) in options {
                if crate::MenuItem::new(name).choice(*key == value.as_str()).show(ui) {
                    *value = (*key).into();
                }
            }
            if !options.is_empty() {
                ui.separator();
            }
            create = crate::MenuItem::new(action).show(ui);
        });
    });
    create
}

/// Align a typed selector with the inspector's text fields, using the full value column and shared control height.
pub fn select(ui: &mut Ui, label: &str, id: egui::Id, selected: &str, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72., t.control_height), egui::Label::new(label).halign(Align::Min));
        ui.spacing_mut().interact_size.y = t.control_height;
        egui::ComboBox::from_id_salt(id).width(ui.available_width()).selected_text(selected).show_ui(ui, add).response.labelled_by(label.id)
    })
    .inner
}

/// Present a whole-degree angle with the same label column, width and height as adjacent inputs.
pub fn angle(ui: &mut Ui, label: &str, degrees: &mut u16) -> egui::Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72., t.control_height), egui::Label::new(label).halign(Align::Min));
        ui.add_sized(vec2(ui.available_width(), t.control_height), egui::DragValue::new(degrees).range(0..=359).suffix("°")).labelled_by(label.id)
    })
    .inner
}

/// Reserve the inspector's heading and action footer when scrolling a form in a short host.
pub fn form_body_height(area: egui::Rect) -> f32 {
    let height = Tokens::get().control_height;
    (area.height() - 6. * height).max(2. * height)
}

/// Distinguish symbol terminal identities from footprint pad identities above mapping rows.
/// Both columns keep the same proportions as [`mapping_choice`] in narrow and wide previews.
pub fn mapping_header(ui: &mut Ui) {
    let t = Tokens::get();
    let label_width = mapping_label_width(ui.available_width());
    ui.horizontal(|ui| {
        field_name(ui, "Symbol terminal", label_width, true);
        ui.label(RichText::new("Footprint pad").size(t.font_small).color(t.text_dim));
    });
}

/// A mapping row uses the Preferences control height and a full-width pad selector.
/// Long terminal names wrap within their column; the accessible name remains complete.
pub fn mapping_choice(ui: &mut Ui, label: &str, id: egui::Id, value: &mut String, options: &[(&str, String)]) -> egui::Response {
    let t = Tokens::get();
    let label_width = mapping_label_width(ui.available_width());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.space_8;
        ui.spacing_mut().interact_size.y = t.control_height;
        let label = field_name(ui, label, label_width, false);
        let selected = options.iter().find(|(key, _)| *key == value.as_str()).map(|(_, text)| text.as_str()).unwrap_or(value.as_str());
        crate::controls::Choice::from_id_salt(id)
            .width(ui.available_width())
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

/// Edit a terminal's physical pad set in the same aligned row as a single-pad mapping.
/// Checkbox choices keep the popup open; callers validate exclusive ownership across rows.
pub fn mapping_multi_choice(ui: &mut Ui, label: &str, id: egui::Id, values: &mut Vec<String>, options: &[(&str, String)]) -> egui::Response {
    let t = Tokens::get();
    let label_width = mapping_label_width(ui.available_width());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.space_8;
        ui.spacing_mut().interact_size.y = t.control_height;
        let label = field_name(ui, label, label_width, false);
        let selected = if values.is_empty() { "None".into() } else { values.join(", ") };
        solidcraft_ui_kit::dropdown_menu_with_close_behavior(ui, id, &selected, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            for (key, text) in options {
                let mut checked = values.iter().any(|value| value == key);
                if crate::controls::checkbox(ui, &mut checked, text).changed() {
                    if checked {
                        values.push((*key).into());
                    } else {
                        values.retain(|value| value != key);
                    }
                }
            }
        })
        .labelled_by(label.id)
    })
    .inner
}

/// Reserve enough of the preview for named terminals without shrinking pad controls off-screen.
fn mapping_label_width(width: f32) -> f32 {
    (width * 0.48).min(Tokens::get().form_label_width * 3.)
}

/// A quarter-turn field shares its label and alignment in compact and expanded properties.
pub fn rotation(ui: &mut Ui, degrees: &mut u16) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(72.0, t.control_height), egui::Label::new("Rotation").halign(Align::Min));
        crate::controls::Choice::from_id_salt("property_rotation")
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
            let height = columns.iter().map(|column| column.min_rect().height()).fold(0., f32::max);
            for column in columns {
                column.set_min_height(height);
            }
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

/// Allocate a large footprint canvas with shared sizing, palette and clipping in either host.
/// The response covers the complete canvas; the returned rectangle reserves its inner margin.
pub fn footprint_preview(ui: &mut Ui, label: &str) -> (egui::Rect, egui::Response) {
    let t = Tokens::get();
    let height = if ui.available_width() > 360. { 320. } else { 240. };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Pane, true, label));
    ui.painter_at(rect).rect_filled(rect, t.radius_md, t.board_canvas);
    (rect.shrink(t.space_12), response)
}

/// Keep a compact inspector's footprint stable across selection changes so a new selection
/// cannot move the canvas's navigation target. The body scrolls in short windows.
pub fn inspector_body(ui: &mut Ui) {
    ui.set_min_height(300.0_f32.min(ui.available_height()));
}

/// Property sections reuse Preferences rather than introducing a separate raised-card style.
pub fn group<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    crate::settings::form_section(ui, title, add)
}

/// Keep an action such as Footprint in the same value column as the editable component fields.
pub fn field_action(ui: &mut Ui, name: &str, action: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get();
    let width = ui.available_width();
    let label_width = field_label_width(width);
    ui.horizontal(|ui| {
        field_name(ui, name, label_width, false);
        crate::controls::bounded_button(
            ui,
            ("field_action", name),
            action,
            (width - label_width - 3. * t.space_8 - 2. * t.space_24).max(40.),
            enabled,
        )
    })
    .inner
}

/// A raised property group establishes hierarchy without adding a separate page or dialog.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    egui::Frame::new()
        .fill(t.panel_header)
        .stroke(egui::Stroke::new(1., t.dialog_rule))
        .corner_radius(t.radius_md)
        .inner_margin(t.space_12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_heading)));
            ui.add_space(t.space_8);
            add(ui)
        })
        .inner
}

/// Column labels keep the visibility eye understandable without repeating a caption on every row.
pub fn field_header(ui: &mut Ui) {
    let t = Tokens::get();
    let width = ui.available_width();
    let label_width = field_label_width(width);
    ui.horizontal(|ui| {
        field_name(ui, "Field", label_width, true);
        field_name(ui, "Content", (width - label_width - 3. * t.space_8 - 2. * t.space_24).max(40.), true);
        field_name(ui, "Link", t.space_24, true);
        ui.label(RichText::new("Visible").small().color(t.text_dim));
    });
}

/// Allocate a left-aligned label column without the centered layout imposed by `add_sized`.
fn field_name(ui: &mut Ui, name: &str, width: f32, header: bool) -> egui::Response {
    let t = Tokens::get();
    ui.allocate_ui_with_layout(vec2(width, t.control_height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(width);
        let text = if header { RichText::new(name).small().color(t.text_dim) } else { RichText::new(name) };
        ui.add(egui::Label::new(text).halign(Align::Min).wrap())
    })
    .inner
}

/// Editable field rows use a fixed left-aligned name column and the same eye toggle as layers.
/// The accessible input/toggle names do not depend on the displayed icon or column headings.
/// Web values expose an open-link action; `link` names an explicit link action even when its URL is empty.
pub fn field_row(ui: &mut Ui, name: &str, value: &mut String, visible: Option<&mut bool>, link: Option<&str>) -> egui::Response {
    let t = Tokens::get();
    let width = ui.available_width();
    let label_width = field_label_width(width);
    ui.horizontal(|ui| {
        let label = field_name(ui, name, label_width, false);
        let response = ui.add(input(value).desired_width((width - label_width - 3. * t.space_8 - 2. * t.space_24).max(40.))).labelled_by(label.id);
        let url = web_link(value);
        if url.is_some() || link.is_some() {
            let label = format!("Open {} link", link.unwrap_or(name));
            if crate::menu::quick_action(ui, crate::menu::MenuIcon::ExternalLink, &label, "", url.is_some()).clicked()
                && let Some(url) = url
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        } else {
            ui.allocate_space(vec2(t.space_24, t.space_24));
        }
        if let Some(visible) = visible {
            if crate::board::visibility(ui, name, *visible).clicked() {
                *visible = !*visible;
            }
        } else {
            ui.add_enabled_ui(false, |ui| crate::board::visibility(ui, name, false));
        }
        response
    })
    .inner
}

/// Long metadata names wrap within the compact column instead of pushing inputs off-screen.
fn field_label_width(width: f32) -> f32 {
    if width >= 600. { Tokens::get().form_label_width * 2. + Tokens::get().space_8 } else { 112. }
}

/// Property navigation uses shared segmented choices, falling back to a dropdown in narrow rows.
pub fn tabs<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, options: &[(T, &str)]) {
    crate::settings::segmented(ui, current, options);
    ui.add_space(Tokens::get().space_8);
}

/// Recognize absolute HTTP(S) field links with a nonempty authority. Whitespace, control
/// characters and backslashes are rejected; opening stays a host action, separate from editing.
pub fn web_link(value: &str) -> Option<&str> {
    let value = value.trim();
    let (scheme, rest) = value.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    ((scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && !authority.is_empty()
        && !authority.starts_with(':')
        && !value.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\'))
    .then_some(value)
}

/// A canvas-local field editor has no window or popup. It stays inside the owning canvas,
/// uses a readable zoom-aware font and shows validation errors beside the current draft.
#[allow(clippy::too_many_arguments)]
pub fn inline_text(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    value: &mut String,
    canvas: egui::Rect,
    anchor: egui::Rect,
    size: f32,
    focus: bool,
    error: Option<&str>,
) -> egui::Response {
    let t = Tokens::get();
    let font = egui::FontId::proportional(size.clamp(t.font_label, t.font_page_heading * 2.));
    let measured = ui.painter().layout_no_wrap(value.clone(), font.clone(), t.text).size();
    let width = (measured.x + 2. * t.space_8).max(anchor.width()).max(t.space_24 * 4.).min(canvas.width().max(1.));
    let height = (measured.y + 2. * t.space_4).max(t.control_height).min(canvas.height().max(1.));
    let left = (anchor.left() - t.space_8).clamp(canvas.left(), (canvas.right() - width).max(canvas.left()));
    let top = (anchor.center().y - height / 2.).clamp(canvas.top(), (canvas.bottom() - height).max(canvas.top()));
    let rect = egui::Rect::from_min_size(egui::pos2(left, top), vec2(width, height));
    let response = ui.put(rect, input(value).id(id).font(font).char_limit(2048));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    if focus {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(value.chars().count()))));
            state.store(ui.ctx(), id);
        }
    }
    if let Some(error) = error {
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1., t.error), egui::StrokeKind::Outside);
        let below = rect.bottom() + t.space_4;
        let at = if below + t.control_height * 2. < canvas.bottom() { below } else { (rect.top() - t.control_height * 2.).max(canvas.top()) };
        ui.put(
            egui::Rect::from_min_size(egui::pos2(rect.left(), at), vec2((canvas.right() - rect.left()).max(width), t.control_height * 2.)),
            egui::Label::new(RichText::new(error).color(t.error)).wrap(),
        );
    }
    response
}

/// Read-only Library geometry grows with the Preview's width and shares the
/// ordinary symbol/board palettes, card corners, hit target and inset rhythm.
pub fn geometry_preview(ui: &mut Ui, label: &str, footprint: bool) -> (egui::Rect, egui::Response) {
    let t = Tokens::get();
    let height = (ui.available_width() * 0.75).clamp(180., 360.);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Pane, ui.is_enabled(), label));
    ui.painter_at(rect).rect_filled(rect, t.radius_md, if footprint { t.board_canvas } else { t.canvas });
    (rect.shrink(t.space_12), response)
}

#[cfg(test)]
mod link_tests {
    /// Ordinary URL fields open on either host, while non-web values remain plain metadata.
    #[test]
    fn only_absolute_web_links_are_openable() {
        for value in ["https://example.com/data.pdf", "http://localhost:8080/a?b=c#d", "HTTPS://example.com"] {
            assert_eq!(super::web_link(value), Some(value));
        }
        assert_eq!(super::web_link(" https://example.com "), Some("https://example.com"));
        for value in [
            "",
            "~",
            "manual.pdf",
            "file:///tmp/manual.pdf",
            "javascript:alert(1)",
            "https:///file",
            "https://:80/file",
            "https://exa mple.com",
            "https://example.com\nfile",
            "https://example.com\\file",
        ] {
            assert!(super::web_link(value).is_none(), "{value:?}");
        }
    }
}
