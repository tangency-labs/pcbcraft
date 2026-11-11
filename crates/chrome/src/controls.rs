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
