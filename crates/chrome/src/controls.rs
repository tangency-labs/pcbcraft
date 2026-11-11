//! Native command controls rendered by the pinned shared Craft toolkit.
use egui::{Response, Ui, Widget, WidgetText};
use solidcraft_ui_kit::{self as kit, Button};

/// Render a secondary command action with native focus and accessibility.
pub fn button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    kit::button(ui, label, Button::Normal)
}

/// Render an enabled or disabled command action without changing its command ownership.
pub fn enabled_button(ui: &mut Ui, enabled: bool, label: impl Into<WidgetText>) -> Response {
    ui.add_enabled_ui(enabled, |ui| button(ui, label)).inner
}

/// Supply a shared action for sized/enabled layouts, retaining their minimum control height.
pub fn action(label: &str, primary: bool) -> impl Widget + '_ {
    move |ui: &mut Ui| {
        let height = ui.spacing().interact_size.y;
        ui.spacing_mut().interact_size.y = height.max(crate::Tokens::get().control_height);
        let response = kit::button(ui, label, if primary { Button::Primary } else { Button::Normal });
        ui.spacing_mut().interact_size.y = height;
        response
    }
}

/// Render a shared checkbox while retaining native change, focus and accessibility responses.
pub fn checkbox(ui: &mut Ui, value: &mut bool, label: impl AsRef<str>) -> Response {
    kit::checkbox(ui, label.as_ref(), value)
}

/// Search field with a stable host-owned identity and accessible name.
pub fn search(ui: &mut Ui, id: egui::Id, value: &mut String, hint: &str, name: &str) -> Response {
    kit::search_field(ui, id, value, hint).accessible_name(name)
}

/// Shared quiet hit target for caller-painted command glyphs. The host owns its bounds,
/// identity and action; toolkit hover/focus/selected chrome preserves disabled behavior.
pub fn glyph_target(ui: &mut Ui, id: egui::Id, bounds: egui::Rect, label: &str, enabled: bool, selected: bool) -> Response {
    kit::panel_host_control(
        ui,
        id,
        bounds,
        kit::HostChrome {
            label,
            icon: None,
            icon_only: true,
            attention: false,
            selected,
            enabled,
            drag: false,
            title: false,
            grip_only: false,
            rtl: false,
        },
    )
}

/// A host-identified choice menu rendered by the shared dropdown. The app supplies typed rows
/// and owns their changes; the adapter preserves native response and popup-result handling.
pub struct Choice {
    id: egui::IdSalt,
    selected: String,
    width: Option<f32>,
    label: Option<String>,
}

impl Choice {
    /// Identify the menu within its UI scope, independently of the selected display text.
    pub fn from_id_salt(id: impl egui::AsIdSalt) -> Self {
        Self { id: egui::IdSalt::new(id), selected: String::new(), width: None, label: None }
    }

    /// Identify and label a menu using its field name.
    pub fn from_label(label: &str) -> Self {
        Self { label: Some(label.to_owned()), ..Self::from_id_salt(label) }
    }

    /// Display the current value while preserving full text for accessibility.
    pub fn selected_text(mut self, text: impl Into<WidgetText>) -> Self {
        self.selected = text.into().text().to_owned();
        self
    }

    /// Bound the trigger and popup to the requested nonnegative width and available UI space.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.0));
        self
    }

    /// Draw shared dropdown chrome and run the host's rows only while its popup is open.
    /// Escape closes the popup before it can dismiss an enclosing modal.
    pub fn show_ui<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
        let width = self.width.unwrap_or(ui.spacing().combo_width).min(ui.available_width().max(0.0));
        let mut inner = None;
        let draw = |ui: &mut Ui| kit::bounded_dropdown_menu(ui, self.id, &self.selected, width, |ui| inner = Some(add(ui)));
        let response = if let Some(label) = &self.label {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                draw(ui).labelled_by(label.id)
            })
            .inner
        } else {
            draw(ui)
        };
        egui::InnerResponse { inner, response }
    }
}
