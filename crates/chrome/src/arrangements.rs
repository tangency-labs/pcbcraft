//! Menu access to named arrangements of project views and windows.

use egui::Ui;

/// Presentation intents; the host validates identities and owns all layout changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Select(u64),
    New,
    Duplicate(u64),
    Rename(u64),
    Remove(u64),
}

/// Keep saved layouts reachable in File without persistent title-bar tabs.
/// Selection and management emit intents; the host retains validation and window ownership.
pub fn menu(ui: &mut Ui, tabs: &[(u64, String)], active: u64) -> Option<Intent> {
    let mut intent = None;
    crate::menu::MenuItem::new("Layouts").enabled(!tabs.is_empty()).submenu(ui, |ui| {
        for (id, name) in tabs {
            ui.push_id(id, |ui| {
                crate::menu::MenuItem::new(name).choice(*id == active).submenu(ui, |ui| {
                    if crate::menu::MenuItem::new("Activate").show(ui) {
                        intent = Some(Intent::Select(*id));
                    }
                    intent = actions(ui, *id, tabs.len()).or(intent);
                });
            });
        }
        crate::menu::separator(ui);
        if crate::menu::MenuItem::new("New arrangement").show(ui) {
            intent = Some(Intent::New);
        }
    });
    intent
}

/// Use the same full-width Craft rows for active and inactive layout actions.
fn actions(ui: &mut Ui, id: u64, count: usize) -> Option<Intent> {
    let mut intent = None;
    for (label, pick) in [("Rename…", Intent::Rename(id)), ("Duplicate…", Intent::Duplicate(id)), ("Delete…", Intent::Remove(id))] {
        if crate::menu::MenuItem::new(label).enabled(!matches!(pick, Intent::Remove(_)) || count > 1).show(ui) {
            intent = Some(pick);
        }
    }
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
                submit |= crate::controls::button(ui, submit_label).clicked();
                cancel = crate::controls::button(ui, "Cancel").clicked();
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
