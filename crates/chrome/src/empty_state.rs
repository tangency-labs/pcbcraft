//! Bounded workspace introductions composed from shared Craft surfaces and controls.

use egui::{Align, Layout, RichText, Ui};
use solidcraft_ui_kit::{Container, Surface, frame::Side};

use crate::{Tokens, controls, fonts};

/// A presentation choice for the host to validate and dispatch.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    None,
    Primary,
    Secondary,
}

/// Explain an empty workspace and its sequence without creating document state. The bounded
/// surface centers in large panes and scrolls in short panes; actions wrap at narrow widths.
pub fn show(ui: &mut Ui, title: &str, description: &str, actions: [&str; 2], steps: &[&str], enabled: bool) -> Action {
    let t = Tokens::get();
    let mut action = Action::None;
    egui::ScrollArea::vertical().id_salt("empty_workspace").auto_shrink([false, false]).show(ui, |ui| {
        crate::workspace::describe_scroll_viewport(ui);
        ui.add_space(t.space_24 + ((ui.available_height() - 380.0) * 0.4).max(0.0));
        let width = (ui.available_width() - t.space_16 * 2.0).clamp(0.0, 560.0);
        let inset = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(inset);
            let frame = Container::new(ui.ctx(), Surface::Feature).frame(ui.ctx(), t.panel, Side::Left).inner_margin(t.space_24);
            frame.show(ui, |ui| {
                ui.set_width((width - t.space_24 * 2.0).max(0.0));
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = t.space_12;
                    ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_title)));
                    ui.add(egui::Label::new(RichText::new(description).color(t.text_dim)).wrap());
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = t.space_8;
                            if ui.add(controls::action(actions[0], true)).clicked() {
                                action = Action::Primary;
                            }
                            if controls::button(ui, actions[1]).clicked() {
                                action = Action::Secondary;
                            }
                        });
                    });
                    ui.add_space(t.space_4);
                    ui.separator();
                    ui.label(RichText::new("Next steps").font(fonts::interface_heading(ui, t.font_heading)));
                    for (index, step) in steps.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new(format!("{}", index + 1)).color(t.accent));
                            ui.add(egui::Label::new(*step).wrap());
                        });
                    }
                });
            });
        });
        ui.add_space(t.space_24);
    });
    action
}
