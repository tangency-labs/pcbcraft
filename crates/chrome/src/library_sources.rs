//! Shared counted headings and source-list scrolling, also used by the placement picker.
use crate::{Tokens, fonts};
use egui::{Align, Layout, RichText, Ui};
/// A pane title with a quiet, aligned count rather than an additional caption row.
pub fn heading(ui: &mut Ui, title: &str, count: usize) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_intro)).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(count.to_string()).size(t.font_label).color(t.text_dim));
        });
    });
}

/// Full-height scroll areas use independent IDs so one pane never displaces the other.
pub fn list(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    scrollbar(ui);
    egui::ScrollArea::vertical().id_salt(id).max_height(ui.available_height()).auto_shrink([false, false]).show(ui, add);
}

/// Reserve a narrow, visible scrollbar so clipped rows have a discoverable scroll affordance.
fn scrollbar(ui: &mut Ui) {
    let t = Tokens::get();
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.bar_width = t.space_6;
    ui.visuals_mut().widgets.inactive.fg_stroke.color = t.text_dim;
}
