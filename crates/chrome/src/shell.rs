//! The parts of the window around the canvas: app bar, status bar, dialogs.

use egui::{Align, Frame, Label, Layout, Margin, RichText, Sense, Shape, Stroke, Ui, vec2};

use crate::{fonts, theme::Tokens};

/// Lay out the brand, file and history actions, project title, and two right-side utilities.
/// Pass the project name and whether it has unsaved edits, or `None` for the empty workspace.
/// The title shrinks before taking space from the utilities. If only the
/// compact marker fits, a dirty project still shows its unsaved state.
pub fn app_bar(ui: &mut Ui, project: Option<(&str, bool)>, actions: impl FnOnce(&mut Ui), utilities: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    egui::Panel::top("chrome_app_bar")
        .exact_size(t.control_height + t.space_6)
        .resizable(false)
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
                    brand(ui, t);
                    ui.separator();
                    actions(ui);
                    ui.separator();

                    let utility_width = 2.0 * (t.space_24 + t.space_2);
                    let title_width = (ui.available_width() - utility_width).clamp(0.0, 200.0);
                    let (name, dirty) = project.unwrap_or(("No project", false));
                    if title_width > t.space_24 {
                        project_title(ui, name, dirty, title_width, t);
                    } else if dirty && title_width >= t.space_8 {
                        let (rect, _) = ui.allocate_exact_size(vec2(title_width.min(t.space_12), t.control_height), Sense::hover());
                        unsaved_marker(ui, rect, t);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), utilities);
                });
        });
}

/// Draw the prototype's routed P mark and product name using the current theme.
/// Fall back to the default UI font on the first frame, before egui loads the bundled face.
fn brand(ui: &mut Ui, t: Tokens) {
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
    ui.add_space(t.space_3);
    let semibold = fonts::semibold(t.font_button);
    let font = if ui.fonts(|fonts| fonts.families().contains(&semibold.family)) { semibold } else { egui::FontId::proportional(t.font_button) };
    ui.label(RichText::new("PCBCraft").font(font).color(t.text));
    ui.add_space(t.space_4);
}

/// Draw the title of the current project and keep the unsaved marker visible when text truncates.
fn project_title(ui: &mut Ui, title: &str, dirty: bool, width: f32, t: Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.control_height - t.space_2), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.panel_header);
    let mut label_rect = rect.shrink2(vec2(t.space_8, 0.0));
    if dirty {
        let marker = egui::Rect::from_center_size(label_rect.right_center() - vec2(t.space_2, 0.0), vec2(t.space_8, rect.height()));
        unsaved_marker(ui, marker, t);
        label_rect.max.x -= t.space_8;
    }
    ui.put(label_rect, Label::new(RichText::new(title).size(t.font_label).color(t.text)).truncate().halign(Align::Center));
}

/// Draw the unsaved indicator and expose its meaning to pointer and assistive technology users.
fn unsaved_marker(ui: &mut Ui, rect: egui::Rect, t: Tokens) {
    const LABEL: &str = "Unsaved changes";
    ui.painter().circle_filled(rect.center(), t.space_2, t.accent);
    let response = ui.interact(rect, ui.id().with("unsaved_project"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, LABEL));
    response.on_hover_text(LABEL);
}

/// The status bar along the bottom: left and right ends.
pub fn status_bar(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    egui::Panel::bottom("chrome_status_bar")
        .exact_size(t.control_height - t.space_4)
        .resizable(false)
        .frame(Frame::new().fill(t.panel).inner_margin(Margin::symmetric(t.space_8 as i8, 0)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                left(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
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
    let t = Tokens::get();
    let frame = Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(t.radius_md).inner_margin(Margin::same(t.space_16 as i8));
    egui::Modal::new(egui::Id::new(id))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(340.0);
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
