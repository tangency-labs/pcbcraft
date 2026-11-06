//! Desktop Assistant preferences and the saved agent catalog.

use std::{
    ffi::OsStr,
    path::{Component, Path},
};

use egui::{Context, Ui};
use pcbcraft_chrome::{
    Tokens,
    shell::{self, dim},
};
use serde::{Deserialize, Serialize};

use super::{
    Assistant, JobKind, Outcome, Visibility,
    agent::{self, Agent},
    options::AgentOptions,
};

const MAX_PROFILES: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Choice {
    BuiltIn(Agent),
    Profile(u32),
}

impl Default for Choice {
    fn default() -> Self {
        Self::BuiltIn(Agent::default())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Profile {
    id: u32,
    name: String,
    agent: Agent,
    command: String,
    kind: Kind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub show_features: bool,
    pub toolbar: bool,
    pub open_at_start: bool,
    pub connect_at_start: bool,
    pub selected: Choice,
    default_agent: Option<Choice>,
    profiles: Vec<Profile>,
    /// Each agent's chosen model and effort; agents not listed use their defaults.
    options: Vec<ChoiceOptions>,
}

/// One agent's saved model and effort.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChoiceOptions {
    choice: Choice,
    #[serde(flatten)]
    options: AgentOptions,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_features: true,
            toolbar: true,
            open_at_start: false,
            connect_at_start: false,
            selected: Choice::default(),
            default_agent: None,
            profiles: Vec::new(),
            options: Vec::new(),
        }
    }
}

impl Settings {
    /// Restore saved profiles and fall back to a built-in agent if a selection is missing.
    pub(super) fn load(saved: Option<&str>, legacy: Option<&str>) -> Self {
        let mut settings = saved.and_then(|text| serde_json::from_str::<Self>(text).ok()).unwrap_or_else(|| Self {
            selected: Choice::BuiltIn(if legacy == Some("claude") { Agent::Claude } else { Agent::Codex }),
            ..Self::default()
        });
        let mut seen = [false; MAX_PROFILES as usize];
        settings.profiles.retain(|profile| {
            let Some(seen) = seen.get_mut(profile.id as usize) else { return false };
            valid_command(&profile.command) && !profile.name.trim().is_empty() && !std::mem::replace(seen, true)
        });
        let options = std::mem::take(&mut settings.options);
        for saved in options {
            if settings.exists(saved.choice) && settings.options(saved.choice) == AgentOptions::default() {
                settings.set_options(saved.choice, saved.options.sanitized());
            }
        }
        settings.default_agent = settings.default_agent.filter(|choice| settings.exists(*choice));
        settings.selected = settings.default_agent.unwrap_or(settings.selected);
        if !settings.exists(settings.selected) {
            settings.selected = Choice::default();
        }
        settings
    }

    /// Return whether this ID still resolves to a saved entry.
    fn exists(&self, choice: Choice) -> bool {
        matches!(choice, Choice::BuiltIn(_)) || self.profile(choice).is_some()
    }

    /// Look up a saved profile by ID so names and paths can be edited.
    fn profile(&self, choice: Choice) -> Option<&Profile> {
        self.profiles.iter().find(|profile| Choice::Profile(profile.id) == choice)
    }

    /// Resolve the CLI contract and executable used by Connect, Test, and Send.
    pub(super) fn target(&self, choice: Choice) -> (Agent, &str, &str) {
        match choice {
            Choice::BuiltIn(agent) => (agent, agent.label(), agent.executable()),
            Choice::Profile(_) => self.profile(choice).map_or((Agent::Codex, Agent::Codex.label(), Agent::Codex.executable()), |profile| {
                (profile.agent, profile.name.as_str(), profile.command.as_str())
            }),
        }
    }

    /// The model and effort `choice` runs with; defaults when none were chosen.
    pub(super) fn options(&self, choice: Choice) -> AgentOptions {
        self.options.iter().find(|saved| saved.choice == choice).map(|saved| saved.options.clone()).unwrap_or_default()
    }

    /// Remember `choice`'s model and effort. Choosing both defaults forgets the entry.
    pub(super) fn set_options(&mut self, choice: Choice, options: AgentOptions) {
        self.options.retain(|saved| saved.choice != choice);
        if options != AgentOptions::default() {
            self.options.push(ChoiceOptions { choice, options });
        }
    }

    /// Whether another custom profile can be saved.
    pub(super) fn can_add(&self) -> bool {
        self.profiles.len() < MAX_PROFILES as usize
    }

    /// List built-in agents first, then profiles in the order they were added.
    pub(super) fn choices(&self) -> impl Iterator<Item = Choice> + '_ {
        [Agent::Codex, Agent::Claude].into_iter().map(Choice::BuiltIn).chain(self.profiles.iter().map(|p| Choice::Profile(p.id)))
    }
}

/// A saved command is a single executable on PATH or an absolute path.
/// Requests run in a temporary directory, so relative paths with directories are ambiguous.
fn valid_command(command: &str) -> bool {
    if command.is_empty() || command.len() > 4096 || command.chars().any(char::is_control) {
        return false;
    }
    let path = Path::new(command);
    path.is_absolute()
        || (!command.chars().any(char::is_whitespace)
            && matches!(path.components().next(), Some(Component::Normal(_)))
            && !command.contains(['/', '\\']))
}

/// A request from the Assistant dropdown or the AI preferences page.
#[derive(Debug)]
pub(crate) enum Request {
    Settings,
    Add(Form),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    #[default]
    Installed,
    Custom,
}

#[derive(Debug, Default)]
pub(crate) struct Form {
    editing: Option<u32>,
    kind: Kind,
    agent: Agent,
    name: String,
    command: String,
    error: Option<String>,
}

impl Assistant {
    /// Render the AI page and return a request to open the agent editor.
    pub(crate) fn settings_page(&mut self, ui: &mut Ui) -> Option<Request> {
        let mut request = None;
        ui.heading("AI");
        ui.separator();
        if ui.checkbox(&mut self.settings.show_features, "Show AI features").changed() {
            self.settings_changed = true;
            if !self.settings.show_features {
                self.hide(ui.ctx(), Visibility::Hidden);
                self.auth = None;
                self.test_result = None;
                if let Some(job) = &self.job {
                    job.stop.store(true, std::sync::atomic::Ordering::Release);
                }
            }
        }

        ui.add_enabled_ui(self.settings.show_features, |ui| {
            self.settings_changed |= ui.checkbox(&mut self.settings.toolbar, "Show Assistant button in application bar").changed();
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Agents");
                if ui.add_enabled(self.settings.can_add(), egui::Button::new("Add agent…")).clicked() {
                    request = Some(Request::Add(Form::default()));
                }
            });
            let choices: Vec<_> = self.settings.choices().collect();
            for choice in choices {
                ui.push_id(choice_id(choice), |ui| self.agent_row(ui, choice, &mut request));
            }

            ui.separator();
            ui.strong("Panel");
            ui.horizontal_wrapped(|ui| {
                for (label, left, popped) in [("Left", true, false), ("Right", false, false), ("Own window", self.dock.left, true)] {
                    let selected = self.popped == popped && (popped || self.dock.left == left);
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        self.release_focus(ui.ctx());
                        self.popped = popped;
                        self.dock.left = left;
                        self.window_placed = false;
                        self.settings_changed = true;
                    }
                }
            });
            ui.separator();
            ui.strong("Startup");
            self.settings_changed |= ui.checkbox(&mut self.settings.connect_at_start, "Connect the default agent at startup").changed();
            self.settings_changed |= ui.checkbox(&mut self.settings.open_at_start, "Open Assistant at startup").changed();
        });
        request
    }

    /// Show the same catalog entries that the Assistant can connect to.
    fn agent_row(&mut self, ui: &mut Ui, choice: Choice, request: &mut Option<Request>) {
        let (agent, name, command) = self.settings.target(choice);
        let name = name.to_owned();
        ui.label(egui::RichText::new(&name).strong());
        let options = self.settings.options(choice);
        let shown = |value: &str| if value.is_empty() { "Default".to_owned() } else { value.to_owned() };
        ui.label(dim(format!("{} · {command} · Model {} · Effort {}", agent.label(), shown(&options.model), shown(&options.effort))));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.job.is_none(), egui::Button::new("Test")).on_hover_text(format!("Check sign-in for {name}")).clicked() {
                self.test_agent(ui.ctx(), choice);
            }
            if self.settings.default_agent == Some(choice) {
                ui.label(dim("Default"));
            } else if ui.button("Make default").clicked() {
                self.settings.default_agent = Some(choice);
                self.settings_changed = true;
            }
            if let Some(profile) = self.settings.profile(choice) {
                if ui.add_enabled(self.job.is_none(), egui::Button::new("Edit")).clicked() {
                    *request = Some(Request::Add(Form {
                        editing: Some(profile.id),
                        kind: profile.kind,
                        agent: profile.agent,
                        name: profile.name.clone(),
                        command: profile.command.clone(),
                        error: None,
                    }));
                }
                if ui.add_enabled(self.job.is_none(), egui::Button::new("Remove")).clicked() {
                    self.settings.profiles.retain(|profile| Choice::Profile(profile.id) != choice);
                    self.settings.set_options(choice, AgentOptions::default());
                    if self.settings.default_agent == Some(choice) {
                        self.settings.default_agent = None;
                    }
                    if self.settings.selected == choice {
                        self.select(self.settings.default_agent.unwrap_or_default());
                    }
                    self.test_result = None;
                    self.settings_changed = true;
                }
            }
        });
        if matches!(self.job.as_ref().map(|job| job.kind), Some(JobKind::Test(current)) if current == choice) {
            ui.label(dim("Checking sign-in…"));
        } else if let Some((current, result)) = &self.test_result
            && *current == choice
        {
            match result {
                Ok(true) => {
                    ui.label("Signed in");
                }
                Ok(false) => {
                    ui.label("Sign-in required");
                }
                Err(error) => {
                    ui.label(shell::error(error));
                }
            }
        }
    }

    /// Check this profile's sign-in in the background without changing the open conversation.
    fn test_agent(&mut self, ctx: &Context, choice: Choice) {
        let (agent, _, command) = self.settings.target(choice);
        let command = command.to_owned();
        if !self.start_job(ctx, JobKind::Test(choice), move |stop| {
            agent::authenticate(agent, OsStr::new(&command), false, stop).map(Outcome::Authenticated)
        }) {
            self.test_result = Some((choice, Err("Could not start the sign-in check.".to_owned())));
        }
    }

    /// Draw the add/edit dialog and save only a validated profile.
    pub(crate) fn agent_dialog(&mut self, ctx: &Context, form: &mut Form) -> bool {
        let mut done = false;
        let title = if form.editing.is_some() { "Edit agent" } else { "Add agent" };
        let closing = shell::dialog(ctx, "assistant_agent_dialog", title, |ui| {
            let t = Tokens::get();
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut form.kind, Kind::Installed, "Installed agent");
                ui.selectable_value(&mut form.kind, Kind::Custom, "Custom command");
            });
            ui.separator();
            let field_width = (ui.available_width() - t.space_24 * 4.0).clamp(100.0, 200.0);
            egui::Grid::new("assistant_agent_form").num_columns(2).spacing([t.space_12, t.space_8]).show(ui, |ui| {
                ui.label("Agent");
                egui::ComboBox::from_id_salt("profile_provider").selected_text(form.agent.label()).show_ui(ui, |ui| {
                    for agent in [Agent::Codex, Agent::Claude] {
                        ui.selectable_value(&mut form.agent, agent, agent.label());
                    }
                });
                ui.end_row();
                let name_label = ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut form.name).char_limit(64).desired_width(field_width).hint_text(form.agent.label()))
                    .labelled_by(name_label.id);
                ui.end_row();
                let command_label = ui.label("Agent command");
                if form.kind == Kind::Custom {
                    ui.add(
                        egui::TextEdit::singleline(&mut form.command).char_limit(4096).desired_width(field_width).hint_text(form.agent.executable()),
                    )
                    .labelled_by(command_label.id);
                } else {
                    ui.monospace(form.agent.executable());
                }
                ui.end_row();
            });
            if form.kind == Kind::Custom {
                ui.label(dim("Use an executable on PATH or an absolute path to a CLI compatible with the selected agent."));
            }
            if let Some(error) = &form.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = true;
                } else if ui.add_enabled(self.job.is_none(), egui::Button::new(if form.editing.is_some() { "Save" } else { "Add" })).clicked() {
                    match self.save_profile(form) {
                        Ok(()) => done = true,
                        Err(error) => form.error = Some(error.to_owned()),
                    }
                }
            });
        });
        closing || done
    }

    /// Store the profile under a stable ID. Editing the active command requires reconnecting.
    fn save_profile(&mut self, form: &Form) -> Result<(), &'static str> {
        let name = form.name.trim();
        if name.is_empty() {
            return Err("Enter a name for this agent.");
        }
        if self
            .settings
            .choices()
            .any(|choice| Some(choice) != form.editing.map(Choice::Profile) && self.settings.target(choice).1.eq_ignore_ascii_case(name))
        {
            return Err("An agent already uses this name.");
        }
        let command = match form.kind {
            Kind::Installed => form.agent.executable(),
            Kind::Custom => form.command.trim(),
        };
        if !valid_command(command) {
            return Err("Enter one executable name or absolute path.");
        }
        let id =
            form.editing.or_else(|| (0..MAX_PROFILES).find(|id| !self.settings.exists(Choice::Profile(*id)))).ok_or("The agent list is full.")?;
        let profile = Profile { id, name: name.to_owned(), agent: form.agent, command: command.to_owned(), kind: form.kind };
        if let Some(current) = self.settings.profiles.iter_mut().find(|current| current.id == id) {
            let target_changed = current.agent != profile.agent || current.command != profile.command;
            *current = profile;
            if target_changed && self.settings.selected == Choice::Profile(id) {
                self.reset_connection();
            }
        } else {
            self.settings.profiles.push(profile);
        }
        self.test_result = None;
        self.settings_changed = true;
        Ok(())
    }
}

/// Keep agent rows stable across name changes.
fn choice_id(choice: Choice) -> (bool, u32) {
    match choice {
        Choice::BuiltIn(Agent::Codex) => (false, 0),
        Choice::BuiltIn(Agent::Claude) => (false, 1),
        Choice::Profile(id) => (true, id),
    }
}
