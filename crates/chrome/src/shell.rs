//! The parts of the window around the canvas: app bar, status bar, dialogs.

use egui::{Align, Frame, Label, Layout, Margin, RichText, Sense, Shape, Stroke, Ui, UiBuilder, vec2};

use crate::{fonts, theme::Tokens, titlebar};

pub use crate::titlebar::resize_window_edges;

/// Lay out the brand, Home button, file and history actions, project title, and right-side utilities.
/// Pass the project name and whether it has unsaved edits, or `None` for the empty workspace.
/// `home` draws beside the brand, before the separator, and stays at compact widths.
/// An integrated title bar adds desktop window controls and lets the free space move the window.
/// `actions_enabled` gates Home, file/history actions and utilities, leaving native controls usable.
/// Call [`resize_window_edges`] after dialogs to keep native interactions above their backdrop.
/// The utility callback gets the full bar width to decide which controls fit.
/// The title shrinks before taking space from the controls and keeps the unsaved marker if it fits.
pub fn app_bar(
    ui: &mut Ui,
    project: Option<(&str, bool)>,
    integrated: bool,
    actions_enabled: bool,
    home: impl FnOnce(&mut Ui),
    actions: impl FnOnce(&mut Ui),
    utilities: impl FnOnce(&mut Ui, f32),
) {
    let t = Tokens::get();
    egui::Panel::top("chrome_app_bar")
        .exact_size(t.control_height + t.space_6)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.app_bar).inner_margin(Margin::symmetric(t.space_8 as i8, t.space_2 as i8)))
        .show(ui, |ui| {
            egui::MenuBar::new()
                .style(move |style: &mut egui::Style| {
                    egui::menu::menu_style(style);
                    // Set the row height before egui places its first widget so the smaller
                    // brand mark and label share the same center as the full-height controls.
                    style.spacing.interact_size.y = t.control_height;
                    style.spacing.item_spacing.x = t.space_2;
                })
                .ui(ui, |ui| {
                    let bar_width = ui.available_width();
                    let compact = bar_width < 380.0;
                    if compact {
                        ui.spacing_mut().item_spacing.x = 0.0;
                    }
                    brand(ui, t, !compact);
                    ui.add_enabled_ui(actions_enabled, home);
                    if !compact {
                        ui.separator();
                    }
                    ui.add_enabled_ui(actions_enabled, actions);
                    if !compact {
                        ui.separator();
                    }

                    let area = ui.available_rect_before_wrap();
                    let mut right =
                        ui.new_child(UiBuilder::new().id_salt("app_bar_utilities").max_rect(area).layout(Layout::right_to_left(Align::Center)));
                    if integrated {
                        right.scope_builder(UiBuilder::new().layer_id(titlebar::interaction_layer()), |ui| {
                            titlebar::window_controls(ui, t, compact);
                        });
                        if compact {
                            right.add_space(t.space_2);
                        }
                    }
                    right.add_enabled_ui(actions_enabled, |ui| utilities(ui, bar_width));
                    let controls_left = right.min_rect().left() - t.space_2;
                    let (name, dirty) = project.unwrap_or(("No project", false));
                    let room = (controls_left - area.left()).max(0.0);
                    let marker_room = if dirty { t.space_6 } else { 0.0 };
                    let drag_room = if integrated { (room - marker_room).clamp(0.0, t.space_12) } else { 0.0 };
                    let title_room = (room - drag_room).min(200.0);
                    if !compact && title_room > t.space_24 {
                        project_title(ui, name, dirty, project.is_some(), title_room, t);
                    } else if dirty && title_room >= marker_room {
                        let (rect, _) = ui.allocate_exact_size(vec2(title_room.min(t.space_12), t.control_height), Sense::hover());
                        unsaved_marker(ui, rect, t);
                    }
                    if integrated {
                        let mut gap = ui.available_rect_before_wrap();
                        gap.max.x = controls_left;
                        titlebar::drag_area(ui, gap);
                    }
                });
        });
}

/// Draw the prototype's routed P mark and product name using the current theme.
/// Fall back to the default UI font on the first frame, before egui loads the bundled face.
fn brand(ui: &mut Ui, t: Tokens, show_name: bool) {
    let size = t.space_16 + t.space_4;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let at = |x, y| rect.min + vec2(x, y) * (size / 64.0);
    let painter = ui.painter();
    painter.add(Shape::convex_polygon(vec![at(8.0, 15.0), at(32.0, 4.0), at(56.0, 15.0), at(32.0, 26.0)], t.accent, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(8.0, 15.0), at(32.0, 26.0), at(32.0, 60.0), at(8.0, 49.0)], t.accent, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(32.0, 26.0), at(56.0, 15.0), at(56.0, 49.0), at(32.0, 60.0)], t.accent_soft, Stroke::NONE));
    let trace = Stroke::new(1.35, t.text);
    painter.add(Shape::line(
        vec![at(20.0, 44.0), at(20.0, 21.0), at(33.0, 15.0), at(46.0, 21.0), at(46.0, 33.0), at(33.0, 39.0), at(25.0, 35.0)],
        trace,
    ));
    painter.circle_filled(at(20.0, 44.0), 1.3, t.text);
    if !show_name {
        return;
    }
    ui.add_space(t.space_3);
    let font = fonts::interface_heading(ui, t.font_button);
    ui.label(RichText::new("PCBCraft").font(font).color(t.text));
    ui.add_space(t.space_4);
}

/// Fit the project tab to its title, with space for the unsaved marker even when the text truncates.
fn project_title(ui: &mut Ui, title: &str, dirty: bool, has_project: bool, room: f32, t: Tokens) {
    let font = if has_project { fonts::interface_heading(ui, t.font_button) } else { egui::FontId::proportional(t.font_button) };
    let color = if has_project { t.text } else { t.text_dim };
    let text_width = ui.painter().layout_no_wrap(title.to_owned(), font.clone(), color).size().x;
    let width = (text_width + 2.0 * t.space_8 + if dirty { t.space_12 } else { 0.0 }).min(room);
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.control_height - t.space_2), Sense::hover());
    if has_project {
        ui.painter().rect_filled(rect, t.radius_sm, t.app_bar_tab);
    }
    let mut label_rect = rect.shrink2(vec2(t.space_8, 0.0));
    if dirty {
        let marker = egui::Rect::from_center_size(label_rect.right_center() - vec2(t.space_2, 0.0), vec2(t.space_8, rect.height()));
        unsaved_marker(ui, marker, t);
        label_rect.max.x -= t.space_8;
    }
    ui.put(label_rect, Label::new(RichText::new(title).font(font).color(color)).truncate()).on_hover_text(title);
}

/// Draw the unsaved indicator and expose its meaning to pointer and assistive technology users.
fn unsaved_marker(ui: &mut Ui, rect: egui::Rect, t: Tokens) {
    const LABEL: &str = "Unsaved changes";
    ui.painter().circle_filled(rect.center(), t.space_2, t.accent);
    let response = ui.interact(rect, ui.id().with("unsaved_project"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, LABEL));
    response.on_hover_text(LABEL);
}

/// The status bar along the bottom: left and right ends. The right end (controls such as the
/// grid, and fixed indicators) is laid out first and always keeps its room; the left end's
/// messages get what remains and truncate with an ellipsis rather than run under it.
pub fn status_bar(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    egui::Panel::bottom("chrome_status_bar")
        .exact_size(t.control_height - t.space_4)
        .resizable(false)
        .frame(Frame::new().fill(t.panel).inner_margin(Margin::symmetric(t.space_8 as i8, 0)))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            let layout = |layout: egui::Layout, rect: egui::Rect| egui::UiBuilder::new().max_rect(rect).layout(layout);
            let used = ui.scope_builder(layout(egui::Layout::right_to_left(egui::Align::Center), bar), right).response.rect;
            let mut room = bar;
            if used.width() > 0.0 {
                room.max.x = (used.min.x - t.space_12).max(bar.min.x);
            }
            ui.scope_builder(layout(egui::Layout::left_to_right(egui::Align::Center), room), |ui| {
                ui.set_clip_rect(room.intersect(ui.clip_rect()));
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                left(ui);
            });
        });
}

/// The space left for the document, with the canvas colour.
pub fn canvas<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    egui::CentralPanel::default().frame(Frame::new().fill(t.canvas)).show(ui, add).inner
}

/// A modal dialog with a title. True when it should close: Escape, or a click outside.
pub fn dialog(ctx: &egui::Context, id: &str, title: &str, add: impl FnOnce(&mut Ui)) -> bool {
    dialog_width(ctx, id, title, 340.0, add)
}

/// Draw a modal with a preferred content width bounded by the viewport.
pub fn dialog_width(ctx: &egui::Context, id: &str, title: &str, width: f32, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    let frame = Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(t.radius_md).inner_margin(Margin::same(t.space_16 as i8));
    egui::Modal::new(egui::Id::new(id))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width.min((ctx.content_rect().width() - t.space_16 * 3.0).max(0.0)));
            ui.spacing_mut().item_spacing.y = t.space_8;
            ui.label(RichText::new(title).font(fonts::semibold(t.font_button + 2.0)));
            add(ui);
        })
        .should_close()
}

/// Small dim text, as in a status bar.
pub fn dim(text: impl Into<String>) -> RichText {
    let t = Tokens::get();
    RichText::new(text.into()).size(t.font_small).color(t.text_dim)
}

/// Small error text.
pub fn error(text: impl Into<String>) -> RichText {
    let t = Tokens::get();
    RichText::new(text.into()).size(t.font_small).color(t.error)
}
