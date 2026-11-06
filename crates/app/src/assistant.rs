//! In-app agent conversation. The agent sees the same command catalog and editor inspection
//! as a live MCP client and proposes an ordered batch of those commands. The editor owns the
//! proposal and its revision, and Apply runs it through the live MCP dispatcher.

mod agent;
mod options;
mod settings;

pub(super) use settings::{Form as AgentForm, Request as SettingsRequest};

use std::{
    ffi::OsStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use egui::{Align, Context, Layout, Rect, ScrollArea, TextEdit, Ui};
use pcbcraft_chrome::{Tokens, assistant as chrome, shell::dim};
use pcbcraft_engine::{Project, Session};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use settings::{Choice, Settings};

const MAX_MESSAGES: usize = 8;

pub(super) const TOGGLE: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::A);
const NEW_CHAT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::L);

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
    test_result: Option<(Choice, Result<bool, String>)>,
    auth: Option<Result<bool, String>>,
    project: Option<Arc<Project>>,
    draft: String,
    messages: Vec<Message>,
    job: Option<Job>,
    proposal: Option<Proposal>,
    /// The person pressed Apply; the editor takes the proposal and runs it after drawing.
    apply_requested: bool,
    /// The person pressed Send with this agent executable; the editor adds its context.
    send_requested: Option<String>,
    /// The commands the pending or last request offered, for checking its proposal.
    known: Known,
    /// Suggested models per agent, read once when its picker first shows.
    models: std::collections::BTreeMap<String, Vec<options::Model>>,
    /// A model name being typed into the model picker.
    model_draft: String,
    notice: Option<String>,
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

#[derive(Clone, Debug, Serialize)]
struct Message {
    role: &'static str,
    text: String,
}

/// Most commands one proposal may hold.
const MAX_STEPS: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    reply: String,
    edits: Vec<Step>,
}

/// One proposed command. `params` is a JSON object written as a string, so strict structured
/// output (which allows no free-form objects) can still carry any command's parameters.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    command: String,
    params: String,
}

/// The commands offered to the agent, by ID, with their titles for the review list.
type Known = std::collections::BTreeMap<String, String>;

/// Read the IDs and titles from a discovery list (`engine.commands`).
fn known_commands(commands: &Value) -> Known {
    commands
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| Some((command["id"].as_str()?.to_owned(), command["title"].as_str().unwrap_or_default().to_owned())))
        .collect()
}

impl Reply {
    /// Check the steps against the offered commands and parse their parameters, dropping any
    /// `expected_revision`: the editor supplies the current revision when it applies them.
    /// Unknown commands, unreadable parameters or too many steps reject the whole proposal.
    fn prepare(edits: Vec<Step>, known: &Known, revision: u64) -> Result<Option<Proposal>, String> {
        if edits.is_empty() {
            return Ok(None);
        }
        if edits.len() > MAX_STEPS {
            return Err(format!("The proposal has more than {MAX_STEPS} steps. Ask for a smaller change."));
        }
        let mut steps = Vec::with_capacity(edits.len());
        let mut summary = Vec::with_capacity(edits.len());
        for (index, step) in edits.into_iter().enumerate() {
            let title = known.get(&step.command).ok_or_else(|| format!("The proposal uses an unknown command `{}`. Ask again.", step.command))?;
            let mut params = match step.params.trim() {
                "" => Value::Object(Default::default()),
                text => serde_json::from_str::<Value>(text)
                    .ok()
                    .filter(Value::is_object)
                    .ok_or_else(|| format!("The proposal's parameters for `{}` aren't a JSON object. Ask again.", step.command))?,
            };
            if let Some(object) = params.as_object_mut() {
                object.remove("expected_revision");
            }
            summary.push(format!("{}. {title} {params}", index + 1));
            steps.push((step.command, params));
        }
        Ok(Some(Proposal { revision, steps, summary: summary.join("\n") }))
    }
}

/// A reviewed batch of shared commands, valid only at the revision it was proposed against.
#[derive(Debug)]
pub(super) struct Proposal {
    pub(super) revision: u64,
    pub(super) steps: Vec<(String, Value)>,
    summary: String,
}

impl Proposal {
    /// The numbered steps as the review list shows them.
    pub(super) fn summary(&self) -> String {
        self.summary.clone()
    }
}

#[derive(Debug)]
struct Job {
    kind: JobKind,
    result: Receiver<Result<Outcome, String>>,
    stop: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

#[derive(Clone, Copy, Debug)]
enum JobKind {
    Chat(u64),
    Check,
    SignIn,
    Test(Choice),
}

#[derive(Debug)]
enum Outcome {
    Reply(Reply),
    Authenticated(bool),
}

impl Drop for Job {
    /// Ask the worker to stop and reap its child without blocking the editor.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for Assistant {
    /// Give a cancelled worker time to reap the agent before the editor exits.
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.stop.store(true, Ordering::Release);
            let deadline = Instant::now() + Duration::from_millis(250);
            while !job.worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl Assistant {
    /// Restore the saved agent catalog and window layout, including startup preferences.
    pub(super) fn from_setting(value: Option<&str>, layout: Option<&str>, settings: Option<&str>) -> Self {
        let settings = Settings::load(settings, value);
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

    /// Serialize agent profiles and AI options without any credentials.
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

    /// Switch agents without carrying the old transcript or proposals into the new one.
    fn select(&mut self, choice: Choice) {
        if self.settings.selected != choice {
            self.settings.selected = choice;
            self.reset_connection();
            self.settings_changed = true;
        }
    }

    /// Invalidate the old connection when its CLI or profile changes, preserving the draft.
    fn reset_connection(&mut self) {
        self.auth = None;
        self.messages.clear();
        self.proposal = None;
        self.notice = None;
    }

    /// Report whether the view is open and whether its agent has a saved login.
    pub(super) fn toolbar_state(&self) -> (bool, bool) {
        (self.visibility == Visibility::Open, matches!(self.auth, Some(Ok(true))))
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

    /// Reserve rail space only while the restore strip is visible.
    pub(super) fn rail_clearance(&self) -> f32 {
        if self.visibility == Visibility::Minimized { chrome::rail_clearance() } else { 0.0 }
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

    /// Advance jobs and render the conversation. Home passes no workspace and suspends its view.
    /// The conversation only reads the session; Send and Apply are handed to the editor through
    /// [`Self::take_send`] and [`Self::take_apply`], which add context and run the commands.
    pub(super) fn show(&mut self, ctx: &Context, workspace: Option<Rect>, session: &Session, can_edit: bool) {
        self.sync_project(session);
        self.poll(session);
        if !self.settings.show_features {
            return;
        }
        if std::mem::take(&mut self.connect_on_first_frame) {
            let command = self.settings.target(self.settings.selected).2.to_owned();
            self.start_auth(ctx, false, OsStr::new(&command));
        }
        if self.proposal.as_ref().is_some_and(|proposal| proposal.revision != session.revision()) {
            self.proposal = None;
            self.notice = Some("The schematic changed. Ask again to refresh the proposal.".to_owned());
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
            Visibility::Open if self.popped => self.show_window(ctx, session, can_edit),
            Visibility::Open => {
                let mut dock = self.dock;
                let (action, rect) = chrome::drawer(ctx, workspace, &mut dock, self.connection_status(), matches!(self.auth, Some(Ok(true))), |ui| {
                    self.panel(ui, session, can_edit);
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
    fn show_window(&mut self, ctx: &Context, session: &Session, can_edit: bool) -> Option<chrome::Action> {
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
            chrome::window(ui, status, matches!(self.auth, Some(Ok(true))), |ui| self.panel(ui, session, can_edit))
        })
    }

    /// Keep new-chat and stop shortcuts inside the conversation when it owns the keyboard.
    fn chat_shortcuts(&mut self, ctx: &Context) {
        if ctx.input_mut(|input| input.consume_shortcut(&NEW_CHAT)) {
            self.request_new_chat();
        }
        if let Some(job) = &self.job
            && matches!(job.kind, JobKind::Chat(_))
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            job.stop.store(true, Ordering::Release);
            self.notice = Some("Request stopped.".to_owned());
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

    /// Ask before clearing a conversation, draft, or unapplied proposal.
    fn request_new_chat(&mut self) {
        self.confirm_new = !self.messages.is_empty() || !self.draft.is_empty() || self.proposal.is_some();
        if !self.confirm_new {
            self.notice = None;
        }
    }

    /// Clear local conversation data and discard any response from a cancelled request.
    fn new_chat(&mut self) {
        if let Some(job) = &self.job
            && matches!(job.kind, JobKind::Chat(_))
        {
            job.stop.store(true, Ordering::Release);
        }
        self.draft.clear();
        self.messages.clear();
        self.proposal = None;
        self.notice = None;
        self.confirm_new = false;
    }

    /// Clear conversation context on a project switch. Keep app-level agent sign-in.
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

    /// The executable the person asked to send with, once. The editor answers with
    /// [`Self::send`] and its context.
    pub(super) fn take_send(&mut self) -> Option<String> {
        self.send_requested.take()
    }

    /// Start one request using an immutable snapshot and the editor's `context`: its project
    /// and editor inspection and the `commands` it offers, as live MCP discovery lists them.
    /// Only a successful start consumes the draft.
    pub(super) fn send(&mut self, ctx: &Context, session: &Session, executable: &OsStr, context: Value) {
        let Some(snapshot) = session.snapshot() else { return };
        let prompt = self.draft.trim().to_owned();
        if prompt.is_empty() || self.job.is_some() || self.proposal.is_some() || !matches!(self.auth, Some(Ok(true))) {
            return;
        }
        self.known = known_commands(&context["commands"]);
        let commands: Vec<String> = self.known.keys().cloned().collect();
        let history = self.messages.clone();
        let revision = session.revision();
        let agent = self.settings.target(self.settings.selected).0;
        let options = self.settings.options(self.settings.selected);
        let executable = executable.to_owned();
        let worker_prompt = prompt.clone();
        if self.start_job(ctx, JobKind::Chat(revision), move |stop| {
            agent::request(agent, &options, &executable, &snapshot, &context, &commands, &history, &worker_prompt, stop).map(Outcome::Reply)
        }) {
            self.draft.clear();
            self.notice = None;
            self.add_message("You", prompt);
        } else {
            self.notice = Some("Could not start the Assistant.".to_owned());
        }
    }

    /// The proposal the person chose to apply, once. The editor runs it and reports back
    /// through [`Self::applied`].
    pub(super) fn take_apply(&mut self) -> Option<Proposal> {
        if std::mem::take(&mut self.apply_requested) { self.proposal.take() } else { None }
    }

    /// Record how an applied proposal went: how many project edits it made (each undoable),
    /// or why it stopped.
    pub(super) fn applied(&mut self, summary: &str, result: Result<usize, String>) {
        self.notice = Some(match result {
            Ok(edits) => {
                self.add_message("Editor", format!("Applied:\n{summary}"));
                if edits == 0 { "Applied.".to_owned() } else { "Applied. Use Undo to revert.".to_owned() }
            }
            Err(error) => error,
        });
    }

    /// Check the selected agent's login or launch its own browser sign-in.
    fn start_auth(&mut self, ctx: &Context, sign_in: bool, executable: &OsStr) {
        let agent = self.settings.target(self.settings.selected).0;
        let executable = executable.to_owned();
        let kind = if sign_in { JobKind::SignIn } else { JobKind::Check };
        if !self.start_job(ctx, kind, move |stop| agent::authenticate(agent, &executable, sign_in, stop).map(Outcome::Authenticated)) {
            self.auth = Some(Err("Could not start the sign-in process.".to_owned()));
        }
    }

    /// Start one cancellable worker. While it is active, the same editor cannot start another.
    fn start_job(&mut self, ctx: &Context, kind: JobKind, run: impl FnOnce(&AtomicBool) -> Result<Outcome, String> + Send + 'static) -> bool {
        if self.job.is_some() {
            return false;
        }
        let (send, result) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_ctx = ctx.clone();
        let worker = thread::Builder::new().name("pcbcraft-assistant".to_owned()).spawn(move || {
            let _ = send.send(run(&worker_stop));
            worker_ctx.request_repaint_of(egui::ViewportId::ROOT);
        });
        let Ok(worker) = worker else {
            return false;
        };
        self.job = Some(Job { kind, result, stop, worker });
        true
    }

    /// Consume a finished reply once. Ignore a stopped reply and discard edits from an old revision.
    fn poll(&mut self, session: &Session) {
        let Some(job) = &self.job else { return };
        let result = match job.result.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The Assistant stopped unexpectedly.".to_owned()),
        };
        let kind = job.kind;
        let stopped = job.stop.load(Ordering::Acquire);
        self.job = None;
        if stopped {
            return;
        }
        match (kind, result) {
            (JobKind::Chat(revision), Ok(Outcome::Reply(reply))) => {
                self.add_message(self.settings.target(self.settings.selected).0.label(), reply.reply);
                if !reply.edits.is_empty() && (session.project().is_none() || session.revision() != revision) {
                    self.notice = Some("The project changed while the agent worked. Ask again to refresh the proposal.".to_owned());
                } else {
                    match Reply::prepare(reply.edits, &self.known, revision) {
                        Ok(proposal) => self.proposal = proposal,
                        Err(error) => self.notice = Some(error),
                    }
                }
            }
            (JobKind::Chat(_), Err(error)) => self.notice = Some(error),
            (JobKind::Check | JobKind::SignIn, Ok(Outcome::Authenticated(signed_in))) => self.auth = Some(Ok(signed_in)),
            (JobKind::Check | JobKind::SignIn, Err(error)) => self.auth = Some(Err(error)),
            (JobKind::Test(choice), Ok(Outcome::Authenticated(signed_in))) => self.test_result = Some((choice, Ok(signed_in))),
            (JobKind::Test(choice), Err(error)) => self.test_result = Some((choice, Err(error))),
            _ => {}
        }
    }

    /// Retain only the newest visible messages and include their roles in follow-up requests.
    fn add_message(&mut self, role: &'static str, text: String) {
        if self.messages.len() == MAX_MESSAGES {
            self.messages.remove(0);
        }
        self.messages.push(Message { role, text });
    }

    /// Draw replies, a reviewable edit, and the request field inside the workspace drawer.
    fn panel(&mut self, ui: &mut Ui, session: &Session, can_edit: bool) {
        let t = Tokens::get();
        if self.confirm_new {
            ui.label("Start a new chat? This clears the conversation, draft, and any unapplied proposal.");
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
        if self.messages.is_empty() {
            ui.add_space(t.space_8);
            ui.label(dim("Sending includes the current schematic and recent conversation."));
        }
        chrome::composer(ui, |ui| self.composer(ui, session, can_edit));
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if let Some(notice) = &self.notice {
                ui.label(dim(notice));
            }
            if let Some(proposal) = &self.proposal {
                ui.label(&proposal.summary);
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_edit, egui::Button::new("Apply")).clicked() {
                        self.apply_requested = true;
                    } else if ui.button("Discard").clicked() {
                        self.proposal = None;
                        self.notice = Some("Proposal discarded.".to_owned());
                    }
                });
            }
            ui.separator();
            ScrollArea::vertical().id_salt("assistant_messages").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical(|ui| {
                    for message in &self.messages {
                        if message.role == "You" {
                            chrome::user_bubble(ui, &message.text);
                        } else {
                            ui.strong(message.role);
                            ui.label(&message.text);
                        }
                        ui.add_space(t.space_6);
                    }
                });
            });
        });
    }

    /// Reflect the current worker before the saved-login result in the header.
    fn connection_status(&self) -> &'static str {
        match self.job.as_ref().map(|job| job.kind) {
            Some(JobKind::Check) => "Connecting…",
            Some(JobKind::SignIn) => "Signing in…",
            Some(JobKind::Chat(_)) => "Working…",
            None | Some(JobKind::Test(_)) => match self.auth {
                None => "Not connected",
                Some(Ok(true)) => "Ready",
                Some(Ok(false)) => "Sign-in required",
                Some(Err(_)) => "Connection failed",
            },
        }
    }

    /// Pick the agent for this chat, then offer the action for its current connection state.
    fn agent_controls(&mut self, ui: &mut Ui) {
        let mut selected = self.settings.selected;
        chrome::agent_switcher(ui, self.settings.target(selected).1, true, |ui| {
            ui.add_enabled_ui(self.job.is_none(), |ui| {
                for choice in self.settings.choices() {
                    ui.selectable_value(&mut selected, choice, self.settings.target(choice).1);
                }
            });
            ui.separator();
            if ui.add_enabled(self.settings.can_add(), egui::Button::new("Add agent…")).clicked() {
                self.settings_request = Some(SettingsRequest::Add(AgentForm::default()));
                ui.close();
            }
            if ui.button("AI settings…").clicked() {
                self.settings_request = Some(SettingsRequest::Settings);
                ui.close();
            }
        });
        self.select(selected);
        self.option_pickers(ui);
        let command = self.settings.target(selected).2.to_owned();
        let executable = OsStr::new(&command);
        match self.job.as_ref().map(|job| job.kind) {
            Some(JobKind::SignIn) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(dim("Complete sign-in in your browser."));
                    if ui.button("Cancel sign-in").clicked()
                        && let Some(job) = &self.job
                    {
                        job.stop.store(true, Ordering::Release);
                        self.auth = Some(Ok(false));
                    }
                });
            }
            Some(_) => {}
            None => match &self.auth {
                Some(Ok(true)) => {
                    if ui.button("Disconnect").clicked() {
                        self.auth = None;
                    }
                }
                Some(Ok(false)) => {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Sign in").clicked() {
                            self.start_auth(ui.ctx(), true, executable);
                        } else if ui.button("I've signed in").clicked() {
                            self.start_auth(ui.ctx(), false, executable);
                        }
                    });
                }
                None | Some(Err(_)) => {
                    let label = if self.auth.is_none() { "Connect" } else { "Retry" };
                    if ui.button(label).clicked() {
                        self.start_auth(ui.ctx(), false, executable);
                    }
                    if let Some(Err(error)) = &self.auth {
                        ui.label(dim(error));
                    }
                }
            },
        }
    }

    /// The selected agent's Model and Effort pickers, as in SolidCraft's chat header. Choices
    /// apply from the next request and are remembered for that agent; Default passes nothing.
    /// A typed model is kept only when it is a valid single argument.
    fn option_pickers(&mut self, ui: &mut Ui) {
        let choice = self.settings.selected;
        let agent = self.settings.target(choice).0;
        let models = self.models.entry(agent.executable().to_owned()).or_insert_with(|| options::models(agent)).clone();
        let mut chosen = self.settings.options(choice);
        let shown = |value: &str, models: &[options::Model]| {
            if value.is_empty() {
                "Default".to_owned()
            } else {
                models.iter().find(|model| model.id == value).map_or(value.to_owned(), |model| model.label.clone())
            }
        };
        let idle = self.job.is_none();
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(idle, |ui| {
                let hint = format!("Passed to {} from the next request. Default uses the agent's own default.", agent.label());
                egui::ComboBox::from_id_salt("assistant_model")
                    .selected_text(format!("Model: {}", shown(&chosen.model, &models)))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen.model, String::new(), "Default");
                        for model in &models {
                            ui.selectable_value(&mut chosen.model, model.id.clone(), &model.label).on_hover_text(&model.id);
                        }
                        ui.separator();
                        let field =
                            ui.add(TextEdit::singleline(&mut self.model_draft).hint_text("Other model…").desired_width(160.0).char_limit(128));
                        if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                            let typed = self.model_draft.trim().to_owned();
                            if !typed.is_empty() && options::valid_model(&typed) {
                                chosen.model = typed;
                                self.model_draft.clear();
                                ui.close();
                            }
                        }
                        if !options::valid_model(self.model_draft.trim()) {
                            ui.label(dim("Use letters, digits and . _ : / [ ] -"));
                        }
                    })
                    .response
                    .on_hover_text(&hint);
                let efforts = options::efforts(agent, &chosen.model, &models);
                egui::ComboBox::from_id_salt("assistant_effort")
                    .selected_text(format!("Effort: {}", if chosen.effort.is_empty() { "Default" } else { &chosen.effort }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen.effort, String::new(), "Default");
                        for effort in efforts {
                            ui.selectable_value(&mut chosen.effort, effort.clone(), effort);
                        }
                    })
                    .response
                    .on_hover_text(&hint);
            });
        });
        if chosen != self.settings.options(choice) {
            self.settings.set_options(choice, chosen);
            self.settings_changed = true;
        }
    }

    /// Keep the prompt above its Send or Stop action. Long drafts scroll inside the input.
    fn composer(&mut self, ui: &mut Ui, session: &Session, can_edit: bool) {
        let can_send =
            session.project().is_some() && can_edit && self.job.is_none() && self.proposal.is_none() && matches!(self.auth, Some(Ok(true)));
        let busy = matches!(self.job.as_ref().map(|job| job.kind), Some(JobKind::Chat(_)));
        let draft_id = egui::Id::new("assistant_draft_input");
        let can_send = can_send && !self.draft.trim().is_empty();
        let enter = can_send
            && ui.memory(|memory| memory.has_focus(draft_id))
            && ui.input_mut(|input| input.modifiers == egui::Modifiers::NONE && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ScrollArea::vertical().id_salt("assistant_draft").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.add_enabled(
                session.project().is_some() && self.proposal.is_none() && !busy,
                TextEdit::multiline(&mut self.draft)
                    .id(draft_id)
                    .hint_text("Ask about the schematic or request a change…")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .char_limit(2048),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(job) = &self.job
                && busy
            {
                let stopping = job.stop.load(Ordering::Acquire);
                if ui.add_enabled(!stopping, egui::Button::new("Stop")).clicked() {
                    job.stop.store(true, Ordering::Release);
                    self.notice = Some("Request stopped.".to_owned());
                }
                ui.spinner();
            } else if ui.add_enabled(can_send, egui::Button::new("Send")).on_hover_text("Send (Enter)").clicked() || enter {
                self.send_requested = Some(self.settings.target(self.settings.selected).2.to_owned());
            }
            if session.project().is_none() {
                ui.label(dim("Open or create a project to start."));
            }
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt as _,
        time::{Duration, Instant},
    };

    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use pcbcraft_engine::PLACE_RESISTOR;

    use crate::{PcbApp, tests::click};

    use serde_json::json;

    use super::{agent::Agent, *};

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
        assert!(harness.state().assistant.job.is_none());
        click(&mut harness, "New chat");
        click(&mut harness, "Keep chat");
        assert_eq!(harness.state().assistant.draft, "Explain this schematic");
        click(&mut harness, "New chat");
        click(&mut harness, "Start new chat");
        assert!(harness.state().assistant.draft.is_empty());

        let assistant = &harness.state().assistant;
        let saved = Assistant::from_setting(None, Some(&assistant.layout_setting()), None);
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
        harness.get_by_label("Move Parts library");
        click(&mut harness, "Assistant");
        let point = harness.state().assistant.panel_rect.ok_or("Assistant panel is missing")?.center();
        harness.drag_at(point);
        harness.run();
        harness.drop_at(point);
        harness.run();
        assert!(harness.state().assistant.keyboard_active);
        harness.key_press(egui::Key::Escape);
        harness.run();
        harness.get_by_label("Move Parts library");
        harness.key_press(egui::Key::A);
        harness.run();
        assert!(matches!(harness.state().dialog, crate::Dialog::None));
        assert!(
            !harness.get_by_role(egui::accesskit::Role::TextInput).accesskit_node().is_focused(),
            "Assistant focus must not move to parts search"
        );
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
        harness.run();
        assert!(matches!(harness.state().dialog, crate::Dialog::Preferences(_)));
        Ok(())
    }

    /// A profile added from the dropdown survives a restart and connects using its saved executable.
    #[test]
    fn assistant_ai_preferences_add_and_connect_a_saved_agent() -> Result<(), Box<dyn std::error::Error>> {
        use egui::accesskit::Role;

        let fixture = tempfile::tempdir()?;
        let executable = fixture.path().join("test codex");
        fs::write(&executable, "#!/bin/sh\ntest \"$*\" = \"login status\"\n")?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;

        let mut harness =
            Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), PcbApp::default());
        harness.run();
        click(&mut harness, "Home");
        harness.query_all_by_label("Preferences").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).ok_or("Missing Home preferences")?.click();
        harness.run_steps(3);
        assert!(harness.state().home);
        click(&mut harness, "AI");
        harness.run_steps(3);
        harness.get_by_label("Show AI features");
        click(&mut harness, "OK");
        harness.step();
        assert!(matches!(harness.state().dialog, crate::Dialog::None), "{:?}", harness.state().dialog);

        click(&mut harness, "Assistant");
        assert_eq!(harness.state().assistant.visibility, Visibility::Open);
        agent_switcher(&harness)?.click();
        harness.run();
        click(&mut harness, "AI settings…");
        harness.run_steps(3);
        harness.get_by_label("Show AI features");
        click(&mut harness, "OK");
        harness.step();
        assert!(matches!(harness.state().dialog, crate::Dialog::None), "{:?}", harness.state().dialog);

        agent_switcher(&harness)?.click();
        harness.run();
        click(&mut harness, "Add agent…");
        harness.run_steps(3);
        click(&mut harness, "Custom command");
        harness.run_steps(3);
        for (label, value) in [("Name", "Local Codex"), ("Agent command", executable.to_str().ok_or("The fixture path is not UTF-8")?)] {
            harness.get_by_role_and_label(Role::TextInput, label).click();
            harness.run();
            harness.get_by_role_and_label(Role::TextInput, label).type_text(value);
            harness.run();
        }
        click(&mut harness, "Add");
        harness.run_steps(3);
        harness.step();
        assert!(matches!(harness.state().dialog, crate::Dialog::None), "{:?}", harness.state().dialog);

        agent_switcher(&harness)?.click();
        harness.run();
        click(&mut harness, "AI settings…");
        harness.run_steps(3);
        harness.get_by_label("Local Codex");
        harness.get_all_by_label("Make default").last().ok_or("Missing default action")?.click();
        harness.run();
        let settings = harness.state().assistant.settings_text();
        let restored = Assistant::from_setting(None, None, Some(&settings));
        assert_eq!(restored.settings.target(restored.settings.selected).1, "Local Codex");
        assert_eq!(restored.settings.target(restored.settings.selected).2, executable.to_str().ok_or("The fixture path is not UTF-8")?);

        click(&mut harness, "OK");
        harness.step();
        agent_switcher(&harness)?.click();
        harness.run();
        click(&mut harness, "Local Codex");
        harness.get_by_role_and_label(Role::Button, "Connect").click();
        harness.run();
        let deadline = Instant::now() + Duration::from_secs(3);
        while harness.state().assistant.job.is_some() && Instant::now() < deadline {
            harness.step();
            thread::sleep(Duration::from_millis(5));
        }
        harness.run();
        harness.get_by_label("Ready");
        Ok(())
    }

    /// The agent switcher: the first combo box in the agent row, ahead of Model and Effort.
    fn agent_switcher<'h>(harness: &'h Harness<'_, PcbApp>) -> Result<egui_kittest::Node<'h>, &'static str> {
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().ok_or("Missing the agent switcher")
    }

    /// Both provider flows start from sign-in, then apply a reviewed edit through shared history.
    #[test]
    fn assistant_proposal_requires_apply_and_respects_revision() -> Result<(), Box<dyn std::error::Error>> {
        for agent in [Agent::Codex, Agent::Claude] {
            exercise_provider(agent)?;
        }
        Ok(())
    }

    /// Exercise each CLI's auth flags and reply format, then reject a reply from a stale revision.
    fn exercise_provider(agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
        let fixture = tempfile::tempdir()?;
        let executable = fixture.path().join(agent.executable());
        fs::write(
            &executable,
            r#"#!/bin/sh
case "${0##*/}:$*" in
    'codex:login status'|'claude:auth status') exit 1 ;;
    'codex:login'|'claude:auth login --claudeai') exit 0 ;;
esac
all="$*"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output-last-message) shift; output="$1" ;;
    esac
    shift
done
prompt=$(cat)
case "$all" in 'exec -m gpt-test -c model_reasoning_effort="high" '*|'--model sonnet --effort high '*) ;; *) exit 1 ;; esac
case "$prompt" in *'"request":"Connect the resistors"'*) ;; *) exit 1 ;; esac
case "$prompt" in *'"reference":"R1"'*'"reference":"R2"'*) ;; *) exit 1 ;; esac
case "$prompt" in *'"id":"schematic.delete_wire"'*'"id":"view.split"'*) ;; *) exit 1 ;; esac
case "$prompt" in *'"focused_view":0'*) ;; *) exit 1 ;; esac
reply='{"reply":"I propose connecting the resistors.\\n\\nThen splitting the view.","edits":[{"command":"schematic.connect_pins","params":"{\"from\":{\"symbol\":\"symbol-2\",\"terminal\":\"2\"},\"to\":{\"symbol\":\"symbol-3\",\"terminal\":\"1\"},\"expected_revision\":99}"},{"command":"view.split","params":"{\"view\":0,\"split\":true}"}]}'
case "${0##*/}" in
    codex) printf '%s' "$reply" > "$output" ;;
    claude) printf '{"is_error":false,"structured_output":%s}' "$reply" ;;
esac
"#,
        )?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;

        let mut app = PcbApp::default();
        app.assistant.settings.selected = Choice::BuiltIn(agent);
        app.session.apply("project.new", &json!({"name": "assistant fixture"}))?;
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.apply(PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision}))?;
        }
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        click(&mut harness, "Assistant");
        harness.get_by_label("Minimize Assistant");
        harness.get_by_label("Not connected");
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Connect");
        let ctx = harness.ctx.clone();
        let start = |app: &mut PcbApp| {
            app.assistant.draft = "Connect the resistors".to_owned();
            let context = app.agent_context(&ctx);
            app.assistant.send(&ctx, &app.session, executable.as_os_str(), context);
        };
        let wait = |harness: &mut Harness<'_, PcbApp>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while harness.state().assistant.job.is_some() && Instant::now() < deadline {
                harness.step();
                thread::sleep(Duration::from_millis(5));
            }
            assert!(harness.state().assistant.job.is_none());
            harness.run();
        };

        let model = if agent == Agent::Codex { "gpt-test" } else { "sonnet" };
        let chosen = options::AgentOptions { model: model.to_owned(), effort: "high".to_owned() };
        harness.state_mut().assistant.settings.set_options(Choice::BuiltIn(agent), chosen.clone());
        let restored = Assistant::from_setting(None, None, Some(&harness.state().assistant.settings_text()));
        assert_eq!(restored.settings.options(Choice::BuiltIn(agent)), chosen, "choices survive a restart");
        harness.state_mut().assistant.start_auth(&ctx, false, executable.as_os_str());
        wait(&mut harness);
        harness.get_by_label("Sign in");
        harness.get_by_label("I've signed in");
        harness.state_mut().assistant.start_auth(&ctx, true, executable.as_os_str());
        wait(&mut harness);
        harness.get_by_label("Ready");
        assert!(harness.query_by_label("I've signed in").is_none());
        assert!(harness.query_by_role_and_label(egui::accesskit::Role::Button, "Connect").is_none());

        start(harness.state_mut());
        wait(&mut harness);
        assert_eq!(harness.state().session.project().map(|project| project.wires().len()), Some(0));
        let proposal =
            harness.state().assistant.proposal.as_ref().ok_or_else(|| format!("Missing proposal: {:?}", harness.state().assistant.notice))?;
        let commands: Vec<&str> = proposal.steps.iter().map(|(command, _)| command.as_str()).collect();
        assert_eq!(commands, ["schematic.connect_pins", "view.split"]);
        assert!(proposal.steps[0].1.get("expected_revision").is_none(), "the editor supplies the revision");
        assert!(proposal.summary.starts_with("1. Connect Pins {"), "{}", proposal.summary);
        assert!(proposal.summary.contains("\n2. Split {"), "{}", proposal.summary);
        harness.get_by_label("I propose connecting the resistors.\n\nThen splitting the view.");
        click(&mut harness, "Apply");
        assert_eq!(harness.state().session.project().map(|project| project.wires().len()), Some(1));
        assert!(harness.state().views.is_split(), "window commands run through the same dispatcher");
        harness.get_by_label("Applied. Use Undo to revert.");
        click(&mut harness, "Undo");
        assert_eq!(harness.state().session.project().map(|project| project.wires().len()), Some(0));

        start(harness.state_mut());
        harness.state_mut().session.apply("project.rename", &json!({"name": "changed while waiting"}))?;
        wait(&mut harness);
        harness.get_by_label("The project changed while the agent worked. Ask again to refresh the proposal.");
        assert!(harness.query_by_label("Apply").is_none());
        assert_eq!(harness.state().session.project().map(|project| project.wires().len()), Some(0));
        Ok(())
    }

    /// A failing step undoes the project edits of the steps before it, names the step, and
    /// leaves the history as it was; a stale proposal runs nothing.
    #[test]
    fn a_failing_step_undoes_the_earlier_project_edits() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"name": "batch"}))?;
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.apply(PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision}))?;
        }
        app.views.sync(&app.session);
        let before = app.session.inspect();
        let ctx = egui::Context::default();
        let connect = json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}});
        let steps = vec![
            ("schematic.connect_pins".to_owned(), connect.clone()),
            ("schematic.rotate_symbol".to_owned(), json!({"symbol": "symbol-2"})),
            ("schematic.delete_wire".to_owned(), json!({"wire": "wire-99"})),
        ];
        let error = app.run_agent_steps(&ctx, before["revision"].as_u64().ok_or("revision")?, &steps).unwrap_err();
        assert!(error.starts_with("Step 3 (schematic.delete_wire) failed:"), "{error}");
        assert!(error.ends_with("The project edits from earlier steps were undone."), "{error}");
        assert_eq!(app.session.inspect()["project"], before["project"]);
        assert_eq!(app.session.inspect()["can_undo"], before["can_undo"]);

        let stale = app.run_agent_steps(&ctx, 1, &steps[..1]).unwrap_err();
        assert!(stale.contains("project changed"), "{stale}");
        let revision = app.session.revision();
        assert_eq!(app.run_agent_steps(&ctx, revision, &steps[..2]), Ok(2));
        Ok(())
    }

    /// Saved model and effort choices that could read as flags load as the defaults, and
    /// choices for removed agents are forgotten.
    #[test]
    fn saved_agent_options_are_sanitized() {
        let saved = r#"{"options":[
            {"choice":{"BuiltIn":"Claude"},"model":"--dangerously-skip-permissions","effort":"high"},
            {"choice":{"Profile":7},"model":"opus","effort":"max"}
        ]}"#;
        let restored = Assistant::from_setting(None, None, Some(saved));
        let claude = restored.settings.options(Choice::BuiltIn(Agent::Claude));
        assert_eq!((claude.model.as_str(), claude.effort.as_str()), ("", "high"));
        assert_eq!(restored.settings.options(Choice::Profile(7)), options::AgentOptions::default());
    }
}
