//! Preferences → AI, a lighter version of SolidCraft's Settings → AI
//! (`crates/ui-egui/src/agent_chat/settings.rs`, see `ATTRIBUTION.md`): the master switch, the
//! permission mode, the agents found and added (each with Test, Default, its own permission mode
//! and, for added ones, Edit and Remove), the agents not installed with their install command,
//! and the panel. Model endpoints (OpenAI- or Anthropic-compatible) are added with a base URL,
//! model and optional key; the key goes to the system keychain, never to these settings.

use craft_agents::{
    agents::{self, AgentConfig, AgentPrefs, Kind, Mode},
    backend::Secrets,
};
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
    /// and added agents that can't start (no name, no command for an ACP agent, an unusable URL
    /// or no model for an endpoint, a duplicate id, too many) are dropped.
    pub(super) fn load(saved: Option<&str>) -> Self {
        let mut settings = saved.and_then(|text| serde_json::from_str::<Self>(text).ok()).unwrap_or_default();
        let mut seen = std::collections::BTreeSet::new();
        settings.agents.agents.retain(|agent| {
            let usable = match agent.kind {
                Kind::Acp => valid_command(&agent.command),
                Kind::Api | Kind::Anthropic => agents::normalize_url(&agent.base_url).is_ok() && valid_model(&agent.model),
            };
            usable && !agent.id.trim().is_empty() && !agent.name.trim().is_empty() && seen.len() < MAX_AGENTS && seen.insert(agent.id.clone())
        });
        settings
    }
}

/// A command line that starts an agent: a program and its arguments, bounded, no control
/// characters. It runs without a shell, so quoting is the only syntax.
fn valid_command(command: &str) -> bool {
    command.len() <= MAX_COMMAND && !command.chars().any(char::is_control) && !craft_agents::acp::split_command(command).is_empty()
}

/// A model name an endpoint takes: present, bounded, no control characters.
fn valid_model(model: &str) -> bool {
    craft_agents::model_id_valid(model.trim())
}

/// A request from the Assistant dropdown or the AI preferences page.
#[derive(Debug)]
pub(crate) enum Request {
    Settings,
    Add(Box<Form>),
}

/// The Add or Edit agent form: an ACP agent's name and command, or an endpoint's.
#[derive(Default)]
pub(crate) struct Form {
    editing: Option<String>,
    kind: Kind,
    name: String,
    command: String,
    base_url: String,
    model: String,
    /// A new key typed in; empty keeps a stored one. Never saved in settings.
    key: String,
    /// Remove the stored key on save.
    forget_key: bool,
    /// The endpoint has a key in the keychain.
    key_stored: bool,
    timeout_s: u64,
    effort: String,
    error: Option<String>,
}

impl std::fmt::Debug for Form {
    /// Describe the form without a key that may have been typed into it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Form").field("editing", &self.editing).field("kind", &self.kind).field("name", &self.name).finish_non_exhaustive()
    }
}

impl Assistant {
    /// Render human-controlled AI settings and return a request to open the agent editor.
    /// Synthetic input cannot operate this page or change permission policy.
    pub(crate) fn settings_page(&mut self, ui: &mut Ui) -> Option<Request> {
        let human = pcbcraft_chrome::automation::human_input(ui.ctx());
        ui.add_enabled_ui(human, |ui| self.settings_contents(ui)).inner
    }

    /// Keep permission policy and agent configuration under human control, including popups
    /// that were opened before a synthetic input pass began.
    fn settings_contents(&mut self, ui: &mut Ui) -> Option<Request> {
        let mut request = None;
        if self.settings.show_features {
            self.ensure_detection(ui.ctx());
        }
        ui.heading("AI");
        ui.separator();
        if pcbcraft_chrome::controls::checkbox(ui, &mut self.settings.show_features, "Show AI features").changed() {
            self.settings_changed = true;
            if !self.settings.show_features {
                self.hide(ui.ctx(), Visibility::Hidden);
                self.chat.disconnect();
            }
        }

        ui.add_enabled_ui(self.settings.show_features, |ui| {
            self.settings_changed |=
                pcbcraft_chrome::controls::checkbox(ui, &mut self.settings.toolbar, "Show Assistant button in application bar").changed();
            ui.separator();
            ui.strong("Permissions");
            ui.label(dim("What agents may do without asking. Reading the project always runs."));
            ui.horizontal_wrapped(|ui| {
                for mode in Mode::ALL {
                    if ui.selectable_label(self.settings.agents.ai_permissions == mode, mode.label()).clicked()
                        && pcbcraft_chrome::automation::human_input(ui.ctx())
                    {
                        self.settings.agents.ai_permissions = mode;
                        self.settings_changed = true;
                    }
                }
            });
            let mut automatic = self.settings.agents.auto_approve_bridge;
            let auto = ui.checkbox(&mut automatic, "Approve agents' requests to use PCBCraft's tools automatically").on_hover_text(
                "PCBCraft still applies the permission mode to every command. Turn this off to also confirm each \
                     request in the agent's own words, which shows that the call comes from it, at the cost of a \
                     second question for commands the mode asks about.",
            );
            if auto.changed() && pcbcraft_chrome::automation::human_input(ui.ctx()) {
                self.settings.agents.auto_approve_bridge = automatic;
                self.settings_changed = true;
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Agents");
                if pcbcraft_chrome::controls::enabled_button(ui, self.settings.agents.agents.len() < MAX_AGENTS, "Add agent…").clicked() {
                    request = Some(Request::Add(Box::default()));
                }
                if pcbcraft_chrome::controls::enabled_button(ui, self.detecting.is_none(), "Refresh")
                    .on_hover_text("Look for installed agents again")
                    .clicked()
                {
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
                            if pcbcraft_chrome::controls::small_button(ui, "Copy").on_hover_text("Copy the install command").clicked() {
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
            self.settings_changed |=
                pcbcraft_chrome::controls::checkbox(ui, &mut self.settings.connect_at_start, "Connect the default agent at startup").changed();
            self.settings_changed |= pcbcraft_chrome::controls::checkbox(ui, &mut self.settings.open_at_start, "Open Assistant at startup").changed();
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
            if pcbcraft_chrome::controls::enabled_button(ui, !testing, "Test").on_hover_text("Start the agent and check that it answers").clicked() {
                self.test_agent(ui.ctx(), entry);
            }
            if self.settings.agents.agent_default == entry.id {
                ui.label(dim("Default"));
            } else if pcbcraft_chrome::controls::button(ui, "Make default").clicked() {
                self.settings.agents.agent_default.clone_from(&entry.id);
                self.settings_changed = true;
            }
            let own = self.settings.agents.agent_permissions.get(&entry.id).copied();
            let shown = own.map_or("Use the global mode", Mode::label);
            pcbcraft_chrome::controls::Choice::from_id_salt("agent_mode")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    let mut chosen = own;
                    ui.selectable_value(&mut chosen, None, "Use the global mode");
                    for mode in Mode::ALL {
                        ui.selectable_value(&mut chosen, Some(mode), mode.label());
                    }
                    if chosen != own && pcbcraft_chrome::automation::human_input(ui.ctx()) {
                        match chosen {
                            Some(mode) => self.settings.agents.agent_permissions.insert(entry.id.clone(), mode),
                            None => self.settings.agents.agent_permissions.remove(&entry.id),
                        };
                        self.settings_changed = true;
                    }
                })
                .response
                .accessible_name("Agent permission mode");
            let remembered = self.settings.agents.agent_always_allow.get(&entry.id).map_or(0, Vec::len);
            if remembered > 0 && pcbcraft_chrome::controls::button(ui, format!("Forget {remembered} always-allowed")).clicked() {
                self.settings.agents.agent_always_allow.remove(&entry.id);
                self.settings_changed = true;
            }
            if !entry.found {
                if pcbcraft_chrome::controls::button(ui, "Edit").clicked()
                    && let Some(agent) = self.settings.agents.agents.iter().find(|agent| agent.id == entry.id)
                {
                    *request = Some(Request::Add(Box::new(Form {
                        editing: Some(agent.id.clone()),
                        kind: agent.kind,
                        name: agent.name.clone(),
                        command: agent.command.clone(),
                        base_url: agent.base_url.clone(),
                        model: agent.model.clone(),
                        key_stored: agent.key,
                        timeout_s: agent.request_timeout_s,
                        effort: agent.reasoning_effort.clone(),
                        ..Form::default()
                    })));
                }
                if pcbcraft_chrome::controls::button(ui, "Remove").clicked() {
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

    /// Forget an added agent, its stored key and everything remembered for it.
    fn remove_agent(&mut self, id: &str) {
        self.remove_agent_with(id, |id| super::host::Keychain.delete(id));
    }

    /// [`Self::remove_agent`], with `delete_key` removing its key from the keychain. When the key
    /// can't be removed, nothing else is: the agent's row stays and says why. A connected chat
    /// with the agent ends first, so it doesn't run on, or keep a key, after the agent is gone.
    /// A connection test still running for it is dropped, so its result can't replace the reason.
    fn remove_agent_with(&mut self, id: &str, delete_key: impl FnOnce(&str) -> Result<(), String>) {
        self.tests.retain(|(test, _)| test != id);
        if self.settings.agents.agents.iter().any(|agent| agent.id == id && agent.key)
            && let Err(error) = delete_key(id)
        {
            self.test_results.insert(id.to_owned(), Err(format!("The key stays in the keychain, so the agent was kept: {error}")));
            return;
        }
        if self.chat.agent.as_ref().is_some_and(|connected| connected.id == id) {
            self.chat.disconnect();
        }
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

    /// Draw the add/edit dialog and save only a valid agent: an ACP agent by its command line,
    /// or a model endpoint by protocol, base URL, model and an optional key.
    pub(crate) fn agent_dialog(&mut self, ctx: &Context, form: &mut Form) -> bool {
        if form.timeout_s == 0 {
            form.timeout_s = agents::DEFAULT_REQUEST_TIMEOUT_S;
        }
        let mut done = false;
        let title = if form.editing.is_some() { "Edit agent" } else { "Add agent" };
        let closing = shell::dialog(ctx, "assistant_agent_dialog", title, |ui| {
            if !pcbcraft_chrome::automation::human_input(ui.ctx()) {
                ui.disable();
            }
            let t = Tokens::get();
            if form.editing.is_none() {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut form.kind, Kind::Acp, "Agent command");
                    ui.selectable_value(&mut form.kind, Kind::Api, "OpenAI-compatible endpoint");
                    ui.selectable_value(&mut form.kind, Kind::Anthropic, "Anthropic-compatible endpoint");
                });
                if let Some(protocol) = form.kind.protocol() {
                    ui.horizontal_wrapped(|ui| {
                        for preset in agents::presets(protocol) {
                            if pcbcraft_chrome::controls::small_button(ui, preset.name).on_hover_text(preset.base_url).clicked() {
                                form.name = preset.name.to_owned();
                                form.base_url = preset.base_url.to_owned();
                            }
                        }
                    });
                }
                ui.separator();
            }
            let field_width = (ui.available_width() - t.space_24 * 4.0).clamp(160.0, 320.0);
            egui::Grid::new("assistant_agent_form").num_columns(2).spacing([t.space_12, t.space_8]).show(ui, |ui| {
                let name_label = ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut form.name).char_limit(64).desired_width(field_width).hint_text("My agent"))
                    .labelled_by(name_label.id);
                ui.end_row();
                if form.kind == Kind::Acp {
                    let command_label = ui.label("Command");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.command).char_limit(MAX_COMMAND).desired_width(field_width).hint_text("opencode acp"),
                    )
                    .labelled_by(command_label.id);
                    ui.end_row();
                } else {
                    let url_label = ui.label("Base URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.base_url)
                            .char_limit(2048)
                            .desired_width(field_width)
                            .hint_text("http://localhost:11434/v1"),
                    )
                    .labelled_by(url_label.id);
                    ui.end_row();
                    let model_label = ui.label("Model");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.model)
                            .char_limit(craft_agents::MODEL_ID_CAP)
                            .desired_width(field_width)
                            .hint_text("qwen3"),
                    )
                    .labelled_by(model_label.id);
                    ui.end_row();
                    let key_label = ui.label("Key");
                    let hint = if form.key_stored && !form.forget_key { "Stored in the keychain" } else { "Optional" };
                    ui.add(egui::TextEdit::singleline(&mut form.key).password(true).char_limit(4096).desired_width(field_width).hint_text(hint))
                        .labelled_by(key_label.id);
                    ui.end_row();
                    let timeout_label = ui.label("Request timeout (s)");
                    ui.add(egui::DragValue::new(&mut form.timeout_s).range(agents::MIN_REQUEST_TIMEOUT_S..=agents::MAX_REQUEST_TIMEOUT_S))
                        .labelled_by(timeout_label.id);
                    ui.end_row();
                    let effort_label = ui.label("Reasoning effort");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.effort)
                            .char_limit(agents::MAX_REASONING_EFFORT_LEN)
                            .desired_width(field_width)
                            .hint_text("Provider default"),
                    )
                    .labelled_by(effort_label.id);
                    ui.end_row();
                }
            });
            if form.kind == Kind::Acp {
                ui.label(dim("A program that speaks the Agent Client Protocol, with its arguments. It runs without a shell."));
            } else {
                ui.label(dim(
                    "PCBCraft runs the conversation itself and gives the model the project tools. A key is kept in the system keychain and only sent over HTTPS or to this computer.",
                ));
                if form.key_stored {
                    pcbcraft_chrome::controls::checkbox(ui, &mut form.forget_key, "Remove the stored key");
                }
            }
            if let Some(error) = &form.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                if pcbcraft_chrome::controls::button(ui, "Cancel").clicked() {
                    done = true;
                } else if pcbcraft_chrome::controls::button(ui, if form.editing.is_some() { "Save" } else { "Add" }).clicked()
                    && pcbcraft_chrome::automation::human_input(ui.ctx())
                {
                    match self.save_agent(form) {
                        Ok(()) => done = true,
                        Err(error) => form.error = Some(error),
                    }
                }
            });
        });
        closing || done
    }

    /// Store the agent under a stable id, and an endpoint's new key in the keychain. Changing
    /// what an agent starts or calls disconnects it if connected, and drops its connection test
    /// and result, which were for the old settings; its name, request timeout and reasoning effort
    /// apply to a connected one at once. Nothing is saved on an error.
    fn save_agent(&mut self, form: &mut Form) -> Result<(), String> {
        let name = form.name.trim().to_owned();
        if name.is_empty() {
            return Err("Enter a name for this agent.".to_owned());
        }
        if self.entries().iter().any(|entry| Some(&entry.id) != form.editing.as_ref() && entry.name.eq_ignore_ascii_case(&name)) {
            return Err("An agent already uses this name.".to_owned());
        }
        let mut agent = AgentConfig { name, kind: form.kind, ..AgentConfig::default() };
        if form.kind == Kind::Acp {
            agent.command = form.command.trim().to_owned();
            if !valid_command(&agent.command) {
                return Err("Enter the command that starts the agent.".to_owned());
            }
        } else {
            agent.base_url = agents::normalize_url(&form.base_url)?;
            agent.model = form.model.trim().to_owned();
            if !valid_model(&agent.model) {
                return Err("Enter the model name.".to_owned());
            }
            agent.request_timeout_s = form.timeout_s.clamp(agents::MIN_REQUEST_TIMEOUT_S, agents::MAX_REQUEST_TIMEOUT_S);
            agent.reasoning_effort = agents::reasoning_effort(&form.effort);
            agent.key = (form.key_stored && !form.forget_key) || !form.key.trim().is_empty();
            if agent.key && !agents::key_safe_for(&agent.base_url) {
                return Err("A key is only sent over HTTPS, or to this computer.".to_owned());
            }
        }
        if form.editing.is_none() && self.settings.agents.agents.len() >= MAX_AGENTS {
            return Err("The agent list is full.".to_owned());
        }
        // A stored key goes only where it was entered for: a new address needs the key again.
        let moved = form.editing.as_ref().and_then(|id| self.settings.agents.agents.iter().find(|existing| existing.id == *id));
        if agent.key && form.key.trim().is_empty() && moved.is_some_and(|existing| existing.key && existing.base_url != agent.base_url) {
            return Err("The address changed: enter the key again for it, or remove the key.".to_owned());
        }
        let prefix = if form.kind == Kind::Acp { "custom" } else { "api" };
        agent.id = form.editing.clone().unwrap_or_else(|| agents::new_id(&self.settings.agents, prefix));
        let key = std::mem::take(&mut form.key);
        let key = key.trim();
        if form.kind != Kind::Acp && !key.is_empty() {
            super::host::store_endpoint_key(&agent.id, &agent.base_url, key)?;
        } else if form.forget_key {
            super::host::Keychain.delete(&agent.id).map_err(|error| format!("Could not remove the key from the keychain: {error}"))?;
        }
        let connected = self.chat.agent.as_ref().is_some_and(|connected| connected.id == agent.id);
        let prefs = &mut self.settings.agents;
        match prefs.agents.iter_mut().find(|existing| existing.id == agent.id) {
            Some(existing) => {
                let restart = existing.command != agent.command
                    || existing.base_url != agent.base_url
                    || existing.model != agent.model
                    || existing.key != agent.key
                    || !key.is_empty();
                *existing = agent.clone();
                if restart {
                    self.tests.retain(|(test, _)| *test != agent.id);
                    self.test_results.remove(&agent.id);
                }
                if restart && connected {
                    self.chat.disconnect();
                } else if connected {
                    self.chat.retune(&agent);
                }
            }
            None => prefs.agents.push(agent),
        }
        self.settings_changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw synthetic pointer/keyboard input cannot broaden the global mode or bridge policy;
    /// genuine user input keeps the existing settings behavior.
    #[test]
    fn synthetic_input_cannot_change_global_permission_policy() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut assistant = Assistant { detection: Some(Default::default()), ..Default::default() };
        assistant.settings.agents.auto_approve_bridge = false;
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(
            |ui, a: &mut Assistant| {
                a.settings_page(ui);
            },
            assistant,
        );
        h.run();
        for label in [Mode::All.label(), "Approve agents' requests to use PCBCraft's tools automatically"] {
            let at = h.get_by_label(label).rect().center();
            pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
            h.input_mut().events.extend([
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ]);
            h.step();
            assert_eq!(h.state().settings.agents.ai_permissions, Mode::Edits);
            assert!(!h.state().settings.agents.auto_approve_bridge);
            pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, false);
            h.run();
        }
        h.get_by_label(Mode::All.label()).focus();
        h.run();
        pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().settings.agents.ai_permissions, Mode::Edits);
        pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, false);
        h.run();
        h.get_by_label(Mode::All.label()).click();
        h.run();
        assert_eq!(h.state().settings.agents.ai_permissions, Mode::All);
    }

    /// A permission popup opened by a person cannot apply a synthetic selection, including
    /// when its rows are rendered independently of the surrounding settings page.
    #[test]
    fn synthetic_input_cannot_change_an_open_agent_permission_popup() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut assistant = Assistant { detection: Some(Default::default()), ..Default::default() };
        assistant.settings.agents.agents.push(AgentConfig {
            id: "fixture".into(),
            name: "Fixture".into(),
            command: "fake".into(),
            ..Default::default()
        });
        assistant.settings.agents.agent_permissions.insert("fixture".into(), Mode::Ask);
        let entry = assistant.entries().into_iter().find(|a| a.id == "fixture").unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
            move |ui, a: &mut Assistant| {
                a.agent_row(ui, &entry, &mut None);
            },
            assistant,
        );
        h.run();
        h.get_by_label("Agent permission mode").click();
        h.run();
        let at = h.get_by_label(Mode::All.label()).rect().center();
        pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
        h.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        h.step();
        assert_eq!(h.state().settings.agents.agent_permissions.get("fixture"), Some(&Mode::Ask));
        pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, false);
        h.run();
        if h.query_by_label(Mode::All.label()).is_none() {
            h.get_by_label("Agent permission mode").click();
            h.run();
        }
        h.get_by_label(Mode::All.label()).click();
        h.run();
        assert_eq!(h.state().settings.agents.agent_permissions.get("fixture"), Some(&Mode::All));
    }

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
            {"id":"custom-3","name":"Bad","kind":"acp","command":"a\u0007b"},
            {"id":"api-2","name":"No model","kind":"api","base_url":"http://localhost:1234/v1","model":""},
            {"id":"api-3","name":"Bad URL","kind":"anthropic","base_url":"ftp://x","model":"m"}
        ],"ai_permissions":"ask"}}"#;
        let settings = Settings::load(Some(saved));
        assert_eq!(settings.agents.agents.iter().map(|agent| agent.id.as_str()).collect::<Vec<_>>(), ["custom-1", "api-1"]);
        assert_eq!(settings.agents.ai_permissions, Mode::Ask);
        assert!(settings.agents.auto_approve_bridge, "settings saved before the choice existed keep approving");
    }

    /// The choice to approve requests for PCBCraft's tools is the person's, and it is kept.
    #[test]
    fn the_auto_approval_choice_is_kept() -> Result<(), serde_json::Error> {
        let mut settings = Settings::load(None);
        assert!(settings.agents.auto_approve_bridge, "on by default, as in SolidCraft");
        settings.agents.auto_approve_bridge = false;
        let saved = serde_json::to_string(&settings)?;
        assert!(!Settings::load(Some(&saved)).agents.auto_approve_bridge);
        Ok(())
    }

    /// An endpoint saves without a key; a key for a plain-HTTP remote endpoint is refused before
    /// the keychain is touched, and nothing is saved.
    #[test]
    fn endpoint_keys_only_go_over_https_or_to_this_computer() {
        let mut assistant = Assistant::default();
        let mut form = Form {
            kind: Kind::Api,
            name: "Local".to_owned(),
            base_url: "http://localhost:11434/v1/".to_owned(),
            model: "qwen3".to_owned(),
            timeout_s: 120,
            ..Form::default()
        };
        assert_eq!(assistant.save_agent(&mut form), Ok(()));
        let saved = &assistant.settings.agents.agents[0];
        assert_eq!((saved.id.as_str(), saved.base_url.as_str(), saved.key), ("api-1", "http://localhost:11434/v1", false));

        let mut remote = Form {
            kind: Kind::Api,
            name: "Remote".to_owned(),
            base_url: "http://example.com/v1".to_owned(),
            model: "m".to_owned(),
            key: "secret".to_owned(),
            ..Form::default()
        };
        assert_eq!(assistant.save_agent(&mut remote), Err("A key is only sent over HTTPS, or to this computer.".to_owned()));
        assert_eq!(assistant.settings.agents.agents.len(), 1, "nothing was saved");
        let mut missing =
            Form { kind: Kind::Anthropic, name: "No model".to_owned(), base_url: "https://api.anthropic.com/v1".to_owned(), ..Form::default() };
        assert!(assistant.save_agent(&mut missing).is_err());

        // A keyed endpoint moved to another address must be given its key again (or lose it);
        // the stored key is never carried to the new address. Refused before the keychain.
        if let Some(saved) = assistant.settings.agents.agents.first_mut() {
            saved.key = true;
        }
        let mut moved = Form {
            editing: Some("api-1".to_owned()),
            kind: Kind::Api,
            name: "Local".to_owned(),
            base_url: "https://other.example/v1".to_owned(),
            model: "qwen3".to_owned(),
            key_stored: true,
            ..Form::default()
        };
        assert_eq!(assistant.save_agent(&mut moved), Err("The address changed: enter the key again for it, or remove the key.".to_owned()));
        assert_eq!(assistant.settings.agents.agents[0].base_url, "http://localhost:11434/v1", "nothing was saved");
    }

    /// A chat backend that records what it is told, for settings that reach a connected agent.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl craft_agents::backend::ChatBackend for Recorder {
        /// Nothing arrives.
        fn poll(&mut self) -> Vec<craft_agents::backend::Event> {
            Vec::new()
        }
        fn prompt(&mut self, _parts: &[craft_agents::backend::PromptPart]) {}
        fn cancel(&mut self) {}
        fn new_session(&mut self) {}
        fn answer_permission(&mut self, _key: u64, _option: Option<&str>) {}
        fn authenticate(&mut self, _method: &str) {}
        /// Record the new timeout.
        fn set_request_timeout(&mut self, secs: u64) {
            if let Ok(mut seen) = self.0.lock() {
                seen.push(format!("timeout {secs}"));
            }
        }
        /// Record the new effort.
        fn set_reasoning_effort(&mut self, effort: &str) {
            if let Ok(mut seen) = self.0.lock() {
                seen.push(format!("effort {effort}"));
            }
        }
        /// Record that it was ended.
        fn shutdown(&mut self, _wait: bool) {
            if let Ok(mut seen) = self.0.lock() {
                seen.push("shutdown".to_owned());
            }
        }
    }

    /// An endpoint saved and connected to a recording backend: the assistant and what it records.
    fn connected_endpoint() -> (Assistant, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let mut assistant = Assistant::default();
        let mut form = Form {
            kind: Kind::Api,
            name: "Local".to_owned(),
            base_url: "http://localhost:11434/v1".to_owned(),
            model: "qwen3".to_owned(),
            timeout_s: 120,
            ..Form::default()
        };
        assert_eq!(assistant.save_agent(&mut form), Ok(()));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let entry = assistant.entries().into_iter().find(|entry| entry.id == "api-1").unwrap_or_else(|| panic!("saved"));
        assistant.chat.attach(Box::new(Recorder(seen.clone())), entry);
        (assistant, seen)
    }

    /// The connected agent takes a new name, timeout and effort at once, without a restart;
    /// a new model restarts it.
    #[test]
    fn saving_a_connected_endpoint_applies_or_restarts() {
        let (mut assistant, seen) = connected_endpoint();
        let mut form = Form {
            editing: Some("api-1".to_owned()),
            kind: Kind::Api,
            name: "Local fast".to_owned(),
            base_url: "http://localhost:11434/v1".to_owned(),
            model: "qwen3".to_owned(),
            timeout_s: 300,
            effort: "high".to_owned(),
            ..Form::default()
        };
        assert_eq!(assistant.save_agent(&mut form), Ok(()));
        assert!(assistant.chat.connected());
        assert_eq!(seen.lock().map(|seen| seen.clone()).unwrap_or_default(), ["timeout 300", "effort high"]);
        let entry = assistant.chat.agent.as_ref().unwrap_or_else(|| panic!("connected"));
        assert_eq!((entry.name.as_str(), entry.request_timeout_s, entry.reasoning_effort.as_str()), ("Local fast", 300, "high"));

        // A test result, and a test still running, were for the old model: both go.
        assistant.test_results.insert("api-1".to_owned(), Ok("Answers: Local fast".to_owned()));
        let (finish, running) = std::sync::mpsc::sync_channel(1);
        assistant.tests.push(("api-1".to_owned(), running));
        let mut form = Form { model: "qwen3-large".to_owned(), ..form };
        assert_eq!(assistant.save_agent(&mut form), Ok(()));
        assert!(!assistant.chat.connected(), "a new model restarts the agent");
        let _ = finish.send(Ok("Answers: Local fast".to_owned()));
        assistant.poll_jobs();
        assert!(assistant.tests.is_empty() && !assistant.test_results.contains_key("api-1"));
    }

    /// Removing the connected agent ends its chat. A key the keychain won't give up keeps the
    /// agent, and its row says why.
    #[test]
    fn removing_an_agent_ends_its_chat_or_keeps_it_when_its_key_stays() {
        let (mut assistant, seen) = connected_endpoint();
        if let Some(agent) = assistant.settings.agents.agents.first_mut() {
            agent.key = true;
        }
        // A connection test still running when the removal fails can't hide why the agent stayed.
        let (finish, running) = std::sync::mpsc::sync_channel(1);
        assistant.tests.push(("api-1".to_owned(), running));
        assistant.remove_agent_with("api-1", |_| Err("locked".to_owned()));
        let _ = finish.send(Ok("Answers: Local".to_owned()));
        assistant.poll_jobs();
        assert_eq!(assistant.settings.agents.agents.len(), 1, "the agent stays");
        assert!(assistant.chat.connected());
        assert!(matches!(assistant.test_results.get("api-1"), Some(Err(error)) if error.contains("locked")));

        assistant.remove_agent_with("api-1", |_| Ok(()));
        assert!(assistant.settings.agents.agents.is_empty());
        assert!(!assistant.chat.connected());
        assert_eq!(seen.lock().map(|seen| seen.clone()).unwrap_or_default(), ["shutdown"]);
    }

    /// A typed key never shows in debug output.
    #[test]
    fn a_typed_key_stays_out_of_debug_output() {
        let form = Form { key: "sk-secret".to_owned(), name: "Remote".to_owned(), ..Form::default() };
        assert!(!format!("{form:?}").contains("sk-secret"));
    }
}
