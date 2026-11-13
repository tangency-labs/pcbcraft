//! Shared Craft menu surfaces and rows with PCBcraft's popup routing and action glyphs.

use egui::{FontId, Frame, Response, RichText, Role, Sense, Ui, WidgetInfo, pos2, vec2};
use solidcraft_ui_kit::Menu;

use crate::theme::Tokens;

mod icons;
pub use icons::MenuIcon;

/// The surface every menu sits on.
pub fn frame() -> Frame {
    Menu::frame()
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
    crate::automation::publish(ui, &response.response, format!("menu:{label}"));
    response
}

/// Draw a quick action whose label is available to assistive technology.
/// Disabled actions still show their tooltip and never report a click. The caller runs the action.
pub fn quick_action(ui: &mut Ui, icon: MenuIcon, label: &str, shortcut: &str, enabled: bool) -> Response {
    let t = Tokens::get();
    let (_, rect) = ui.allocate_space(vec2(t.space_24, t.control_height));
    let response = crate::controls::glyph_target(ui, ui.scope_id().with(("quick_action", label)), rect, label, enabled, false);
    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.4);
    }
    icon.paint(&painter, rect.center(), t, enabled);
    let tooltip = if shortcut.is_empty() { label.to_owned() } else { format!("{label} ({shortcut})") };
    response.on_hover_text(tooltip.clone()).on_disabled_hover_text(tooltip)
}

/// Draw a bordered app bar action with its icon leading the visible label.
pub fn labeled_action(ui: &mut Ui, icon: MenuIcon, label: &str) -> Response {
    let t = Tokens::get();
    let text = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(t.font_label), t.text);
    let width = t.space_16 + t.space_6 + text.size().x + 2.0 * t.space_8;
    let (_, rect) = ui.allocate_space(vec2(width, t.control_height - t.space_2));
    let response = solidcraft_ui_kit::button_at(ui, label, rect, "", solidcraft_ui_kit::Button::Normal).accessible_name(label);
    let painter = ui.painter();
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

/// Draw a divider across the popup without expanding its provisional sizing-pass width.
/// Once content is measured, dividers fill the same width as the menu rows.
pub fn separator(ui: &mut Ui) {
    let t = Tokens::get();
    let width = if ui.is_sizing_pass() { ui.min_rect().width() } else { ui.available_width() };
    let (r, _) = ui.allocate_exact_size(vec2(width, t.space_8 + 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
}

/// Activation semantics for a menu row with a check mark.
#[derive(Clone, Copy, Debug)]
pub enum CheckKind {
    /// Select a value without clearing it when it is already selected.
    Choice,
    /// Flip an independent Boolean value on each activation.
    Toggle,
}

/// One row with a label and optional icon and shortcut.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    pub label: &'a str,
    pub shortcut: &'a str,
    pub enabled: bool,
    icon: Option<MenuIcon>,
    /// A ribbon glyph in the leading column, for rows that repeat a ribbon action.
    tool: Option<crate::workspace::ToolIcon>,
    /// `Some` for a row that displays a check mark when checked.
    checked: Option<(CheckKind, bool)>,
}

impl<'a> MenuItem<'a> {
    /// Open nested choices from the shared row painter, retaining egui's submenu behavior
    /// and the Craft popup surface. Disabled rows cannot open a submenu.
    pub fn submenu<R>(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
        let response = self.row(ui, true);
        if !self.enabled || !ui.is_enabled() {
            return None;
        }
        ui.scope(|ui| {
            let surface = frame();
            let style = ui.style_mut();
            style.spacing.menu_margin = surface.inner_margin;
            style.visuals.menu_corner_radius = surface.corner_radius;
            style.visuals.popup_shadow = surface.shadow;
            style.visuals.window_fill = surface.fill;
            style.visuals.window_stroke = surface.stroke;
            egui::containers::menu::SubMenu::default().show(ui, &response, add)
        })
        .inner
    }

    /// Start with an enabled text row. Use the builders to add a glyph or shortcut.
    pub fn new(label: &'a str) -> Self {
        Self { label, shortcut: "", enabled: true, icon: None, tool: None, checked: None }
    }

    /// Add a theme-aware glyph in the row's leading column.
    pub fn icon(mut self, icon: MenuIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Show a ribbon action's glyph in the leading column, as its ribbon group menu does.
    pub fn tool_icon(mut self, icon: crate::workspace::ToolIcon) -> Self {
        self.tool = Some(icon);
        self
    }

    /// Expose a mutually exclusive choice as a radio button; activation always selects it.
    /// The caller must apply that selection when this row is activated.
    pub fn choice(self, selected: bool) -> Self {
        self.with_check(CheckKind::Choice, selected)
    }

    /// Expose an independent checkbox toggle and report its new state on activation.
    /// The caller must flip the value when this row is activated.
    pub fn toggle(self, checked: bool) -> Self {
        self.with_check(CheckKind::Toggle, checked)
    }

    /// Apply explicit choice or toggle semantics supplied by a ribbon action.
    pub(crate) fn with_check(self, kind: CheckKind, checked: bool) -> Self {
        Self { checked: Some((kind, checked)), ..self }
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
        self.show_response(ui).clicked()
    }

    /// Draw the row and return its response, so the caller can preview what a hovered row will
    /// do. A click closes the menu, as with [`Self::show`].
    pub fn show_response(&self, ui: &mut Ui) -> Response {
        let response = self.row(ui, false);
        if response.clicked() {
            ui.close();
        }
        response
    }

    /// Draw and size the shared row. Its check state takes precedence over a PCBcraft glyph.
    fn row(&self, ui: &mut Ui, submenu: bool) -> Response {
        let mut item = solidcraft_ui_kit::MenuItem::new(self.label, self.label, "");
        item.enabled = self.enabled && ui.is_enabled();
        item.shortcut = self.shortcut;
        item.checked = self.checked.map(|(_, checked)| checked);
        item.submenu = submenu;
        let response = item.show(ui);
        crate::automation::publish_enabled(ui, &response, format!("menu:{}", self.label), self.enabled);
        response.widget_info(|| match self.checked {
            Some((CheckKind::Choice, selected)) => WidgetInfo::selected(Role::RadioButton, item.enabled, selected || response.clicked(), self.label),
            Some((CheckKind::Toggle, checked)) => WidgetInfo::selected(Role::CheckBox, item.enabled, checked ^ response.clicked(), self.label),
            None => WidgetInfo::labeled(Role::Button, item.enabled, self.label),
        });
        if self.checked.is_none() && (self.tool.is_some() || self.icon.is_some()) {
            let t = Tokens::get();
            let shared = solidcraft_theme::Tokens::get();
            let lead = response.rect.left_center() + vec2(shared.control_height / 2.0, 0.0);
            let mut painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(response.rect));
            if !item.enabled {
                painter.multiply_opacity(0.4);
            }
            if let Some(tool) = self.tool {
                crate::workspace::paint_menu_glyph(&painter, tool, lead, shared.space_16);
            } else if let Some(icon) = self.icon {
                icon.paint_sized(&painter, lead, shared.space_16, t, item.enabled && (response.hovered() || response.has_focus()));
            }
        }
        response
    }
}
