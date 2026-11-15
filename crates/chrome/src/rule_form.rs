//! Dimensional forms compose Craft fields, sections and semantic notices in aligned cells.
use crate::{Tokens, settings};
use egui::{Align, Layout, Ui, vec2};
use solidcraft_ui_kit as kit;

/// A cell's role is presentation only; the host retains typed dimensions and authored state.
#[derive(Clone, Copy)]
pub enum Cell {
    Rule,
    Value,
    Source,
    /// Host-authored electronics geometry, independent of generic form controls.
    Diagram,
}

/// Pad the page inside shared modal chrome without introducing another nested surface.
pub fn body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    egui::Frame::new()
        .inner_margin(t.space_16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width().max(0.));
            ui.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
            add(ui)
        })
        .inner
}

/// Reuse the Preferences section heading/rule and its bounded spacing.
pub fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    settings::form_section(ui, title, add)
}

/// Align form fields without another scroll container. The pinned selectable table adds a
/// horizontal scroll host and row focus targets; forms need one outer vertical scroll owner
/// so keyboard/accessibility reveal reaches off-screen fields. Host illustrations occupy a
/// separate fixed-width geometry column; narrow hosts stack the rule above its other cells.
pub fn rows<T>(ui: &mut Ui, id: &str, title: &str, headings: [&str; 2], items: &mut [T], mut add: impl FnMut(&mut Ui, &mut T, Cell)) {
    let t = Tokens::get();
    let width = ui.available_width();
    ui.add_space(t.space_8);
    ui.push_id(id, |ui| {
        ui.spacing_mut().item_spacing.y = t.space_4;
        let diagram = 104.;
        let row_height = 52.;
        if width >= 600. {
            let value = 140.;
            let rule = (width * 0.30).max(216.);
            let source = width - rule - diagram - value - t.space_12 * 3.;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.space_12;
                cell(ui, rule, t.control_height, |ui| {
                    ui.strong(title);
                });
                for (name, width) in [("Geometry", diagram), (headings[0], value), (headings[1], source)] {
                    cell(ui, width, t.control_height, |ui| {
                        ui.label(crate::shell::dim(name));
                    });
                }
            });
            ui.separator();
            for (index, item) in items.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.space_12;
                        for (width, role) in [(rule, Cell::Rule), (diagram, Cell::Diagram), (value, Cell::Value), (source, Cell::Source)] {
                            cell(ui, width, row_height, |ui| add(ui, item, role));
                        }
                    })
                });
            }
        } else {
            let value = (width * 0.34).min(140.);
            let source = (width - diagram - value - t.space_8 * 2.).max(1.);
            ui.strong(title);
            ui.separator();
            ui.horizontal(|ui| {
                cell(ui, diagram, t.control_height, |ui| {
                    ui.label(crate::shell::dim("Geometry"));
                });
                cell(ui, value, t.control_height, |ui| {
                    ui.label(crate::shell::dim(headings[0]));
                });
                cell(ui, source, t.control_height, |ui| {
                    ui.label(crate::shell::dim(headings[1]));
                });
            });
            for (index, item) in items.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    add(ui, item, Cell::Rule);
                    ui.horizontal(|ui| {
                        cell(ui, diagram, row_height, |ui| add(ui, item, Cell::Diagram));
                        cell(ui, value, row_height, |ui| add(ui, item, Cell::Value));
                        cell(ui, source, row_height, |ui| add(ui, item, Cell::Source));
                    });
                    ui.add_space(t.space_4);
                });
            }
        }
    });
}

/// Reserve the complete column even when a label is short; controls keep their native style.
fn cell<R>(ui: &mut Ui, width: f32, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(width, height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// Present actual resolution conflicts as a shared semantic warning above the affected rows.
pub fn conflict<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    kit::banner(ui, kit::AssistantTone::Warning, add).inner
}
