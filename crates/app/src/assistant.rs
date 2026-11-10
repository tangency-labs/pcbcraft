//! The in-app Assistant: a chat with a coding agent the person already has (Claude Code, Codex,
//! OpenCode and others that speak the Agent Client Protocol), as in SolidCraft's agent chat. The
//! agent reaches the open project through its own live MCP endpoint, so it runs the same commands
//! as the menus and any MCP client, sharing undo; the permission gate decides which of them wait
//! for the person. Agents come from `craft-agents` (see `ATTRIBUTION.md`).

mod chat;
mod host;
mod settings;

pub(crate) use chat::{Answer, Approval, Source, Verdict, gate};
pub(super) use settings::{Form as AgentForm, Request as SettingsRequest};

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
};

use craft_agents::{
    agents::{self, Entry, Mode},
    backend::TestError,
    discover::Detection,
    markdown::{self, Block, Span},
};
use egui::{Align, Context, Layout, Rect, RichText, ScrollArea, TextEdit, Ui};
use pcbcraft_chrome::{Tokens, assistant as chrome, shell::dim};
use pcbcraft_engine::{Project, Session};
use serde::{Deserialize, Serialize};

use chat::{Chat, Item, State};
use settings::Settings;

pub(super) const TOGGLE: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::A);
const NEW_CHAT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::L);
/// Longest tool input or output shown on its card.
const MAX_TOOL_DETAIL: usize = 2000;

#[derive(Debug, Default, PartialEq, Eq)]
enum Visibility {
    #[default]
    Hidden,
    Minimized,
    Open,
}

#[derive(Debug, Default)]
pub(super) struct Assistant {
    visibility: Visibility,
    dock: chrome::Dock,
    popped: bool,
    window_placed: bool,
    window_place: Option<(egui::Pos2, egui::Vec2)>,
    panel_rect: Option<Rect>,
    keyboard_active: bool,
    confirm_new: bool,
    settings: Settings,
    settings_request: Option<SettingsRequest>,
    settings_changed: bool,
    connect_on_first_frame: bool,
    project: Option<Arc<Project>>,
    draft: String,
    chat: Chat,
    /// Installed agents found by the last search, and the search in progress.
    detection: Option<Detection>,
    detecting: Option<Receiver<Detection>>,
    /// Sign-in results seen when agents connected, by agent id.
    login_results: HashMap<String, bool>,
    /// Connection tests running, by agent id, and their results.
    tests: Vec<(String, Receiver<Result<String, TestError>>)>,
    test_results: BTreeMap<String, Result<String, String>>,
    /// Held tool calls the person answered this frame, for the editor to run or refuse.
    answers: Vec<(Approval, Answer)>,
    /// The saved model was sent to this connection's agent.
    model_restored: bool,
}

#[derive(Deserialize, Serialize)]
struct WindowSetting {
    left: bool,
    width: Option<f32>,
    popped: bool,
    place: Option<[f32; 4]>,
}

impl WindowSetting {
    /// Reject invalid geometry while allowing monitors left of or above the main display.
    fn usable_place([x, y, width, height]: [f32; 4]) -> Option<(egui::Pos2, egui::Vec2)> {
        let usable = x.abs() <= 100_000.0 && y.abs() <= 100_000.0 && [width, height].iter().all(|v| (320.0..=10_000.0).contains(v));
        usable.then_some((egui::pos2(x, y), egui::vec2(width, height)))
    }
}

impl Assistant {
    /// Restore the saved AI settings and window layout, including startup preferences.
    pub(super) fn from_setting(layout: Option<&str>, settings: Option<&str>) -> Self {
        let settings = Settings::load(settings);
        let mut assistant = Self::default();
        if settings.show_features && settings.open_at_start {
            assistant.visibility = Visibility::Open;
        }
        assistant.connect_on_first_frame = settings.show_features && settings.connect_at_start;
        assistant.settings = settings;
        if let Some(layout) = layout.and_then(|text| serde_json::from_str::<WindowSetting>(text).ok()) {
            assistant.dock = chrome::Dock { left: layout.left, width: layout.width.filter(|width| (280.0..=572.0).contains(width)) };
            assistant.popped = layout.popped;
            assistant.window_place = layout.place.and_then(WindowSetting::usable_place);
        }
        assistant
    }

    /// Store the dock edge, width, and detached window position with the editor preferences.
    pub(super) fn layout_setting(&self) -> String {
        let place = self.window_place.map(|(position, size)| [position.x, position.y, size.x, size.y]);
        serde_json::to_string(&WindowSetting { left: self.dock.left, width: self.dock.width, popped: self.popped, place }).unwrap_or_default()
    }

    /// Serialize the AI settings and added agents without any credentials.
    pub(super) fn settings_text(&self) -> String {
        serde_json::to_string(&self.settings).unwrap_or_default()
    }

    /// Tell the editor to persist changes after this frame.
    pub(super) fn take_settings_changed(&mut self) -> bool {
        std::mem::take(&mut self.settings_changed)
    }

    /// Transfer a settings shortcut from the chat to the editor's modal.
    pub(super) fn take_settings_request(&mut self) -> Option<SettingsRequest> {
        self.settings_request.take()
    }

    /// Whether the editor should expose Assistant entry points.
    pub(super) fn enabled(&self) -> bool {
        self.settings.show_features
    }

    /// Whether the app bar should include the Assistant button.
    pub(super) fn toolbar_visible(&self) -> bool {
        self.enabled() && self.settings.toolbar
    }

    /// Report whether the view is open and whether an agent is connected and answering.
    pub(super) fn toolbar_state(&self) -> (bool, bool) {
        (self.visibility == Visibility::Open, matches!(self.chat.state, State::Ready | State::Running))
    }

    /// Handle chat shortcuts and return whether the conversation owns the editing keys.
    pub(super) fn shortcuts(&mut self, ctx: &Context) -> bool {
        if ctx.input(|input| input.pointer.any_pressed()) {
            self.keyboard_active = self.panel_rect.zip(ctx.pointer_interact_pos()).is_some_and(|(rect, point)| rect.contains(point));
        }
        let owns_keys = self.keyboard_active || ctx.memory(|memory| memory.has_focus(egui::Id::new("assistant_draft_input")));
        if owns_keys {
            self.chat_shortcuts(ctx);
        }
        owns_keys
    }

    /// The card covering this workspace, excluding a detached window or hidden feature.
    /// Uses current geometry so a resize does not leave Fit targeting last frame's bounds.
    pub(super) fn covered_rect(&self, workspace: Rect) -> Rect {
        match self.visibility {
            _ if !self.settings.show_features => Rect::NOTHING,
            Visibility::Minimized => chrome::entry_bounds(workspace),
            Visibility::Open if !self.popped => chrome::drawer_bounds(workspace, self.dock),
            _ => Rect::NOTHING,
        }
    }

    /// Open the last-used view, or hide it if currently open.
    pub(super) fn toggle(&mut self, ctx: &Context) {
        if self.visibility == Visibility::Open {
            self.hide(ctx, Visibility::Hidden);
        } else {
            self.open(ctx);
        }
        ctx.request_repaint();
    }

    /// Show the current conversation on return to the workspace.
    pub(super) fn open(&mut self, ctx: &Context) {
        self.visibility = Visibility::Open;
        ctx.request_repaint();
    }

    /// Release conversation focus and keep the agent available when hiding the view.
    fn hide(&mut self, ctx: &Context, visibility: Visibility) {
        self.visibility = visibility;
        self.window_placed = false;
        self.confirm_new = false;
        self.release_focus(ctx);
    }

    /// Drop the conversation's keyboard ownership on a window transition.
    fn release_focus(&mut self, ctx: &Context) {
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused()
                && (self.keyboard_active || id == egui::Id::new("assistant_draft_input"))
            {
                memory.surrender_focus(id);
            }
        });
        self.keyboard_active = false;
        self.panel_rect = None;
    }

    /// Advance the agent and render the conversation. Home passes no workspace and suspends its
    /// view. Held tool calls the person answers are handed to the editor through
    /// [`Self::take_answers`]; the conversation itself never edits the session.
    /// `has_document` also admits independent library workspaces without a project.
    pub(super) fn show(&mut self, ctx: &Context, workspace: Option<Rect>, session: &Session, can_edit: bool, has_document: bool) {
        self.sync_project(session);
        if !self.settings.show_features {
            return;
        }
        if self.visibility != Visibility::Hidden || self.connect_on_first_frame {
            self.ensure_detection(ctx);
        }
        self.poll_jobs();
        self.chat.poll(self.mode(), self.settings.agents.auto_approve_bridge);
        if let Some(due) = self.chat.next_expiry() {
            ctx.request_repaint_after(due);
        }
        self.restore_model();
        if self.connect_on_first_frame && self.detection.is_some() {
            self.connect_on_first_frame = false;
            self.connect(ctx);
        }
        let Some(workspace) = workspace else {
            self.release_focus(ctx);
            self.window_placed = false;
            return;
        };
        let action = match self.visibility {
            Visibility::Hidden => None,
            Visibility::Minimized => {
                if chrome::minimized(ctx, workspace) {
                    self.visibility = Visibility::Open;
                    ctx.request_repaint();
                }
                None
            }
            Visibility::Open if self.popped => self.show_window(ctx, session, can_edit, has_document),
            Visibility::Open => {
                let mut dock = self.dock;
                let ready = matches!(self.chat.state, State::Ready | State::Running);
                let (action, rect) = chrome::drawer(ctx, workspace, &mut dock, self.connection_status(), ready, |ui| {
                    self.panel(ui, session, can_edit, has_document);
                });
                self.dock = dock;
                self.panel_rect = Some(rect);
                action
            }
        };
        if let Some(action) = action {
            self.window_action(ctx, action);
            ctx.request_repaint();
        }
    }

    /// Move the conversation into a native viewport and remember where it was on this run.
    fn show_window(&mut self, ctx: &Context, session: &Session, can_edit: bool, has_document: bool) -> Option<chrome::Action> {
        let title = session.project().map_or_else(|| "Assistant".to_owned(), |p| format!("{} — Assistant", p.name));
        let mut builder = egui::ViewportBuilder::default().with_title(title).with_min_inner_size([320.0, 320.0]);
        if !self.window_placed {
            builder = match self.window_place {
                Some((position, size)) => builder.with_position(position).with_inner_size(size),
                None => builder.with_inner_size([460.0, 680.0]),
            };
        }
        self.window_placed = true;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("pcbcraft_assistant"), builder, |ui, class| {
            if class != egui::ViewportClass::EmbeddedWindow {
                let (outer, inner, close) = ui.ctx().input(|input| {
                    let viewport = input.viewport();
                    (viewport.outer_rect, viewport.inner_rect, viewport.close_requested())
                });
                if let Some((outer, inner)) = outer.zip(inner)
                    && inner.width() >= 320.0
                    && inner.height() >= 320.0
                {
                    self.window_place = Some((outer.min, inner.size()));
                }
                if close || ui.ctx().input_mut(|input| input.consume_shortcut(&TOGGLE)) {
                    return Some(chrome::Action::Close);
                }
                self.chat_shortcuts(ui.ctx());
            } else {
                self.panel_rect = Some(ui.max_rect());
            }
            let status = self.connection_status();
            let ready = matches!(self.chat.state, State::Ready | State::Running);
            chrome::window(ui, status, ready, |ui| self.panel(ui, session, can_edit, has_document))
        })
    }

    /// Keep new-chat and stop shortcuts inside the conversation when it owns the keyboard.
    fn chat_shortcuts(&mut self, ctx: &Context) {
        if ctx.input_mut(|input| input.consume_shortcut(&NEW_CHAT)) {
            self.request_new_chat();
        }
        if self.chat.state == State::Running && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.chat.stop();
        }
    }

    /// Keep chat and agent state when moving, folding, or closing the view.
    fn window_action(&mut self, ctx: &Context, action: chrome::Action) {
        match action {
            chrome::Action::Close => self.hide(ctx, Visibility::Hidden),
            chrome::Action::Minimize => self.hide(ctx, Visibility::Minimized),
            chrome::Action::NewChat => self.request_new_chat(),
            chrome::Action::PopOut | chrome::Action::Dock => {
                self.release_focus(ctx);
                self.window_placed = false;
                self.popped = action == chrome::Action::PopOut;
            }
        }
    }

    /// Ask before clearing a conversation or draft.
    fn request_new_chat(&mut self) {
        self.confirm_new = !self.chat.items.is_empty() || !self.draft.is_empty();
        if !self.confirm_new {
            self.new_chat();
        }
    }

    /// Clear the conversation and draft; a connected agent starts a fresh session.
    fn new_chat(&mut self) {
        self.draft.clear();
        self.chat.new_conversation();
        self.confirm_new = false;
    }

    /// Start a fresh conversation on a project switch. The agent stays connected.
    fn sync_project(&mut self, session: &Session) {
        let changed = match (&self.project, session.project()) {
            (Some(previous), Some(current)) => !previous.same_document(current),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.project = session.snapshot();
            self.new_chat();
        }
    }

    /// Look for installed agents once the Assistant or its settings are used. Hidden, it starts
    /// no programs.
    fn ensure_detection(&mut self, ctx: &Context) {
        if self.detection.is_none() && self.detecting.is_none() {
            self.detect(ctx);
        }
    }

    /// Look for installed agents in the background. Detection never signs in or starts a session.
    fn detect(&mut self, ctx: &Context) {
        // Unit tests never probe the machine's installed agents: what they find, and when, would
        // vary by machine and load. `craft_agents::discover` tests detection on fake folders.
        if cfg!(test) {
            self.detection = Some(Detection::default());
            return;
        }
        let (send, receive) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        let started = std::thread::Builder::new().name("pcbcraft-agent-detect".to_owned()).spawn(move || {
            let _ = send.send(craft_agents::discover::detect());
            ctx.request_repaint();
        });
        if started.is_ok() {
            self.detecting = Some(receive);
        } else {
            self.detection = Some(Detection::default());
        }
    }

    /// Collect finished detection and connection tests.
    fn poll_jobs(&mut self) {
        if let Some(receive) = &self.detecting {
            match receive.try_recv() {
                Ok(detection) => {
                    self.detection = Some(detection);
                    self.detecting = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.detection.get_or_insert_with(Detection::default);
                    self.detecting = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let mut finished = Vec::new();
        self.tests.retain(|(id, receive)| match receive.try_recv() {
            Ok(result) => {
                finished.push((id.clone(), result));
                false
            }
            Err(TryRecvError::Disconnected) => {
                finished.push((id.clone(), Err(TestError::Other("The test stopped unexpectedly.".to_owned()))));
                false
            }
            Err(TryRecvError::Empty) => true,
        });
        for (id, result) in finished {
            if matches!(result, Err(TestError::AuthRequired)) {
                self.login_results.insert(id.clone(), false);
            } else if result.is_ok() {
                self.login_results.insert(id.clone(), true);
            }
            self.test_results.insert(id, result.map_err(|error| error.message().to_owned()));
        }
    }

    /// Check in the background that `entry` answers, without opening the chat: an ACP agent is
    /// started and asked to open a session; an endpoint is asked for its models and the model.
    fn test_agent(&mut self, ctx: &Context, entry: &Entry) {
        let (send, receive) = mpsc::sync_channel(1);
        let id = entry.id.clone();
        let entry = entry.clone();
        let ctx = ctx.clone();
        let started = std::thread::Builder::new().name("pcbcraft-agent-test".to_owned()).spawn(move || {
            let result = match entry.kind.protocol() {
                None => tempfile::tempdir()
                    .map_err(|error| TestError::Other(format!("Could not make a working folder: {error}")))
                    .and_then(|dir| craft_agents::acp_client::test_agent(&entry.argv, dir.path())),
                Some(protocol) => test_endpoint(&entry, protocol).map_err(TestError::Other),
            };
            let _ = send.send(result);
            ctx.request_repaint();
        });
        match started {
            Ok(_) => {
                self.test_results.remove(&id);
                self.tests.push((id.clone(), receive));
            }
            Err(_) => {
                self.test_results.insert(id.clone(), Err("Could not start the test.".to_owned()));
            }
        }
    }

    /// Every agent the chat can use: the installed ones found, then the ones the person added.
    fn entries(&self) -> Vec<Entry> {
        let found = self.detection.as_ref().map_or(&[][..], |detection| detection.found.as_slice());
        agents::entries(&self.settings.agents, found, &self.login_results)
    }

    /// The agent the next connection uses: the one picked, else the default, else the first.
    fn current(&self) -> Option<Entry> {
        agents::current(&self.settings.agents, &self.entries(), self.settings.picked.as_deref())
    }

    /// The permission mode for the connected (or picked) agent.
    pub(super) fn mode(&self) -> Mode {
        let id = self.chat.agent.as_ref().map(|agent| agent.id.clone()).or_else(|| self.current().map(|agent| agent.id));
        agents::mode_for(&self.settings.agents, id.as_deref().unwrap_or_default())
    }

    /// Whether the person always allows `command` for the connected agent.
    pub(super) fn always_allowed(&self, command: &str) -> bool {
        self.chat.agent.as_ref().is_some_and(|agent| agents::always_allowed(&self.settings.agents, &agent.id, command))
    }

    /// Remember an "always allow" answer for the connected agent.
    fn remember(&mut self, command: &str) {
        if let Some(agent) = self.chat.agent.as_ref().map(|agent| agent.id.clone()) {
            agents::remember(&mut self.settings.agents, &agent, command);
            self.settings_changed = true;
        }
    }

    /// Connect the current agent.
    fn connect(&mut self, ctx: &Context) {
        match self.current() {
            Some(entry) => {
                self.model_restored = false;
                self.chat.connect(ctx, entry);
            }
            None => self.chat.push(Item::Notice("No agent is installed. Preferences › AI lists the ones you can install.".to_owned())),
        }
    }

    /// Send the model last chosen for this agent once it lists its models. An endpoint is Ready
    /// before its listing arrives (that takes a request), so this waits for a model option or
    /// list, trying again each frame, and only then counts as done.
    fn restore_model(&mut self) {
        let listed = self.chat.models.is_some() || self.chat.config.iter().any(|option| option.category == "model");
        if self.model_restored || self.chat.state != State::Ready || !listed {
            return;
        }
        self.model_restored = true;
        let Some(agent) = self.chat.agent.as_ref() else { return };
        let Some(saved) = self.settings.agents.agent_models.get(&agent.id).cloned() else { return };
        if let Some(option) = self.chat.config.iter().find(|option| option.category == "model").cloned() {
            if option.current_value != saved && option.options.iter().any(|choice| choice.value == saved) {
                self.chat.set_option(&option.id, &saved);
            }
        } else if let Some((current, models)) = &self.chat.models
            && *current != saved
            && models.iter().any(|model| model.id == saved)
        {
            self.chat.set_model(&saved);
        }
    }

    /// Remember the model chosen for the connected agent, for its next connection.
    fn remember_model(&mut self, model: &str) {
        if let Some(agent) = self.chat.agent.as_ref().map(|agent| agent.id.clone()) {
            self.settings.agents.agent_models.insert(agent, model.to_owned());
            self.settings_changed = true;
        }
    }

    /// Hold a tool call from the chat's endpoint for the person's answer.
    pub(super) fn hold(&mut self, request: crate::live_control::Request, command: String, detail: String) {
        self.chat.hold(request, command, detail);
        if self.visibility != Visibility::Open {
            self.visibility = Visibility::Open;
        }
    }

    /// The chat endpoint's next request, for the editor to gate and run.
    pub(super) fn take_request(&mut self) -> Option<crate::live_control::Request> {
        self.chat.take_request()
    }

    /// Whether the chat is answering a message, so its tool calls may run.
    pub(crate) fn turn_open(&self) -> bool {
        self.chat.turn_open()
    }

    /// The chat endpoint's current request generation, if it is running.
    pub(crate) fn generation(&self) -> Option<u64> {
        self.chat.generation()
    }

    /// The approvals the person answered since the last frame, each once, for the editor to run
    /// or refuse. "Always allow" is remembered when the person answers.
    pub(super) fn take_answers(&mut self) -> Vec<(Approval, Answer)> {
        std::mem::take(&mut self.answers)
    }

    /// Apply the person's answer to an approval: agent questions go back to the agent, held tool
    /// calls go to the editor.
    fn answer(&mut self, id: u64, answer: Answer) {
        let Some(approval) = self.chat.take_approval(id) else { return };
        match &approval.source {
            chat::Source::Agent { key, .. } => {
                let option = match &answer {
                    Answer::Option(option) => Some(option.as_str()),
                    _ => None,
                };
                self.chat.answer_agent(*key, option);
            }
            chat::Source::Tool { command, .. } => {
                if answer == Answer::AlwaysAllow {
                    let command = command.clone();
                    self.remember(&command);
                }
                self.answers.push((approval, answer));
            }
        }
    }

    /// Note an edit the agent made, in the conversation.
    pub(super) fn note(&mut self, text: String) {
        self.chat.push(Item::Notice(text));
    }

    /// Draw the agent row, conversation and questions for the active document context.
    /// Document availability is separate from temporary edit refusals.
    fn panel(&mut self, ui: &mut Ui, session: &Session, can_edit: bool, has_document: bool) {
        let t = Tokens::get();
        if self.confirm_new {
            ui.label("Start a new chat? This clears the conversation and draft.");
            ui.horizontal(|ui| {
                if ui.button("Start new chat").clicked() {
                    self.new_chat();
                } else if ui.button("Keep chat").clicked() {
                    self.confirm_new = false;
                }
            });
            return;
        }
        self.agent_controls(ui);
        self.option_pickers(ui);
        if self.chat.items.is_empty() {
            ui.add_space(t.space_8);
            ui.label(dim("The agent works on open documents through the same commands as the menus. Edits are undoable."));
        }
        chrome::composer(ui, |ui| self.composer(ui, session, can_edit, has_document));
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            self.approvals(ui);
            ui.separator();
            ScrollArea::vertical().id_salt("assistant_messages").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical(|ui| {
                    let agent = self.chat.agent.as_ref().map_or("Agent", |agent| agent.name.as_str()).to_owned();
                    for item in &self.chat.items {
                        conversation_item(ui, item, &agent);
                        ui.add_space(t.space_6);
                    }
                });
            });
        });
    }

    /// Cards for the questions waiting for the person, with their answers.
    fn approvals(&mut self, ui: &mut Ui) {
        let mut answered = None;
        for approval in &self.chat.approvals {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(&approval.title);
                if !approval.detail.is_empty() {
                    ui.label(dim(&approval.detail));
                }
                ui.horizontal_wrapped(|ui| match &approval.source {
                    chat::Source::Tool { .. } => {
                        if ui.button("Allow").clicked() {
                            answered = Some((approval.id, Answer::Allow));
                        }
                        if ui.button("Always allow").on_hover_text("Allow this command for this agent from now on").clicked() {
                            answered = Some((approval.id, Answer::AlwaysAllow));
                        }
                        if ui.button("Deny").clicked() {
                            answered = Some((approval.id, Answer::Deny));
                        }
                    }
                    chat::Source::Agent { options, .. } => {
                        for option in options {
                            if ui.button(&option.name).clicked() {
                                answered = Some((approval.id, Answer::Option(option.id.clone())));
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            answered = Some((approval.id, Answer::Deny));
                        }
                    }
                });
            });
        }
        if let Some((id, answer)) = answered {
            self.answer(id, answer);
        }
    }

    /// The header's status text.
    fn connection_status(&self) -> &'static str {
        match self.chat.state {
            State::Disconnected => "Not connected",
            State::Connecting => "Connecting…",
            State::Ready => "Ready",
            State::Running => "Working…",
            State::SignIn(_) => "Sign-in required",
            State::Failed(_) => "Connection failed",
        }
    }

    /// Pick the agent, connect or disconnect it, and sign in when it asks.
    fn agent_controls(&mut self, ui: &mut Ui) {
        let entries = self.entries();
        let current = self.current();
        let label = current.as_ref().map_or_else(
            || if self.detecting.is_some() { "Looking for agents…".to_owned() } else { "No agent found".to_owned() },
            |entry| entry.name.clone(),
        );
        let mut picked = None;
        let missing: Vec<(&'static str, &'static str)> =
            self.detection.as_ref().map(|detection| detection.missing.iter().map(|agent| (agent.name, agent.install)).collect()).unwrap_or_default();
        chrome::agent_switcher(ui, &label, true, |ui| {
            for entry in &entries {
                let selected = current.as_ref().is_some_and(|current| current.id == entry.id);
                if ui.selectable_label(selected, &entry.name).on_hover_text(entry.detail()).clicked() {
                    picked = Some(entry.id.clone());
                }
            }
            for (name, install) in &missing {
                ui.add_enabled(false, egui::Button::selectable(false, format!("{name} (not installed)")))
                    .on_disabled_hover_text(format!("Install with: {install}"));
            }
            ui.separator();
            if ui.button("Add agent…").clicked() {
                self.settings_request = Some(SettingsRequest::Add(Box::default()));
                ui.close();
            }
            if ui.button("AI settings…").clicked() {
                self.settings_request = Some(SettingsRequest::Settings);
                ui.close();
            }
        });
        if let Some(id) = picked.filter(|id| current.as_ref().is_none_or(|current| &current.id != id)) {
            self.settings.picked = Some(id);
            self.settings_changed = true;
            if self.chat.connected() {
                self.chat.disconnect();
            }
        }
        ui.horizontal_wrapped(|ui| {
            if self.chat.connected() {
                if ui.button("Disconnect").clicked() {
                    self.chat.disconnect();
                }
            } else {
                let label = if matches!(self.chat.state, State::Failed(_)) { "Retry" } else { "Connect" };
                if ui.add_enabled(current.is_some(), egui::Button::new(label)).clicked() {
                    self.connect(ui.ctx());
                }
            }
            if self.chat.tools == Some(false) {
                ui.label(dim("No project tools"));
            }
        });
        match self.chat.state.clone() {
            State::Failed(error) => {
                ui.label(pcbcraft_chrome::shell::error(error));
            }
            State::SignIn(methods) => {
                ui.horizontal(|ui| {
                    ui.label("Sign in to continue.");
                    // A terminal or environment sign-in happens outside; the agent restarts to see it.
                    if ui.button("Retry").on_hover_text("Restart the agent after signing in elsewhere").clicked() {
                        self.connect(ui.ctx());
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for method in methods {
                        match &method.kind {
                            craft_agents::acp::AuthKind::Agent => {
                                if ui.button(&method.name).on_hover_text(&method.description).clicked() {
                                    self.chat.sign_in(&method.id);
                                }
                            }
                            craft_agents::acp::AuthKind::Terminal { .. } => {
                                ui.label(dim(format!("{}: sign in from a terminal, then Retry.", method.name)));
                            }
                            craft_agents::acp::AuthKind::EnvVar { vars } => {
                                ui.label(dim(format!("{}: set {} before starting PCBCraft.", method.name, vars.join(", "))));
                            }
                        }
                    }
                });
            }
            _ => {}
        }
    }

    /// The agent's own options (model, effort, mode) as it lists them, as SolidCraft's chat header
    /// shows them. A chosen model is remembered for the agent.
    fn option_pickers(&mut self, ui: &mut Ui) {
        let idle = self.chat.state == State::Ready;
        let mut chosen: Option<(String, String, bool)> = None;
        let mut model = None;
        let mut mode = None;
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(idle, |ui| {
                if self.chat.config.is_empty() {
                    if let Some((current, models)) = &self.chat.models {
                        let shown = models.iter().find(|m| &m.id == current).map_or(current.as_str(), |m| m.name.as_str());
                        egui::ComboBox::from_id_salt("assistant_model").selected_text(format!("Model: {shown}")).show_ui(ui, |ui| {
                            for m in models {
                                if ui.selectable_label(&m.id == current, &m.name).on_hover_text(&m.description).clicked() {
                                    model = Some(m.id.clone());
                                }
                            }
                        });
                    }
                    if let Some((current, modes)) = &self.chat.modes {
                        let shown = modes.iter().find(|(id, _)| id == current).map_or(current.as_str(), |(_, name)| name.as_str());
                        egui::ComboBox::from_id_salt("assistant_mode").selected_text(format!("Mode: {shown}")).show_ui(ui, |ui| {
                            for (id, name) in modes {
                                if ui.selectable_label(id == current, name).clicked() {
                                    mode = Some(id.clone());
                                }
                            }
                        });
                    }
                }
                for option in &self.chat.config {
                    let shown = option
                        .options
                        .iter()
                        .find(|choice| choice.value == option.current_value)
                        .map_or(option.current_value.as_str(), |choice| choice.name.as_str());
                    egui::ComboBox::from_id_salt(("assistant_option", &option.id)).selected_text(format!("{}: {shown}", option.name)).show_ui(
                        ui,
                        |ui| {
                            for choice in &option.options {
                                let row = ui.selectable_label(choice.value == option.current_value, &choice.name);
                                if row.on_hover_text(&choice.description).clicked() {
                                    chosen = Some((option.id.clone(), choice.value.clone(), option.category == "model"));
                                }
                            }
                        },
                    );
                }
            });
        });
        if let Some((id, value, is_model)) = chosen {
            self.chat.set_option(&id, &value);
            if is_model {
                self.remember_model(&value);
            }
        }
        if let Some(value) = model {
            self.chat.set_model(&value);
            self.remember_model(&value);
        }
        if let Some(value) = mode {
            self.chat.set_mode(&value);
        }
    }

    /// Keep the prompt above Send/Stop, retaining typed input while edits are paused.
    /// A library document can provide context even when no project session is open.
    fn composer(&mut self, ui: &mut Ui, _session: &Session, can_edit: bool, has_document: bool) {
        let running = self.chat.state == State::Running;
        let can_send = has_document && can_edit && self.chat.state == State::Ready && !self.draft.trim().is_empty();
        let draft_id = egui::Id::new("assistant_draft_input");
        let enter = can_send
            && ui.memory(|memory| memory.has_focus(draft_id))
            && ui.input_mut(|input| input.modifiers == egui::Modifiers::NONE && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ScrollArea::vertical().id_salt("assistant_draft").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.add_enabled(
                has_document,
                TextEdit::multiline(&mut self.draft)
                    .id(draft_id)
                    .hint_text("Ask about the document or request a change…")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .char_limit(8192),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if running {
                if ui.button("Stop").clicked() {
                    self.chat.stop();
                }
                ui.spinner();
            } else if ui.add_enabled(can_send, egui::Button::new("Send")).on_hover_text("Send (Enter)").clicked() || enter {
                let text = self.draft.trim().to_owned();
                if self.chat.send(&text) {
                    self.draft.clear();
                }
            }
            if !has_document {
                ui.label(dim("Open or create a document to start."));
            } else if !self.chat.connected() {
                ui.label(dim("Connect an agent to start."));
            }
        });
    }
}

/// Ask an endpoint for its models and whether `entry`'s model answers, with its stored key.
fn test_endpoint(entry: &Entry, protocol: craft_agents::agents::Protocol) -> Result<String, String> {
    use craft_agents::{agents::Protocol, api_client};
    let key = if entry.key { host::endpoint_key(&entry.id, &entry.base_url)? } else { None };
    if entry.key && key.is_none() {
        return Err("No key is stored for this endpoint. Edit it to add one.".to_owned());
    }
    let http: craft_agents::backend::HttpFn = Arc::new(host::http);
    match protocol {
        Protocol::OpenAi => api_client::test_endpoint(&http, &entry.base_url, key.as_deref(), &entry.model),
        Protocol::Anthropic => api_client::test_anthropic_endpoint(&http, &entry.base_url, key.as_deref(), &entry.model),
    }
}

/// One conversation item: the person's message, the agent's reply or thought, a tool call card,
/// its plan, or a notice.
fn conversation_item(ui: &mut Ui, item: &Item, agent: &str) {
    match item {
        Item::User(text) => chrome::user_bubble(ui, text),
        Item::Agent { thought: false, text, parsed, cut } => {
            ui.strong(agent);
            markdown_view(ui, parsed.get_or_init(|| markdown::parse(text)));
            if *cut {
                ui.label(dim("The rest of this reply was dropped: it passed 64 KiB."));
            }
        }
        Item::Agent { thought: true, text, .. } => {
            ui.label(dim(text.chars().take(MAX_TOOL_DETAIL).collect::<String>()));
        }
        Item::Tool { id, title, status, input, output } => {
            let status = match status {
                Some(craft_agents::acp::ToolStatus::Completed) => "Done",
                Some(craft_agents::acp::ToolStatus::Failed) => "Failed",
                Some(craft_agents::acp::ToolStatus::InProgress) => "Running",
                Some(craft_agents::acp::ToolStatus::Pending) | None => "Waiting",
            };
            egui::CollapsingHeader::new(format!("{title} · {status}")).id_salt(("assistant_tool", id)).show(ui, |ui| {
                if let Some(input) = input {
                    ui.monospace(input.to_string().chars().take(MAX_TOOL_DETAIL).collect::<String>());
                }
                if let Some(output) = output {
                    ui.label(dim(output.chars().take(MAX_TOOL_DETAIL).collect::<String>()));
                }
            });
        }
        Item::Plan(entries) => {
            ui.strong("Plan");
            for (text, status) in entries {
                let mark = match status.as_str() {
                    "completed" => "[x]",
                    "in_progress" => "[>]",
                    _ => "[ ]",
                };
                ui.label(format!("{mark} {text}"));
            }
        }
        Item::Notice(text) => {
            ui.label(dim(text));
        }
    }
}

/// An agent's Markdown reply: paragraphs, headings, lists, quotes, code and links.
fn markdown_view(ui: &mut Ui, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Paragraph(spans) => spans_view(ui, spans),
            Block::Heading { spans, .. } => {
                ui.strong(markdown::plain(spans));
            }
            Block::Code { text, .. } | Block::Pre(text) => {
                egui::Frame::group(ui.style()).show(ui, |ui| ui.monospace(text));
            }
            Block::List { ordered, start, items } => {
                for (index, item) in items.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if *ordered { format!("{}.", start.saturating_add(index as u64)) } else { "•".to_owned() });
                        spans_view(ui, item);
                    });
                }
            }
            Block::Quote(spans) => {
                ui.label(dim(markdown::plain(spans)));
            }
            Block::Rule => {
                ui.separator();
            }
        }
    }
}

/// Inline Markdown in one wrapped line: bold, italic, code and links.
fn spans_view(ui: &mut Ui, spans: &[Span]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for span in spans {
            match span {
                Span::Text(text) => ui.label(text),
                Span::Bold(text) => ui.label(RichText::new(text).strong()),
                Span::Italic(text) => ui.label(RichText::new(text).italics()),
                Span::Code(text) => ui.label(RichText::new(text).code()),
                Span::Link { label, url } => ui.hyperlink_to(label, url),
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable};

    use crate::{PcbApp, tests::click};

    use serde_json::json;

    use super::*;

    /// Moving, folding, or hiding the Assistant retains its draft and never starts an agent.
    #[test]
    fn assistant_window_actions_keep_the_conversation() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = PcbApp::default().with_integrated_titlebar(true);
        app.session.apply("project.new", &json!({"name": "Assistant fixture"}))?;
        let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        assert!(harness.query_by_label("Pop out Assistant").is_none());

        click(&mut harness, "Assistant");
        let original = harness.state().assistant.panel_rect.unwrap();
        harness.state_mut().assistant.draft = "Explain this schematic".to_owned();
        harness.run();
        let start = original.left_top() + egui::vec2(18.0, 25.0);
        let end = egui::pos2(80.0, start.y);
        harness.drag_at(start);
        harness.run();
        harness.hover_at(end);
        harness.run();
        harness.drop_at(end);
        harness.run();
        assert!(harness.state().assistant.dock.left);

        let panel = harness.state().assistant.panel_rect.unwrap();
        let start = panel.right_center();
        let end = start + egui::vec2(40.0, 0.0);
        harness.drag_at(start);
        harness.run();
        harness.hover_at(end);
        harness.run();
        harness.drop_at(end);
        harness.run();
        assert!(harness.state().assistant.panel_rect.unwrap().width() > panel.width());

        click(&mut harness, "Home");
        assert!(harness.state().home);
        assert!(harness.query_by_label("Pop out Assistant").is_none());
        click(&mut harness, "Assistant");
        assert!(!harness.state().home);
        assert_eq!(harness.state().assistant.draft, "Explain this schematic");

        click(&mut harness, "Pop out Assistant");
        assert!(harness.state().assistant.popped);
        click(&mut harness, "Dock Assistant");
        assert!(!harness.state().assistant.popped);
        click(&mut harness, "Minimize Assistant");
        click(&mut harness, "Expand Assistant");
        click(&mut harness, "Close Assistant");
        harness.key_press_modifiers(TOGGLE.modifiers, TOGGLE.logical_key);
        harness.run();
        assert_eq!(harness.state().assistant.draft, "Explain this schematic");
        assert!(!harness.state().assistant.chat.connected());
        click(&mut harness, "New chat");
        click(&mut harness, "Keep chat");
        assert_eq!(harness.state().assistant.draft, "Explain this schematic");
        click(&mut harness, "New chat");
        click(&mut harness, "Start new chat");
        assert!(harness.state().assistant.draft.is_empty());

        let assistant = &harness.state().assistant;
        let saved = Assistant::from_setting(Some(&assistant.layout_setting()), None);
        assert_eq!(saved.dock.left, assistant.dock.left);
        assert_eq!(saved.dock.width, assistant.dock.width);
        assert_eq!(saved.visibility, Visibility::Hidden);
        Ok(())
    }

    /// Clicking the chat keeps canvas/search shortcuts with the Assistant while application shortcuts still work.
    #[test]
    fn assistant_focus_keeps_application_shortcuts_available() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"name": "Assistant shortcut fixture"}))?;
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        harness.key_press(egui::Key::A);
        harness.run();
        harness.get_by_label("Close Parts Catalog");
        click(&mut harness, "Cancel");
        click(&mut harness, "Assistant");
        let point = harness.state().assistant.panel_rect.ok_or("Assistant panel is missing")?.center();
        harness.drag_at(point);
        harness.run();
        harness.drop_at(point);
        harness.run();
        assert!(harness.state().assistant.keyboard_active);
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(!harness.state().parts.open);
        harness.key_press(egui::Key::A);
        harness.run();
        assert!(matches!(harness.state().dialog, crate::Dialog::None));
        assert!(!harness.state().parts.open, "Assistant focus must not open the catalog");
        assert!(harness.query_by_label("Search parts").is_none());
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
        harness.run();
        assert!(matches!(harness.state().dialog, crate::Dialog::Preferences(_)));
        Ok(())
    }
}

#[cfg(test)]
mod chat_tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use craft_agents::{
        agents::{Entry, Kind},
        backend::{ChatBackend, Event, PermissionOption, PromptPart},
    };
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_mcp::{Backend, live::Remote};
    use serde_json::{Value, json};

    use crate::PcbApp;

    /// Click `label` and draw a few frames: while a reply runs, its spinner keeps repainting, so
    /// the harness can't wait for the window to settle.
    fn click(harness: &mut Harness<'_, PcbApp>, label: &str) {
        harness.get_by_label(label).click();
        harness.run_steps(3);
    }

    use super::*;

    /// What the scripted agent was told, and the events it will report next.
    #[derive(Default)]
    struct Script {
        events: VecDeque<Event>,
        prompts: Vec<String>,
        answers: Vec<(u64, Option<String>)>,
        shut_down: bool,
        models: Vec<String>,
    }

    /// A backend that reports scripted events and records what the chat sends it.
    struct Fake(Arc<Mutex<Script>>);

    impl ChatBackend for Fake {
        fn poll(&mut self) -> Vec<Event> {
            self.0.lock().map(|mut script| script.events.drain(..).collect()).unwrap_or_default()
        }
        /// Record a text message the chat sent.
        fn prompt(&mut self, parts: &[PromptPart]) {
            if let (Ok(mut script), [PromptPart::Text(text)]) = (self.0.lock(), parts) {
                script.prompts.push(text.clone());
            }
        }
        fn cancel(&mut self) {}
        fn new_session(&mut self) {}
        /// Record the chat's answer to a permission question.
        fn answer_permission(&mut self, key: u64, option: Option<&str>) {
            if let Ok(mut script) = self.0.lock() {
                script.answers.push((key, option.map(str::to_owned)));
            }
        }
        fn authenticate(&mut self, _method: &str) {}
        /// Record a model the chat chose.
        fn set_model(&mut self, model: &str) {
            if let Ok(mut script) = self.0.lock() {
                script.models.push(model.to_owned());
            }
        }
        /// Record that the chat ended the agent.
        fn shutdown(&mut self, _wait: bool) {
            if let Ok(mut script) = self.0.lock() {
                script.shut_down = true;
            }
        }
    }

    fn entry() -> Entry {
        Entry {
            id: "fake".to_owned(),
            name: "Fake agent".to_owned(),
            kind: Kind::Acp,
            argv: vec!["fake".to_owned()],
            version: None,
            login: "",
            found: false,
            base_url: String::new(),
            model: String::new(),
            key: false,
            request_timeout_s: 60,
            reasoning_effort: String::new(),
            problem: None,
            install: "",
        }
    }

    fn option(id: &str, kind: &str) -> PermissionOption {
        PermissionOption { id: id.to_owned(), name: id.to_owned(), kind: kind.to_owned() }
    }

    /// The window, the scripted agent and the chat endpoint's port and token file.
    type Fixture = (Harness<'static, PcbApp>, Arc<Mutex<Script>>, (u16, std::path::PathBuf));

    /// A project with two wired resistors, the Assistant open and the scripted agent ready.
    fn harness_with_agent() -> Result<Fixture, Box<dyn std::error::Error>> {
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"name": "chat"}))?;
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision}))?;
        }
        app.session.apply(
            pcbcraft_engine::CONNECT_PINS,
            &json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}, "expected_revision": 3}),
        )?;
        app.assistant.detection = Some(Default::default());
        let script = Arc::new(Mutex::new(Script::default()));
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        click(&mut harness, "Assistant");
        let ctx = harness.ctx.clone();
        let endpoint = harness.state_mut().assistant.chat.attach_with_endpoint(&ctx, Box::new(Fake(Arc::clone(&script))), entry());
        script.lock().map_err(|_| "poisoned")?.events.push_back(Event::Ready { agent: "Fake 1.0".to_owned(), images: false });
        harness.run();
        Ok((harness, script, endpoint))
    }

    /// Send the scripted agent a message, so its tool calls may run as in a real reply.
    fn open_turn(harness: &mut Harness<'_, PcbApp>) {
        assert!(harness.state_mut().assistant.chat.send("Go ahead"));
        harness.run_steps(2);
    }

    /// Queue an event for the scripted agent to report on its next poll.
    fn push(script: &Arc<Mutex<Script>>, event: Event) {
        if let Ok(mut script) = script.lock() {
            script.events.push_back(event);
        }
    }

    /// Run `command` from another thread as the agent's bridge would, while the window runs; when
    /// `answer` is given, press it once the approval card shows.
    fn call(
        harness: &mut Harness<'_, PcbApp>,
        endpoint: &(u16, std::path::PathBuf),
        command: &str,
        params: Value,
        answer: Option<&str>,
    ) -> Result<Value, String> {
        thread::scope(|scope| {
            let request = json!({"command": command, "params": params, "inspect": false});
            let worker = scope.spawn(move || {
                let mut remote = Remote::from_token_file(endpoint.0, &endpoint.1)?.with_wait(30);
                remote.call("engine.execute", request)
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut answered = answer.is_none();
            while !worker.is_finished() && Instant::now() < deadline {
                harness.run_steps(1);
                if !answered
                    && let Some(answer) = answer
                    && harness.query_by_label(answer).is_some()
                {
                    click(harness, answer);
                    answered = true;
                }
                thread::sleep(Duration::from_millis(2));
            }
            assert!(worker.is_finished(), "the chat endpoint did not answer {command}");
            // Draw what the answer changed, such as the edit's note.
            harness.run_steps(2);
            worker.join().unwrap_or_else(|_| Err("worker panicked".to_owned()))
        })
    }

    /// Replies stream into the conversation as Markdown; the agent's questions about PCBCraft's
    /// tools are allowed for it (the gate checks those calls), others wait for the person.
    #[test]
    fn the_chat_streams_replies_and_asks_only_about_the_agents_own_tools() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, _) = harness_with_agent()?;
        harness.get_by_label("Ready");
        harness.state_mut().assistant.draft = "Delete the wire".to_owned();
        harness.run();
        harness.get_by_label("Send").click();
        harness.run_steps(2);
        assert_eq!(script.lock().map_err(|_| "poisoned")?.prompts, ["Delete the wire"]);
        harness.get_by_label("Delete the wire");
        push(&script, Event::Text { thought: false, text: "Deleting **wire-4**".to_owned() });
        push(
            &script,
            Event::Permission {
                key: 7,
                title: format!("mcp__{}__execute", craft_agents::acp::bridge_name("0123456789ab")),
                kind: Some("other".to_owned()),
                options: vec![option("ok", "allow_once"), option("no", "reject_once")],
            },
        );
        push(
            &script,
            Event::Permission {
                key: 8,
                title: "Run rm -rf".to_owned(),
                kind: Some("execute".to_owned()),
                options: vec![option("ok", "allow_once"), option("no", "reject_once")],
            },
        );
        harness.run_steps(2);
        harness.get_by_label("wire-4");
        assert_eq!(script.lock().map_err(|_| "poisoned")?.answers, [(7, Some("ok".to_owned()))]);
        harness.get_by_label("Run rm -rf");
        harness.get_by_label("no").click();
        harness.run_steps(2);
        assert_eq!(script.lock().map_err(|_| "poisoned")?.answers.last(), Some(&(8, Some("no".to_owned()))));
        push(&script, Event::TurnEnded { reason: "end_turn".to_owned() });
        harness.run();
        assert_eq!(harness.state().assistant.chat.state, State::Ready);
        Ok(())
    }

    /// A held tool call nobody answers is declined before any connection gives up: the agent
    /// hears that the command did not run, the card goes and nothing changes.
    #[test]
    fn an_unanswered_tool_call_is_declined_with_an_answer() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, _script, endpoint) = harness_with_agent()?;
        open_turn(&mut harness);
        let unsaved = harness.state().files.dirty(&harness.state().session);
        let result = thread::scope(|scope| {
            let request = json!({"command": "file.save", "params": {}, "inspect": false});
            let endpoint = &endpoint;
            let worker = scope.spawn(move || {
                let mut remote = Remote::from_token_file(endpoint.0, &endpoint.1)?.with_wait(30);
                remote.call("engine.execute", request)
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while !worker.is_finished() && Instant::now() < deadline {
                harness.run_steps(1);
                if let Some(approval) = harness.state_mut().assistant.chat.approvals.first_mut()
                    && let Some(long_ago) = Instant::now().checked_sub(Duration::from_secs(craft_agents::acp::APPROVAL_WAIT_S + 1))
                {
                    approval.since = long_ago;
                }
                thread::sleep(Duration::from_millis(2));
            }
            worker.join().unwrap_or_else(|_| Err("worker panicked".to_owned()))
        });
        let error = result.err().ok_or("the unanswered save ran")?;
        assert!(error.contains("`file.save` was not run"), "{error}");
        assert!(harness.state().assistant.chat.approvals.is_empty());
        harness.get_by_label("`file.save` waited 4 minutes without an answer and was declined.");
        assert_eq!(harness.state().files.dirty(&harness.state().session), unsaved);
        Ok(())
    }

    /// Tool calls run only while a message is answered: after Stop, or once a new chat starts
    /// (as a project switch does), a call still on its way is refused and changes nothing.
    #[test]
    fn calls_after_stop_or_a_new_chat_do_not_run() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, endpoint) = harness_with_agent()?;
        let revision = harness.state().session.revision();
        let delete = json!({"wire": "wire-4", "expected_revision": revision});
        let idle = call(&mut harness, &endpoint, pcbcraft_engine::DELETE_WIRE, delete.clone(), None).unwrap_err();
        assert!(idle.contains("did not run"), "no reply was asked for: {idle}");

        open_turn(&mut harness);
        harness.state_mut().assistant.chat.stop();
        let stopped = call(&mut harness, &endpoint, pcbcraft_engine::DELETE_WIRE, delete.clone(), None).unwrap_err();
        assert!(stopped.contains("`schematic.delete_wire` did not run"), "{stopped}");
        let inspected = call(&mut harness, &endpoint, "view.fit", json!({"view": 0}), None).unwrap_err();
        assert!(inspected.contains("did not run"), "window commands are refused too: {inspected}");

        push(&script, Event::TurnEnded { reason: "cancelled".to_owned() });
        harness.run_steps(2);
        open_turn(&mut harness);
        assert!(harness.state().assistant.chat.turn_open());
        harness.state_mut().assistant.new_chat();
        let reset = call(&mut harness, &endpoint, pcbcraft_engine::DELETE_WIRE, delete, None).unwrap_err();
        assert!(reset.contains("did not run"), "{reset}");
        assert_eq!(harness.state().session.revision(), revision);
        Ok(())
    }

    /// A request that reached the endpoint before Stop doesn't run in the next reply, even when
    /// the person sends a new message before the window serves it: each reply has a generation
    /// of its own, checked right before a request runs.
    #[test]
    fn a_request_from_a_stopped_reply_does_not_run_in_the_next() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, _script, endpoint) = harness_with_agent()?;
        let revision = harness.state().session.revision();
        open_turn(&mut harness);
        harness.state_mut().assistant.chat.stop();
        let result = thread::scope(|scope| {
            let request =
                json!({"command": pcbcraft_engine::DELETE_WIRE, "params": {"wire": "wire-4", "expected_revision": revision}, "inspect": false});
            let endpoint = &endpoint;
            let worker = scope.spawn(move || {
                let mut remote = Remote::from_token_file(endpoint.0, &endpoint.1)?.with_wait(30);
                remote.call("engine.execute", request)
            });
            // Let it reach the endpoint's queue while no frame serves it, then start a new reply.
            thread::sleep(Duration::from_millis(500));
            harness.state_mut().assistant.chat.state = State::Ready;
            assert!(harness.state_mut().assistant.chat.send("Try again"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while !worker.is_finished() && Instant::now() < deadline {
                harness.run_steps(1);
                thread::sleep(Duration::from_millis(2));
            }
            worker.join().unwrap_or_else(|_| Err("worker panicked".to_owned()))
        });
        let error = result.err().ok_or("the stopped reply's edit ran in the next one")?;
        assert!(error.contains("did not run"), "{error}");
        assert_eq!(harness.state().session.revision(), revision);
        Ok(())
    }

    /// Only this chat's bridge, by its random name, gets the agent's permission questions
    /// answered for it; a server under the old fixed name is asked about like any other tool.
    #[test]
    fn only_this_chats_bridge_is_allowed_without_asking() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, _) = harness_with_agent()?;
        harness.state_mut().assistant.settings.agents.ai_permissions = craft_agents::agents::Mode::Ask;
        let ask = |key, title: String| Event::Permission {
            key,
            title,
            kind: Some("other".to_owned()),
            options: vec![option("ok", "allow_once"), option("no", "reject_once")],
        };
        let ours = format!("mcp__{}__execute", craft_agents::acp::bridge_name("0123456789ab"));
        push(&script, ask(1, ours.clone()));
        push(&script, ask(2, "mcp__pcbcraft-app__execute".to_owned()));
        harness.run();
        assert_eq!(script.lock().map_err(|_| "poisoned")?.answers, [(1, Some("ok".to_owned()))]);
        harness.get_by_label("mcp__pcbcraft-app__execute");

        // The person can choose to confirm the agent's requests for PCBCraft's tools too.
        harness.state_mut().assistant.settings.agents.auto_approve_bridge = false;
        push(&script, ask(3, ours.clone()));
        harness.run();
        assert_eq!(script.lock().map_err(|_| "poisoned")?.answers.len(), 1, "nothing more was answered for the person");
        harness.get_by_label(&ours);
        Ok(())
    }

    /// An endpoint reports Ready before it lists its models; the model chosen last time is still
    /// sent once the list arrives, and only once.
    #[test]
    fn the_saved_model_is_restored_when_the_list_arrives_after_ready() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, _) = harness_with_agent()?;
        harness.state_mut().assistant.settings.agents.agent_models.insert("fake".to_owned(), "large".to_owned());
        harness.state_mut().assistant.model_restored = false;
        harness.run();
        assert!(script.lock().map_err(|_| "poisoned")?.models.is_empty(), "nothing to restore before the list");
        let model = |id: &str| craft_agents::acp::ModelInfo { id: id.to_owned(), name: id.to_owned(), description: String::new() };
        push(&script, Event::Models { current: "small".to_owned(), available: vec![model("small"), model("large")] });
        harness.run();
        push(&script, Event::Models { current: "small".to_owned(), available: Vec::new() });
        harness.run();
        assert_eq!(script.lock().map_err(|_| "poisoned")?.models, ["large"]);
        Ok(())
    }

    /// Signing in from a terminal happens outside the chat, so the sign-in state offers Retry.
    #[test]
    fn the_sign_in_state_offers_retry() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, _) = harness_with_agent()?;
        push(&script, Event::AuthRequired { methods: Vec::new() });
        harness.run();
        harness.get_by_label("Sign in to continue.");
        harness.get_by_label("Retry");
        Ok(())
    }

    /// An agent that refuses the session but keeps running is ended, so the panel offers Retry
    /// with the reason instead of Disconnect.
    #[test]
    fn a_refused_session_offers_retry_while_the_agent_still_runs() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, script, _) = harness_with_agent()?;
        harness.state_mut().assistant.chat.state = State::Connecting;
        push(&script, Event::SetupFailed("no workspace".to_owned()));
        push(&script, Event::Text { thought: false, text: "late".to_owned() });
        harness.run();
        assert!(script.lock().map_err(|_| "poisoned")?.shut_down, "the agent is ended");
        assert!(!harness.state().assistant.chat.connected());
        assert_eq!(harness.state().assistant.chat.state, State::Failed("The agent could not start a session: no workspace".to_owned()));
        harness.get_by_label("Retry");
        assert!(harness.query_by_label("Disconnect").is_none());
        assert!(harness.query_by_label("late").is_none(), "nothing from the ended agent is shown");
        Ok(())
    }

    /// Under Edits a project edit runs at once and shares undo; saving waits and a denial reaches
    /// the agent. Under Ask a view command waits for Allow, and Always allow is remembered.
    #[test]
    fn the_gate_holds_tool_calls_for_the_person() -> Result<(), Box<dyn std::error::Error>> {
        let (mut harness, _script, endpoint) = harness_with_agent()?;
        open_turn(&mut harness);
        let revision = harness.state().session.revision();
        let deleted = call(&mut harness, &endpoint, pcbcraft_engine::DELETE_WIRE, json!({"wire": "wire-4", "expected_revision": revision}), None)?;
        assert_eq!(deleted["revision"], revision + 1);
        harness.get_by_label("Edited: Delete Wire. Undo reverts it.");
        click(&mut harness, "Undo");
        assert_eq!(harness.state().session.project().map(|project| project.wires().len()), Some(1));

        let denied = call(&mut harness, &endpoint, "file.save", json!({}), Some("Deny")).unwrap_err();
        assert!(denied.contains("denied `file.save`"), "{denied}");
        assert!(harness.state().assistant.chat.approvals.is_empty());

        harness.state_mut().assistant.settings.agents.ai_permissions = craft_agents::agents::Mode::Ask;
        let fit = json!({"view": 0});
        call(&mut harness, &endpoint, "view.fit", fit.clone(), Some("Allow"))?;
        call(&mut harness, &endpoint, "view.fit", fit.clone(), Some("Always allow"))?;
        assert!(harness.state().assistant.always_allowed("view.fit"));
        call(&mut harness, &endpoint, "view.fit", fit, None)?;
        let inspected = call(&mut harness, &endpoint, "edit.undo", Value::Null, Some("Allow"));
        assert!(inspected.is_ok() || inspected.is_err_and(|error| error.contains("nothing to undo")));
        Ok(())
    }

    /// Each request's first line and body, in order.
    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// Serve `replies` as an OpenAI-compatible endpoint on this computer, one connection each,
    /// recording each request's first line and body.
    fn endpoint(replies: Vec<Value>) -> Result<(u16, Seen), std::io::Error> {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        thread::spawn(move || {
            let mut replies = replies.into_iter();
            for stream in listener.incoming().map_while(Result::ok) {
                let mut reader = BufReader::new(stream);
                let (mut first, mut length) = (String::new(), 0);
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    line.clear();
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                if let Ok(mut seen) = record.lock() {
                    seen.push((first.trim().to_owned(), String::from_utf8_lossy(&body).into_owned()));
                }
                let answer = replies.next().unwrap_or_else(|| json!({"error": {"message": "no more replies"}})).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = reader.get_mut().write_all(response.as_bytes());
            }
        });
        Ok((port, seen))
    }

    /// A model endpoint on this computer connects through the desktop's HTTP client: its models
    /// are listed, a message goes to its chat completions and the reply streams into the chat.
    #[test]
    fn a_model_endpoint_answers_through_the_desktop_http_client() -> Result<(), Box<dyn std::error::Error>> {
        let (port, seen) = endpoint(vec![
            json!({"data": [{"id": "test-model"}, {"id": "other-model"}]}),
            json!({"choices": [{"message": {"role": "assistant", "content": "Hello from **the endpoint**"}, "finish_reason": "stop"}]}),
        ])?;
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"name": "endpoint"}))?;
        app.assistant.detection = Some(Default::default());
        app.assistant.settings.agents.agents.push(craft_agents::agents::AgentConfig {
            id: "api-1".to_owned(),
            name: "Local model".to_owned(),
            kind: Kind::Api,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "test-model".to_owned(),
            request_timeout_s: 30,
            ..Default::default()
        });
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        click(&mut harness, "Assistant");
        assert_eq!(harness.state().assistant.current().map(|agent| agent.name), Some("Local model".to_owned()));
        click(&mut harness, "Connect");
        let wait = |harness: &mut Harness<'_, PcbApp>, done: &dyn Fn(&PcbApp) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(harness.state()) && Instant::now() < deadline {
                harness.step();
                thread::sleep(Duration::from_millis(5));
            }
            assert!(done(harness.state()), "{:?}", harness.state().assistant.chat);
        };
        wait(&mut harness, &|app| {
            app.assistant.chat.state == State::Ready && app.assistant.chat.models.as_ref().is_some_and(|(_, models)| models.len() == 2)
        });
        assert!(harness.state_mut().assistant.chat.send("hi"));
        wait(&mut harness, &|app| app.assistant.chat.state == State::Ready);
        harness.run_steps(2);
        harness.get_by_label("the endpoint");
        let seen = seen.lock().map_err(|_| "poisoned")?.clone();
        assert_eq!(seen[0].0, "GET /v1/models HTTP/1.1");
        assert_eq!(seen[1].0, "POST /v1/chat/completions HTTP/1.1");
        let body: Value = serde_json::from_str(&seen[1].1)?;
        assert_eq!(body["model"], "test-model");
        assert!(body["messages"].as_array().is_some_and(|messages| messages.iter().any(|message| message["content"] == "hi")), "{body}");

        // Disconnecting removes the agent's working folder once the tools bridge has ended (#218;
        // cargo puts `pcbcraft-cli` on PATH for tests when it is built, so the bridge runs then).
        let workdir = harness.state().assistant.chat.workdir_path().ok_or("connected chats have a working folder")?;
        harness.state_mut().assistant.chat.disconnect();
        let deadline = Instant::now() + chat::WORKDIR_RELEASE_LIMIT;
        while workdir.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!workdir.exists(), "{} was left behind", workdir.display());
        Ok(())
    }
}
