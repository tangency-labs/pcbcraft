//! Selection filter rows and stacked right-hand cards share the workspace chrome.
use crate::Tokens;
use egui::{Align2, FontId, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

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

/// Draw a full-row checkbox with the same focus, hit-target and palette rhythm as Browser rows.
/// The outlined box remains legible when unchecked; mixed state is reserved for All items.
fn checkbox(ui: &mut Ui, checked: &mut bool, mixed: bool, label: &str, accessible: &str, width: f32) -> Response {
    let t = Tokens::get();
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, t.control_height), Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        if mixed {
            egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, ui.is_enabled(), accessible)
        } else {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *checked, accessible)
        }
    });
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if response.hovered() || response.has_focus() {
            p.rect_filled(rect, t.radius_sm, t.hover);
        }
        if response.has_focus() {
            p.rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }
        let center = pos2(rect.left() + t.space_4 + 7.0, rect.center().y);
        let box_rect = Rect::from_center_size(center, vec2(14.0, 14.0));
        let selected = *checked || mixed;
        p.rect(
            box_rect,
            3,
            if selected { t.accent_soft } else { t.panel },
            Stroke::new(1.0, if selected { t.accent } else { t.text_dim }),
            StrokeKind::Inside,
        );
        if mixed {
            p.hline(center.x - 3.5..=center.x + 3.5, center.y, Stroke::new(1.4, t.text));
        } else if *checked {
            p.add(Shape::line(vec![center + vec2(-3.5, 0.0), center + vec2(-0.8, 3.0), center + vec2(4.0, -3.5)], Stroke::new(1.5, t.text)));
        }
        p.text(pos2(box_rect.right() + t.space_8, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(t.font_button), t.text);
    }
    response
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
