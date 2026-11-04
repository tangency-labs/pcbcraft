//! Menus share one popup surface and one row painter, like SolidCraft's. What a row does stays with
//! the caller.

use egui::{CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Ui, pos2, vec2};

use crate::theme::Tokens;

/// The surface every menu sits on.
pub fn frame() -> Frame {
    let t = Tokens::get();
    Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(t.radius_md))
        .inner_margin(Margin::same(t.space_4 as i8))
        .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(60) })
}

/// A menu in the menu bar.
pub fn top<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
    let t = Tokens::get();
    let previous = ui.style().clone();
    let surface = frame();
    let style = ui.style_mut();
    style.spacing.menu_margin = surface.inner_margin;
    style.visuals.menu_corner_radius = surface.corner_radius;
    style.visuals.popup_shadow = surface.shadow;
    style.visuals.window_fill = surface.fill;
    style.visuals.window_stroke = surface.stroke;
    // Fills the bar's height, so an open menu starts below the bar rather than over it.
    style.spacing.button_padding = vec2(t.space_8, t.space_6);
    let response = ui.menu_button(RichText::new(label).size(t.font_label), |ui| {
        ui.set_min_width(200.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    });
    ui.set_style(previous);
    response
}

pub fn separator(ui: &mut Ui) {
    let t = Tokens::get();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_8 + 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
}

/// One row: a label, an optional shortcut and an enabled state.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    pub label: &'a str,
    pub shortcut: &'a str,
    pub enabled: bool,
}

impl<'a> MenuItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { label, shortcut: "", enabled: true }
    }

    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = shortcut;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draw the row. True when it was clicked, and the menu closes.
    pub fn show(&self, ui: &mut Ui) -> bool {
        let response = self.row(ui);
        let clicked = response.clicked();
        if clicked {
            ui.close();
        }
        clicked
    }

    fn row(&self, ui: &mut Ui) -> Response {
        let t = Tokens::get();
        let height = t.control_height;
        // Popups first measure their content. Filling the provisional width would keep them wide.
        let width = if ui.is_sizing_pass() {
            let measure = |text: &str, font: FontId| ui.painter().layout_no_wrap(text.to_owned(), font, t.text).size().x;
            let label = measure(self.label, FontId::proportional(t.font_label));
            let shortcut = measure(self.shortcut, FontId::monospace(t.font_small));
            (label + shortcut + t.space_24 * 2.0).min(ui.available_width())
        } else {
            ui.available_width()
        };
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (r, response) = ui.allocate_exact_size(vec2(width, height), sense);
        let (enabled, label) = (self.enabled, self.label);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        let enabled = self.enabled && ui.is_enabled();
        let hot = enabled && (response.hovered() || response.has_focus());
        let mut painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(r));
        if !enabled {
            painter.multiply_opacity(0.4);
        }
        if hot {
            painter.rect_filled(r, t.control_radius, t.accent_soft);
        }
        let label = painter.layout_no_wrap(self.label.to_owned(), FontId::proportional(t.font_label), t.text);
        painter.galley(pos2(r.left() + t.space_8, r.center().y - label.size().y / 2.0), label, t.text);
        if !self.shortcut.is_empty() {
            let keys = painter.layout_no_wrap(self.shortcut.to_owned(), FontId::monospace(t.font_small), t.text_dim);
            painter.galley(pos2(r.right() - t.space_8 - keys.size().x, r.center().y - keys.size().y / 2.0), keys, t.text_dim);
        }
        response
    }
}
