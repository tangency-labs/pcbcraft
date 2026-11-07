//! Named arrangements of project views and windows in the title bar.

use egui::{Ui, WidgetInfo, WidgetType};

/// Presentation intents; the host validates identities and owns all layout changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Select(u64),
    New,
    Duplicate(u64),
    Rename(u64),
    Remove(u64),
}

/// Show only arrangement tabs and a plus. Opening a tab’s context menu does not select
/// that arrangement; each action carries the clicked tab’s identity to the host.
/// Tab tooltips explain the remembered view layout and available context actions.
pub fn tabs(ui: &mut Ui, tabs: &[(u64, String)], active: u64) -> Option<Intent> {
    if tabs.is_empty() {
        return None;
    }
    let mut intent = None;
    ui.horizontal(|ui| {
        let label = "+";
        let button_width = ui.painter().layout_no_wrap(label.into(), egui::TextStyle::Button.resolve(ui.style()), ui.visuals().text_color()).size().x
            + 2.0 * ui.spacing().button_padding.x;
        let width = (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(0.0);
        egui::ScrollArea::horizontal()
            .id_salt("arrangement_tabs")
            .max_width(width)
            .min_scrolled_width(0.0)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (id, name) in tabs {
                        ui.push_id(id, |ui| {
                            let selected = *id == active;
                            let response = ui.selectable_label(selected, name);
                            response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, format!("Arrangement {name}")));
                            response.context_menu(|ui| {
                                for (label, pick) in
                                    [("Rename…", Intent::Rename(*id)), ("Duplicate…", Intent::Duplicate(*id)), ("Delete…", Intent::Remove(*id))]
                                {
                                    if ui.add_enabled(!matches!(pick, Intent::Remove(_)) || tabs.len() > 1, egui::Button::new(label)).clicked() {
                                        intent = Some(pick);
                                        ui.close_kind(egui::UiKind::Menu);
                                    }
                                }
                            });
                            if response
                                .on_hover_text(
                                    "An arrangement of this project's workspaces, split panes and windows. Right-click to rename, duplicate or delete.",
                                )
                                .clicked()
                            {
                                intent = Some(Intent::Select(*id));
                            }
                        });
                    }
                });
            });
        let create = ui.button(label);
        create.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "New arrangement"));
        if create.on_hover_text("New arrangement").clicked() {
            intent = Some(Intent::New);
        }
    });
    intent
}

/// A naming or removal form, retained independently of menus and pointer position.
#[derive(Debug)]
pub struct Form {
    pub intent: Intent,
    pub name: String,
    pub single_view: bool,
    pub error: Option<String>,
    focused: bool,
}

/// A modal result never requests closure of an editor or project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Submit,
    Cancel,
}

impl Form {
    /// Start with a suggested name selected for replacement on the first rendered frame.
    pub fn new(intent: Intent, name: String) -> Self {
        Self { intent, name, single_view: false, error: None, focused: false }
    }

    /// Keep a rejected draft open and select its name so typing can correct it immediately.
    pub fn reject(&mut self, error: &str) {
        self.error = Some(error.into());
        self.focused = false;
    }

    /// Draw a stable modal with explicit submission and cancellation. Invalid names remain
    /// editable with the host's error, and Enter submits only from the focused name field.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let (title, submit_label) = match self.intent {
            Intent::Rename(_) => ("Rename arrangement", "Rename"),
            Intent::Duplicate(_) => ("Duplicate arrangement", "Create"),
            Intent::Remove(_) => ("Delete arrangement", "Delete"),
            _ => ("New arrangement", "Create"),
        };
        let (mut submit, mut cancel) = (false, false);
        let dismissed = crate::shell::dialog(ctx, "arrangement_dialog", title, |ui| {
            if matches!(self.intent, Intent::Remove(_)) {
                ui.label(format!("Delete “{}” and its saved view setup? Project documents are kept.", self.name));
            } else {
                let label = ui.label("Arrangement name");
                let mut field = egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).show(ui);
                field.response.response = field.response.response.labelled_by(label.id);
                if !self.focused {
                    field.response.request_focus();
                    field.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(self.name.chars().count()),
                    )));
                    field.state.store(ctx, field.response.id);
                    self.focused = true;
                }
                submit = field.response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if self.intent == Intent::New {
                    ui.label("Start with");
                    ui.radio_value(&mut self.single_view, false, "Current views and windows");
                    ui.radio_value(&mut self.single_view, true, "Single view in one window");
                }
                if let Some(error) = &self.error {
                    ui.label(crate::shell::error(error));
                }
            }
            ui.horizontal(|ui| {
                submit |= ui.button(submit_label).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || dismissed {
            Outcome::Cancel
        } else if submit {
            Outcome::Submit
        } else {
            Outcome::Open
        }
    }
}
