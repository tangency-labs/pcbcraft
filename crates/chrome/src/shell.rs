//! The parts of the window around the canvas: menu bar, status bar, dialogs.

use egui::{Frame, Margin, RichText, Stroke, Ui};

use crate::{fonts, theme::Tokens};

/// The menu bar along the top of the window.
pub fn menu_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    egui::Panel::top("chrome_menu_bar")
        .exact_size(t.control_height + t.space_4)
        .resizable(false)
        .frame(Frame::new().fill(t.panel).inner_margin(Margin::symmetric(t.space_4 as i8, t.space_2 as i8)))
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, add);
        });
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
