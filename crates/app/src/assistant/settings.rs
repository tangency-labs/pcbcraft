//! Preferences → AI, a lighter version of SolidCraft's Settings → AI
//! (`crates/ui-egui/src/agent_chat/settings.rs`, see `ATTRIBUTION.md`): the master switch, the
//! permission mode, the agents found and added (each with Test, Default, its own permission mode
//! and, for added ones, Edit and Remove), the agents not installed with their install command,
//! and the panel. Endpoint agents (OpenAI- or Anthropic-compatible) are not offered yet.

use craft_agents::agents::{self, AgentConfig, AgentPrefs, Kind, Mode};
use egui::{Context, Ui};
use pcbcraft_chrome::{
    Tokens,
    shell::{self, dim},
};
use serde::{Deserialize, Serialize};

use super::{Assistant, Visibility};

/// Most agents a person can add.
const MAX_AGENTS: usize = 32;
/// Longest command line accepted for an added agent.
const MAX_COMMAND: usize = 4096;

/// The person's AI settings. No keys or credentials.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub show_features: bool,
    pub toolbar: bool,
    pub open_at_start: bool,
    pub connect_at_start: bool,
    /// The agent picked in the panel, by id; `None` uses the default.
    pub picked: Option<String>,
    pub agents: AgentPrefs,
}

impl Default for Settings {
    fn default() -> Self {
        Self { show_features: true, toolbar: true, open_at_start: false, connect_at_start: false, picked: None, agents: AgentPrefs::default() }
    }
}

impl Settings {
    /// Restore saved settings. Settings from the earlier one-shot Assistant load as defaults,
    /// and added agents that can't start (no name or command, an endpoint kind, a duplicate id,
    /// too many) are dropped.
    pub(super) fn load(saved: Option<&str>) -> Self {
        let mut settings = saved.and_then(|text| serde_json::from_str::<Self>(text).ok()).unwrap_or_default();
        let mut seen = std::collections::BTreeSet::new();
        settings.agents.agents.retain(|agent| {
            agent.kind == Kind::Acp
                && !agent.id.trim().is_empty()
                && !agent.name.trim().is_empty()
                && valid_command(&agent.command)
                && seen.len() < MAX_AGENTS
                && seen.insert(agent.id.clone())
        });
        settings
    }
}

/// A command line that starts an agent: a program and its arguments, bounded, no control
/// characters. It runs without a shell, so quoting is the only syntax.
fn valid_command(command: &str) -> bool {
    command.len() <= MAX_COMMAND && !command.chars().any(char::is_control) && !craft_agents::acp::split_command(command).is_empty()
}

/// A request from the Assistant dropdown or the AI preferences page.
#[derive(Debug)]
pub(crate) enum Request {
    Settings,
    Add(Form),
}

/// The Add or Edit agent form: a name and the command that starts an ACP agent.
#[derive(Debug, Default)]
pub(crate) struct Form {
    editing: Option<String>,
    name: String,
    command: String,
    error: Option<String>,
}

impl Assistant {
    /// Render the AI page and return a request to open the agent editor.
    pub(crate) fn settings_page(&mut self, ui: &mut Ui) -> Option<Request> {
        let mut request = None;
        if self.settings.show_features {
            self.ensure_detection(ui.ctx());
        }
        ui.heading("AI");
        ui.separator();
        if ui.checkbox(&mut self.settings.show_features, "Show AI features").changed() {
            self.settings_changed = true;
            if !self.settings.show_features {
                self.hide(ui.ctx(), Visibility::Hidden);
                self.chat.disconnect();
            }
        }

        ui.add_enabled_ui(self.settings.show_features, |ui| {
            self.settings_changed |= ui.checkbox(&mut self.settings.toolbar, "Show Assistant button in application bar").changed();
            ui.separator();
            ui.strong("Permissions");
            ui.label(dim("What agents may do without asking. Reading the project always runs."));
            ui.horizontal_wrapped(|ui| {
                for mode in Mode::ALL {
                    if ui.selectable_label(self.settings.agents.ai_permissions == mode, mode.label()).clicked() {
                        self.settings.agents.ai_permissions = mode;
                        self.settings_changed = true;
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Agents");
                if ui.add_enabled(self.settings.agents.agents.len() < MAX_AGENTS, egui::Button::new("Add agent…")).clicked() {
                    request = Some(Request::Add(Form::default()));
                }
                if ui.add_enabled(self.detecting.is_none(), egui::Button::new("Refresh")).on_hover_text("Look for installed agents again").clicked() {
                    self.detect(ui.ctx());
                }
            });
            if self.detecting.is_some() {
                ui.label(dim("Looking for installed agents…"));
            }
            for entry in self.entries() {
                ui.push_id(&entry.id, |ui| self.agent_row(ui, &entry, &mut request));
            }
            let missing: Vec<(&'static str, &'static str)> = self
                .detection
                .as_ref()
                .map(|detection| detection.missing.iter().map(|agent| (agent.name, agent.install)).collect())
                .unwrap_or_default();
            if !missing.is_empty() {
                ui.add_space(Tokens::get().space_8);
                ui.label(dim("Not installed"));
                for (name, install) in missing {
                    ui.push_id(name, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(name);
                            ui.label(dim(install));
                            if ui.small_button("Copy").on_hover_text("Copy the install command").clicked() {
                                ui.ctx().copy_text(install.to_owned());
                            }
                        });
                    });
                }
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

    /// One agent: its name, what it is, Test, Default, its own permission mode, remembered
    /// answers, and Edit/Remove for an added one.
    fn agent_row(&mut self, ui: &mut Ui, entry: &agents::Entry, request: &mut Option<Request>) {
        ui.label(egui::RichText::new(&entry.name).strong());
        let login = match entry.login {
            "login_found" | "signed_in" => " · signed in",
            "signed_out" => " · sign-in needed",
            "key" => " · key in environment",
            _ => "",
        };
        ui.label(dim(format!("{}{login}", entry.detail())));
        ui.horizontal_wrapped(|ui| {
            let testing = self.tests.iter().any(|(id, _)| *id == entry.id);
            if ui.add_enabled(!testing, egui::Button::new("Test")).on_hover_text("Start the agent and check that it answers").clicked() {
                self.test_agent(ui.ctx(), entry);
            }
            if self.settings.agents.agent_default == entry.id {
                ui.label(dim("Default"));
            } else if ui.button("Make default").clicked() {
                self.settings.agents.agent_default.clone_from(&entry.id);
                self.settings_changed = true;
            }
            let own = self.settings.agents.agent_permissions.get(&entry.id).copied();
            let shown = own.map_or("Use the global mode", Mode::label);
            egui::ComboBox::from_id_salt("agent_mode").selected_text(shown).show_ui(ui, |ui| {
                let mut chosen = own;
                ui.selectable_value(&mut chosen, None, "Use the global mode");
                for mode in Mode::ALL {
                    ui.selectable_value(&mut chosen, Some(mode), mode.label());
                }
                if chosen != own {
                    match chosen {
                        Some(mode) => self.settings.agents.agent_permissions.insert(entry.id.clone(), mode),
                        None => self.settings.agents.agent_permissions.remove(&entry.id),
                    };
                    self.settings_changed = true;
                }
            });
            let remembered = self.settings.agents.agent_always_allow.get(&entry.id).map_or(0, Vec::len);
            if remembered > 0 && ui.button(format!("Forget {remembered} always-allowed")).clicked() {
                self.settings.agents.agent_always_allow.remove(&entry.id);
                self.settings_changed = true;
            }
            if !entry.found {
                if ui.button("Edit").clicked() {
                    let command =
                        self.settings.agents.agents.iter().find(|agent| agent.id == entry.id).map(|agent| agent.command.clone()).unwrap_or_default();
                    *request = Some(Request::Add(Form { editing: Some(entry.id.clone()), name: entry.name.clone(), command, error: None }));
                }
                if ui.button("Remove").clicked() {
                    self.remove_agent(&entry.id);
                }
            }
        });
        if self.tests.iter().any(|(id, _)| *id == entry.id) {
            ui.label(dim("Testing…"));
        } else if let Some(result) = self.test_results.get(&entry.id) {
            match result {
                Ok(answer) => ui.label(answer.as_str()),
                Err(error) => ui.label(shell::error(error.as_str())),
            };
        }
        if let Some(hint) = entry.problem_hint() {
            ui.label(shell::error(hint));
        }
        ui.add_space(Tokens::get().space_6);
    }

    /// Forget an added agent and everything remembered for it.
    fn remove_agent(&mut self, id: &str) {
        let prefs = &mut self.settings.agents;
        prefs.agents.retain(|agent| agent.id != id);
        prefs.agent_permissions.remove(id);
        prefs.agent_models.remove(id);
        prefs.agent_always_allow.remove(id);
        if prefs.agent_default == id {
            prefs.agent_default.clear();
        }
        if self.settings.picked.as_deref() == Some(id) {
            self.settings.picked = None;
        }
        self.test_results.remove(id);
        self.settings_changed = true;
    }

    /// Draw the add/edit dialog and save only a valid agent.
    pub(crate) fn agent_dialog(&mut self, ctx: &Context, form: &mut Form) -> bool {
        let mut done = false;
        let title = if form.editing.is_some() { "Edit agent" } else { "Add agent" };
        let closing = shell::dialog(ctx, "assistant_agent_dialog", title, |ui| {
            let t = Tokens::get();
            let field_width = (ui.available_width() - t.space_24 * 4.0).clamp(160.0, 320.0);
            egui::Grid::new("assistant_agent_form").num_columns(2).spacing([t.space_12, t.space_8]).show(ui, |ui| {
                let name_label = ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut form.name).char_limit(64).desired_width(field_width).hint_text("My agent"))
                    .labelled_by(name_label.id);
                ui.end_row();
                let command_label = ui.label("Command");
                ui.add(egui::TextEdit::singleline(&mut form.command).char_limit(MAX_COMMAND).desired_width(field_width).hint_text("opencode acp"))
                    .labelled_by(command_label.id);
                ui.end_row();
            });
            ui.label(dim("A program that speaks the Agent Client Protocol, with its arguments. It runs without a shell."));
            if let Some(error) = &form.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = true;
                } else if ui.button(if form.editing.is_some() { "Save" } else { "Add" }).clicked() {
                    match self.save_agent(form) {
                        Ok(()) => done = true,
                        Err(error) => form.error = Some(error.to_owned()),
                    }
                }
            });
        });
        closing || done
    }

    /// Store the agent under a stable id. Editing the connected agent's command disconnects it.
    fn save_agent(&mut self, form: &Form) -> Result<(), &'static str> {
        let name = form.name.trim();
        let command = form.command.trim();
        if name.is_empty() {
            return Err("Enter a name for this agent.");
        }
        if !valid_command(command) {
            return Err("Enter the command that starts the agent.");
        }
        if self.entries().iter().any(|entry| Some(&entry.id) != form.editing.as_ref() && entry.name.eq_ignore_ascii_case(name)) {
            return Err("An agent already uses this name.");
        }
        let connected = self.chat.agent.as_ref().map(|agent| agent.id.clone());
        let prefs = &mut self.settings.agents;
        let editing = form.editing.as_ref().and_then(|id| prefs.agents.iter().position(|agent| &agent.id == id));
        match editing {
            Some(index) => {
                let agent = &mut prefs.agents[index];
                let restart = agent.command != command;
                agent.name = name.to_owned();
                agent.command = command.to_owned();
                if restart && connected.as_ref() == form.editing.as_ref() {
                    self.chat.disconnect();
                }
            }
            None if prefs.agents.len() >= MAX_AGENTS => return Err("The agent list is full."),
            None => {
                let id = agents::new_id(prefs, "custom");
                prefs.agents.push(AgentConfig { id, name: name.to_owned(), kind: Kind::Acp, command: command.to_owned(), ..AgentConfig::default() });
            }
        }
        self.settings_changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings from the earlier Assistant load as defaults; unusable added agents are dropped.
    #[test]
    fn saved_settings_keep_only_agents_that_can_start() {
        let old = r#"{"selected":{"BuiltIn":"Claude"},"profiles":[{"id":1,"name":"x","agent":"Codex","command":"codex","kind":"Installed"}]}"#;
        let settings = Settings::load(Some(old));
        assert!(settings.agents.agents.is_empty());
        assert_eq!(settings.agents.ai_permissions, Mode::Edits, "Edits is the default, as in SolidCraft");

        let saved = r#"{"agents":{"agents":[
            {"id":"custom-1","name":"Open","kind":"acp","command":"opencode acp"},
            {"id":"custom-1","name":"Duplicate","kind":"acp","command":"x"},
            {"id":"custom-2","name":"","kind":"acp","command":"x"},
            {"id":"api-1","name":"Endpoint","kind":"api","base_url":"http://localhost:11434/v1","model":"m"},
            {"id":"custom-3","name":"Bad","kind":"acp","command":"a\u0007b"}
        ],"ai_permissions":"ask"}}"#;
        let settings = Settings::load(Some(saved));
        assert_eq!(settings.agents.agents.iter().map(|agent| agent.id.as_str()).collect::<Vec<_>>(), ["custom-1"]);
        assert_eq!(settings.agents.ai_permissions, Mode::Ask);
    }
}
