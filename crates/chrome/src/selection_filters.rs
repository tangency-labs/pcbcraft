//! Selection filter rows and stacked right-hand cards share the workspace chrome.
use crate::Tokens;
use egui::{Align, Layout, Response, Ui, vec2};

/// One caller-owned object category; unsupported categories remain visible but disabled.
pub struct Filter<'a> {
    /// Stable identity independent of the visible label.
    pub key: &'a str,
    /// Short object-category label.
    pub label: &'a str,
    /// Whether the host supports individual selection for this category.
    pub enabled: bool,
    /// Current view-local selection permission.
    pub checked: bool,
}

/// Reserve a column cell for the shared checkbox. The master checkbox can show a mixed state.
fn checkbox(ui: &mut Ui, checked: &mut bool, mixed: bool, label: &str, accessible: &str, width: f32) -> Response {
    let height = Tokens::get().control_height;
    ui.allocate_ui_with_layout(vec2(width, height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        ui.spacing_mut().interact_size.y = height;
        solidcraft_ui_kit::checkbox_indeterminate(ui, label, checked, mixed).accessible_name(accessible)
    })
    .inner
}

/// Render an aligned master row and evenly spaced category columns. Narrow cards stack rows;
/// unsupported categories retain their disabled state and All items applies only to supported ones.
pub fn controls(ui: &mut Ui, filters: &mut [Filter<'_>]) -> bool {
    let t = Tokens::get();
    let mut changed = false;
    let mut all = filters.iter().filter(|row| row.enabled).all(|row| row.checked);
    let any = filters.iter().any(|row| row.enabled && row.checked);
    let mixed = any && !all;
    ui.add_space(t.space_4);
    let width = ui.available_width().min((ui.clip_rect().right() - ui.cursor().left()).max(1.0));
    ui.horizontal(|ui| {
        let toggle = checkbox(ui, &mut all, mixed, "All items", "All items", (width - 48.0 - ui.spacing().item_spacing.x).max(1.0));
        let reset = ui
            .add_sized(vec2(48.0, t.control_height), egui::Button::new(egui::RichText::new("Reset").size(t.font_body).color(t.text_dim)).frame(false))
            .clicked();
        if toggle.changed() || reset {
            for row in filters.iter_mut().filter(|row| row.enabled) {
                row.checked = reset || all;
            }
            changed = true;
        }
    });
    ui.add_space(t.space_4);
    ui.separator();
    ui.add_space(t.space_4);
    let columns = if width < 220.0 { 1 } else { 2 };
    let gap = t.space_8;
    let cell_width = (width - gap * (columns - 1) as f32) / columns as f32;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, t.space_2);
        for row in filters.chunks_mut(columns) {
            ui.horizontal(|ui| {
                for filter in row {
                    let response = ui
                        .push_id(filter.key, |ui| {
                            ui.add_enabled_ui(filter.enabled, |ui| {
                                checkbox(ui, &mut filter.checked, false, filter.label, &format!("Select {}", filter.label.to_lowercase()), cell_width)
                            })
                            .inner
                        })
                        .inner;
                    changed |= response.changed();
                }
            });
        }
    });
    ui.add_space(t.space_4);
    changed
}
