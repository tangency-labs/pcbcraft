//! Menus share one popup surface and one row painter, like SolidCraft's. What a row does stays with
//! the caller.

use egui::{CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Ui, pos2, vec2};

use crate::theme::Tokens;

mod icons;
pub use icons::MenuIcon;

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

/// Anchor a popup under its label and restore the caller's style after drawing it.
fn top<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
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
    style.spacing.button_padding = vec2(t.space_8, t.space_4);
    style.spacing.interact_size.y = t.control_height;
    let response = ui.menu_button(RichText::new(label).size(t.font_label), |ui| {
        ui.set_min_width(200.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    });
    ui.set_style(previous);
    response
}

/// Draw a quick action whose label is available to assistive technology.
/// Disabled actions still show their tooltip and never report a click. The caller runs the action.
pub fn quick_action(ui: &mut Ui, icon: MenuIcon, label: &str, shortcut: &str, enabled: bool) -> Response {
    let t = Tokens::get();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(t.space_24, t.control_height), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let hot = enabled && (response.hovered() || response.has_focus());
    let mut painter = ui.painter().clone();
    if hot {
        painter.rect_filled(rect.shrink(t.space_2), t.radius_sm, t.hover);
    }
    if !enabled {
        painter.multiply_opacity(0.4);
    }
    icon.paint(&painter, rect.center(), t, enabled);
    let tooltip = if shortcut.is_empty() { label.to_owned() } else { format!("{label} ({shortcut})") };
    response.on_hover_text(tooltip)
}

/// Draw a bordered app bar action with its icon leading the visible label.
pub fn labeled_action(ui: &mut Ui, icon: MenuIcon, label: &str) -> Response {
    let t = Tokens::get();
    let text = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(t.font_label), t.text);
    let width = t.space_16 + t.space_6 + text.size().x + 2.0 * t.space_8;
    let (rect, response) = ui.allocate_exact_size(vec2(width, t.control_height - t.space_2), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if response.hovered() || response.has_focus() { t.hover } else { egui::Color32::TRANSPARENT };
    let painter = ui.painter();
    painter.rect(rect, t.radius_sm, fill, egui::Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
    icon.paint(painter, rect.left_center() + vec2(t.space_8 + t.space_16 / 2.0, 0.0), t, true);
    painter.galley(pos2(rect.left() + t.space_8 + t.space_16 + t.space_6, rect.center().y - text.size().y / 2.0), text, t.text);
    response
}

/// Draw the File trigger and render the fixed-width popup only while it is open.
/// The inner response contains the popup content's return value when it is shown.
pub fn file<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
    let t = Tokens::get();
    top(ui, "File", |ui| {
        ui.set_width(t.control_height * 8.0 + t.space_12);
        add(ui)
    })
}

/// Draw the ? button and, while it is open, the Help menu under it, on the same surface and rows
/// as File. Returns the content's value while the menu shows. Picking a row closes it.
pub fn help<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let t = Tokens::get();
    let response = quick_action(ui, MenuIcon::Help, "Help", "", true);
    // A fixed width, as File has: menu rows fill whatever width they are given. The ? button sits
    // at the right of the bar, so the menu hangs from its right edge.
    egui::Popup::menu(&response)
        .frame(frame())
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_width(t.control_height * 8.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .map(|shown| shown.inner)
}

/// Open a pointer context menu with the same surface and rows as the menu bar.
pub fn context<R>(response: &Response, add: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    egui::Popup::context_menu(response).frame(frame()).show(|ui| {
        ui.set_min_width(120.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    })
}

/// Draw a divider across the popup between groups of related actions.
pub fn separator(ui: &mut Ui) {
    let t = Tokens::get();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_8 + 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
}

/// One row with a label and optional icon and shortcut.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    pub label: &'a str,
    pub shortcut: &'a str,
    pub enabled: bool,
    icon: Option<MenuIcon>,
}

impl<'a> MenuItem<'a> {
    /// Start with an enabled text row. Use the builders to add a glyph or shortcut.
    pub fn new(label: &'a str) -> Self {
        Self { label, shortcut: "", enabled: true, icon: None }
    }

    /// Add a theme-aware glyph in the row's leading column.
    pub fn icon(mut self, icon: MenuIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Display a shortcut in its own column. The caller handles keyboard input.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = shortcut;
        self
    }

    /// Keep the row visible but prevent activation when disabled.
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

    /// Measure to content on the popup's sizing pass and fill the row on subsequent passes.
    /// Truncate long labels before the shortcut and preserve the full accessible label.
    fn row(&self, ui: &mut Ui) -> Response {
        let t = Tokens::get();
        let icon_column = if self.icon.is_some() { t.space_16 + t.space_8 } else { 0.0 };
        let height = if self.icon.is_some() { t.control_height + t.space_2 } else { t.control_height };
        // Popups first measure their content. Filling the provisional width would keep them wide.
        let width = if ui.is_sizing_pass() {
            let measure = |text: &str, font: FontId| ui.painter().layout_no_wrap(text.to_owned(), font, t.text).size().x;
            let label = measure(self.label, FontId::proportional(t.font_label));
            let shortcut = measure(self.shortcut, FontId::monospace(t.font_small));
            (label + shortcut + icon_column + t.space_24 * 2.0).min(ui.available_width())
        } else {
            ui.available_width()
        };
        let enabled = self.enabled && ui.is_enabled();
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (r, response) = ui.allocate_exact_size(vec2(width, height), sense);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, self.label));
        let hot = enabled && (response.hovered() || response.has_focus());
        let mut painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(r));
        if !enabled {
            painter.multiply_opacity(0.4);
        }
        if hot {
            painter.rect_filled(r, t.control_radius, t.accent_soft);
        }
        if let Some(icon) = self.icon {
            icon.paint(&painter, pos2(r.left() + t.space_16, r.center().y), t, hot);
        }
        let mut right = r.right() - t.space_8;
        if !self.shortcut.is_empty() {
            let keys = painter.layout_no_wrap(self.shortcut.to_owned(), FontId::monospace(t.font_small), t.text_dim);
            right -= keys.size().x;
            painter.galley(pos2(right, r.center().y - keys.size().y / 2.0), keys, t.text_dim);
            right -= t.space_12;
        }
        let left = r.left() + t.space_8 + icon_column;
        let mut text = egui::text::LayoutJob::simple_singleline(self.label.to_owned(), FontId::proportional(t.font_label), t.text);
        text.wrap.max_width = (right - left).max(0.0);
        text.wrap.max_rows = 1;
        let label = painter.layout_job(text);
        painter.galley(pos2(left, r.center().y - label.size().y / 2.0), label, t.text);
        response
    }
}
