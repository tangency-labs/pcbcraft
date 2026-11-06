//! The conversation with a connected agent, a lighter rewrite of SolidCraft's agent chat state
//! (`crates/ui-egui/src/agent_chat/mod.rs` and `gate.rs`, see `ATTRIBUTION.md`). The agent runs
//! as an ACP backend from `craft-agents` and reaches the open window through its own live MCP
//! endpoint; every request from that endpoint passes [`gate`] before the editor runs it, and the
//! ones that need the person's answer wait here as [`Approval`]s.

use std::time::Duration;

use craft_agents::backend::Secrets as _;
use craft_agents::{
    acp::{self, McpBridge, ModelInfo},
    agents::{Entry, Mode},
    backend::{AuthMethod, ChatBackend, ConfigOption, Event, PermissionOption, PromptPart, StartContext, ToolStatus},
};
use serde_json::Value;

use crate::live_control::{LiveControl, Request};

/// Most conversation items kept; older ones scroll away.
const MAX_ITEMS: usize = 400;
/// Longest text kept for one item, so a runaway reply stays bounded.
const MAX_TEXT: usize = 64 * 1024;

/// Where the connection stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum State {
    #[default]
    Disconnected,
    Connecting,
    Ready,
    /// A message is being answered.
    Running,
    /// The agent needs the person to sign in with one of its methods.
    SignIn(Vec<AuthMethod>),
    Failed(String),
}

/// One entry in the conversation.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Item {
    User(String),
    /// Streamed text; `thought` is the agent's reasoning, shown dimmed.
    Agent {
        thought: bool,
        text: String,
    },
    Tool {
        id: String,
        title: String,
        status: Option<ToolStatus>,
        input: Option<Value>,
        output: Option<String>,
    },
    Plan(Vec<(String, String)>),
    Notice(String),
}

/// A question waiting for the person: the agent's own permission request, or a tool call the
/// gate is holding.
pub(crate) struct Approval {
    pub(super) id: u64,
    pub(super) title: String,
    pub(super) detail: String,
    pub(crate) source: Source,
}

impl std::fmt::Debug for Approval {
    /// Name the question without the held request.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Approval").field("id", &self.id).field("title", &self.title).finish_non_exhaustive()
    }
}

/// What an [`Approval`] answers.
pub(crate) enum Source {
    /// The agent asked before running something of its own; answered with one of its options.
    Agent { key: u64, options: Vec<PermissionOption> },
    /// A request from the chat's endpoint, held until allowed or denied. `command` names it for
    /// "always allow".
    Tool { request: Request, command: String },
}

/// The person's answer to an approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// An agent permission option, by id.
    Option(String),
    Allow,
    AlwaysAllow,
    Deny,
}

/// What the gate does with a request from the chat's endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Run,
    Ask,
}

/// Decide whether a live request from the chat runs at once or waits for the person. Reading
/// always runs. Under Ask every command waits; under Edits project and view commands run while
/// saving, replacing the project and closing windows wait; All runs everything. A command the
/// person always allowed for this agent runs too. The editor's own refusals still apply after.
pub(crate) fn gate(mode: Mode, method: &str, command: Option<&str>, always: impl Fn(&str) -> bool) -> Verdict {
    let Some(command) = command.filter(|_| method == "engine.execute") else {
        return Verdict::Run;
    };
    if always(command) {
        return Verdict::Run;
    }
    match mode {
        Mode::All => Verdict::Run,
        Mode::Ask => Verdict::Ask,
        Mode::Edits if asks_under_edits(command) => Verdict::Ask,
        Mode::Edits => Verdict::Run,
    }
}

/// Commands that touch files, replace the project or close a window: they ask even under Edits.
fn asks_under_edits(command: &str) -> bool {
    command.starts_with("file.") || matches!(command, "project.new" | "window.close")
}

/// The chat: its backend, conversation, the agent's own options and pending approvals.
#[derive(Default)]
pub(super) struct Chat {
    backend: Option<Box<dyn ChatBackend>>,
    pub(super) state: State,
    pub(super) items: Vec<Item>,
    /// The agent this conversation is connected to.
    pub(super) agent: Option<Entry>,
    pub(super) config: Vec<ConfigOption>,
    pub(super) models: Option<(String, Vec<ModelInfo>)>,
    pub(super) modes: Option<(String, Vec<(String, String)>)>,
    /// Whether the agent took the PCBCraft tools; `Some(false)` means it can only talk.
    pub(super) tools: Option<bool>,
    pub(super) approvals: Vec<Approval>,
    next_approval: u64,
    /// The chat's own live endpoint, for the agent's MCP bridge.
    control: Option<LiveControl>,
    /// An empty working folder for the agent, removed when the chat disconnects.
    workdir: Option<tempfile::TempDir>,
}

impl std::fmt::Debug for Chat {
    /// Summarize the conversation without its contents or the endpoint's credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chat")
            .field("state", &self.state)
            .field("items", &self.items.len())
            .field("approvals", &self.approvals.len())
            .finish_non_exhaustive()
    }
}

impl Drop for Chat {
    /// End the agent and everything it started, without blocking the window.
    fn drop(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            backend.shutdown(false);
        }
    }
}

impl Chat {
    /// Start `entry` with a fresh endpoint and working folder: an ACP agent as a child process,
    /// or a model endpoint that PCBCraft talks to itself with its stored key. The bridge needs
    /// `pcbcraft-cli` beside the app; without it the agent can talk but not reach the project.
    pub(super) fn connect(&mut self, ctx: &egui::Context, entry: Entry) {
        self.disconnect();
        let key = match (entry.kind.is_api() && entry.key).then(|| super::host::Keychain.get(&entry.id)) {
            None => None,
            Some(Ok(Some(key))) => Some(key),
            Some(Ok(None)) => {
                self.state = State::Failed("No key is stored for this endpoint. Edit it in Preferences › AI to add one.".to_owned());
                return;
            }
            Some(Err(error)) => {
                self.state = State::Failed(format!("Could not read the key from the keychain: {error}"));
                return;
            }
        };
        let workdir = match tempfile::Builder::new().prefix("pcbcraft-agent-").tempdir() {
            Ok(dir) => dir,
            Err(error) => {
                self.state = State::Failed(format!("Could not make the agent's working folder: {error}"));
                return;
            }
        };
        let control = LiveControl::start_with_wait(ctx, Duration::from_secs(acp::BRIDGE_WAIT_S)).ok();
        let bridge =
            control.as_ref().zip(cli_path()).map(|(control, cli)| McpBridge { cli, port: control.port(), token: control.secret().to_owned() });
        if bridge.is_none() {
            self.items.push(Item::Notice("pcbcraft-cli was not found beside PCBCraft, so the agent can talk but not reach the project.".to_owned()));
        }
        let wake_ctx = ctx.clone();
        let start = StartContext { cwd: workdir.path().to_path_buf(), bridge, tools: None, wake: Box::new(move || wake_ctx.request_repaint()) };
        let started: Result<Box<dyn ChatBackend>, String> = match entry.kind.protocol() {
            None => craft_agents::acp_client::AcpBackend::start(&entry.argv, start).map(|backend| Box::new(backend) as Box<dyn ChatBackend>),
            Some(protocol) => {
                let config = craft_agents::api_client::ApiConfig {
                    name: entry.name.clone(),
                    protocol,
                    base_url: entry.base_url.clone(),
                    model: entry.model.clone(),
                    key,
                    http: std::sync::Arc::new(super::host::http),
                    request_timeout_s: entry.request_timeout_s,
                    reasoning_effort: entry.reasoning_effort.clone(),
                };
                craft_agents::api_client::ApiBackend::start(config, start).map(|backend| Box::new(backend) as Box<dyn ChatBackend>)
            }
        };
        match started {
            Ok(backend) => {
                self.attach(backend, entry);
                self.control = control;
                self.workdir = Some(workdir);
            }
            Err(error) => self.state = State::Failed(entry.problem_hint().unwrap_or(error)),
        }
    }

    /// Attach a scripted backend with a real chat endpoint, as [`Self::connect`] would.
    #[cfg(test)]
    pub(super) fn attach_with_endpoint(&mut self, ctx: &egui::Context, backend: Box<dyn ChatBackend>, entry: Entry) -> (u16, String) {
        self.attach(backend, entry);
        let control = LiveControl::start_with_wait(ctx, Duration::from_secs(acp::BRIDGE_WAIT_S)).unwrap_or_else(|error| panic!("{error}"));
        let endpoint = (control.port(), control.secret().to_owned());
        self.control = Some(control);
        endpoint
    }

    /// Use an already started backend for `entry`. Tests attach a scripted one.
    pub(super) fn attach(&mut self, backend: Box<dyn ChatBackend>, entry: Entry) {
        self.backend = Some(backend);
        self.agent = Some(entry);
        self.state = State::Connecting;
    }

    /// End the agent, revoke its endpoint, deny anything waiting and keep the conversation.
    pub(super) fn disconnect(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.shutdown(false);
        }
        self.deny_all("The chat disconnected.");
        self.control = None;
        self.workdir = None;
        self.config.clear();
        self.models = None;
        self.modes = None;
        self.tools = None;
        if !matches!(self.state, State::Failed(_)) {
            self.state = State::Disconnected;
        }
    }

    /// Whether an agent is connected or connecting.
    pub(super) fn connected(&self) -> bool {
        self.backend.is_some()
    }

    /// Clear the conversation; a connected agent starts a fresh session.
    pub(super) fn new_conversation(&mut self) {
        self.items.clear();
        self.deny_all("A new chat started.");
        if let Some(backend) = self.backend.as_mut() {
            backend.new_session();
            self.state = State::Connecting;
        }
    }

    /// Send a message when the agent is ready for one.
    pub(super) fn send(&mut self, text: &str) -> bool {
        let Some(backend) = self.backend.as_mut().filter(|_| self.state == State::Ready) else { return false };
        backend.prompt(&[PromptPart::Text(text.to_owned())]);
        self.push(Item::User(text.to_owned()));
        self.state = State::Running;
        true
    }

    /// Stop the message being answered. Held tool calls are denied with it.
    pub(super) fn stop(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            backend.cancel();
        }
        self.deny_all("Stopped.");
    }

    /// Ask the agent to sign in by itself with `method`.
    pub(super) fn sign_in(&mut self, method: &str) {
        if let Some(backend) = self.backend.as_mut() {
            backend.authenticate(method);
            self.state = State::Connecting;
        }
    }

    /// Choose a value of one of the agent's own options (model, effort, mode).
    pub(super) fn set_option(&mut self, id: &str, value: &str) {
        if let Some(backend) = self.backend.as_mut() {
            backend.set_config_option(id, value);
        }
    }

    /// Use one of the models the agent listed.
    pub(super) fn set_model(&mut self, model: &str) {
        if let Some(backend) = self.backend.as_mut() {
            backend.set_model(model);
        }
        if let Some((current, _)) = self.models.as_mut() {
            current.clone_from(&model.to_owned());
        }
    }

    /// Switch to one of the agent's modes.
    pub(super) fn set_mode(&mut self, mode: &str) {
        if let Some(backend) = self.backend.as_mut() {
            backend.set_mode(mode);
        }
        if let Some((current, _)) = self.modes.as_mut() {
            current.clone_from(&mode.to_owned());
        }
    }

    /// The chat endpoint's next request, if any. A stopped endpoint disconnects the chat.
    pub(super) fn take_request(&mut self) -> Option<Request> {
        match self.control.as_ref().map(LiveControl::poll) {
            Some(Ok(request)) => request,
            Some(Err(error)) => {
                self.push(Item::Notice(error));
                self.disconnect();
                None
            }
            None => None,
        }
    }

    /// Hold a request until the person answers. `title` and `detail` describe it.
    pub(super) fn hold(&mut self, request: Request, command: String, detail: String) {
        let id = self.next_id();
        self.approvals.push(Approval { id, title: format!("Allow `{command}`?"), detail, source: Source::Tool { request, command } });
    }

    /// Take the approval answered with `id`, for the editor to act on.
    pub(super) fn take_approval(&mut self, id: u64) -> Option<Approval> {
        let index = self.approvals.iter().position(|approval| approval.id == id)?;
        Some(self.approvals.remove(index))
    }

    /// Answer an agent permission question with one of its options.
    pub(super) fn answer_agent(&mut self, key: u64, option: Option<&str>) {
        if let Some(backend) = self.backend.as_mut() {
            backend.answer_permission(key, option);
        }
    }

    /// Advance the backend: apply its events to the conversation and the options, answer its
    /// permission questions about PCBCraft's own tools (the gate checks those calls), and drop
    /// held requests whose caller gave up. `mode` decides the agent's other questions.
    pub(super) fn poll(&mut self, mode: Mode) {
        let expired: Vec<u64> = self
            .approvals
            .iter()
            .filter(|approval| matches!(&approval.source, Source::Tool { request, .. } if request.expired()))
            .map(|approval| approval.id)
            .collect();
        for id in expired {
            self.take_approval(id);
            self.push(Item::Notice("A tool call waited too long for an answer and was cancelled.".to_owned()));
        }
        let events = self.backend.as_mut().map(|backend| backend.poll()).unwrap_or_default();
        for event in events {
            self.event(event, mode);
        }
    }

    /// Apply one backend event.
    fn event(&mut self, event: Event, mode: Mode) {
        match event {
            Event::Documents(_) => {}
            Event::ToolsAvailable(available) => self.tools = Some(available),
            Event::Ready { agent, .. } => {
                if let Some(entry) = self.agent.as_mut().filter(|_| !agent.is_empty()) {
                    entry.version = Some(agent);
                }
                self.state = State::Ready;
            }
            Event::Text { thought, text } => self.stream(thought, &text),
            Event::Tool { id, title, kind: _, status, input, output } => self.tool(id, title, status, input, output),
            Event::Plan(entries) => match self.items.last_mut() {
                Some(Item::Plan(plan)) => *plan = entries,
                _ => self.push(Item::Plan(entries)),
            },
            Event::Permission { key, title, kind, options } => {
                let allow = options.iter().find(|option| option.kind == "allow_once").map(|option| option.id.clone());
                if acp::is_bridge_tool(&title, kind.as_deref()) || mode == Mode::All {
                    self.answer_agent(key, allow.as_deref());
                } else {
                    let id = self.next_id();
                    let detail = kind.map(|kind| format!("The agent wants to run a {kind} tool of its own.")).unwrap_or_default();
                    self.approvals.push(Approval { id, title, detail, source: Source::Agent { key, options } });
                }
            }
            Event::AuthRequired { methods } => self.state = State::SignIn(methods),
            Event::AuthStatus(status) => self.push(Item::Notice(status)),
            Event::Modes { current, available } => self.modes = Some((current, available)),
            Event::Models { current, available } => match self.models.as_mut() {
                Some(models) if available.is_empty() => models.0 = current,
                _ => self.models = Some((current, available)),
            },
            Event::ModelsNote(note) => {
                if let Some(note) = note {
                    self.push(Item::Notice(note));
                }
            }
            Event::ModelRejected(reason) => self.push(Item::Notice(format!("The agent refused that model: {reason}"))),
            Event::ConfigOptions(options) => self.config = options,
            Event::ConfigOptionFinished { result, .. } => match result {
                Ok(options) => self.config = options,
                Err(error) => self.push(Item::Notice(error)),
            },
            Event::TurnEnded { reason } => {
                if !reason.is_empty() && reason != "end_turn" {
                    self.push(Item::Notice(format!("Stopped: {reason}")));
                }
                if self.state == State::Running {
                    self.state = State::Ready;
                }
            }
            Event::Error(error) => self.push(Item::Notice(error)),
            Event::Exited { tail } => {
                let reason = tail.last().cloned().unwrap_or_else(|| "The agent stopped.".to_owned());
                self.backend = None;
                self.disconnect();
                self.state = State::Failed(reason);
            }
        }
    }

    /// Append streamed text to the agent's current reply or thought, or start a new one.
    fn stream(&mut self, thought: bool, text: &str) {
        if let Some(Item::Agent { thought: current, text: reply }) = self.items.last_mut()
            && *current == thought
        {
            if reply.len() + text.len() <= MAX_TEXT {
                reply.push_str(text);
            }
            return;
        }
        self.push(Item::Agent { thought, text: text.chars().take(MAX_TEXT).collect() });
    }

    /// Record a new tool call or update one already shown.
    fn tool(&mut self, id: String, title: Option<String>, status: Option<ToolStatus>, input: Option<Value>, output: Option<String>) {
        let existing = self.items.iter_mut().rev().find_map(|item| match item {
            Item::Tool { id: current, title, status, input, output } if *current == id => Some((title, status, input, output)),
            _ => None,
        });
        match existing {
            Some((current_title, current_status, current_input, current_output)) => {
                if let Some(title) = title {
                    *current_title = title;
                }
                if status.is_some() {
                    *current_status = status;
                }
                if input.is_some() {
                    *current_input = input;
                }
                if let Some(output) = output {
                    *current_output = Some(output.chars().take(MAX_TEXT).collect());
                }
            }
            None => self.push(Item::Tool { title: title.unwrap_or_else(|| id.clone()), id, status, input, output }),
        }
    }

    /// Add an item, dropping the oldest beyond the limit.
    pub(super) fn push(&mut self, item: Item) {
        if self.items.len() >= MAX_ITEMS {
            self.items.remove(0);
        }
        self.items.push(item);
    }

    fn next_id(&mut self) -> u64 {
        self.next_approval += 1;
        self.next_approval
    }

    /// Deny every waiting question: held tool calls get `reason`, agent questions are cancelled.
    fn deny_all(&mut self, reason: &str) {
        for approval in std::mem::take(&mut self.approvals) {
            match approval.source {
                Source::Tool { request, .. } => request.answer(Err(reason.to_owned())),
                Source::Agent { key, .. } => self.answer_agent(key, None),
            }
        }
    }
}

/// `pcbcraft-cli` beside the running app, or on PATH as a fallback when the app's folder can't
/// be read. `None` when neither exists.
fn cli_path() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "pcbcraft-cli.exe" } else { "pcbcraft-cli" };
    let beside = std::env::current_exe().ok().map(|exe| exe.with_file_name(name)).filter(|path| path.is_file());
    beside.or_else(|| craft_agents::discover::find_in(name.trim_end_matches(".exe"), &craft_agents::discover::search_dirs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reading always runs; Edits holds files, project replacement and window closing; Ask holds
    /// every command; All and remembered answers run them.
    #[test]
    fn the_gate_follows_the_permission_mode() {
        let never = |_: &str| false;
        for method in ["project.inspect", "editor.inspect", "engine.commands", "project.cross_probe"] {
            assert_eq!(gate(Mode::Ask, method, None, never), Verdict::Run, "{method}");
        }
        let run = |mode, command| gate(mode, "engine.execute", Some(command), never);
        for command in ["schematic.delete_wire", "edit.undo", "view.split", "app.set_preferences", "project.rename"] {
            assert_eq!(run(Mode::Edits, command), Verdict::Run, "{command}");
            assert_eq!(run(Mode::Ask, command), Verdict::Ask, "{command}");
        }
        for command in ["file.save", "project.new", "window.close"] {
            assert_eq!(run(Mode::Edits, command), Verdict::Ask, "{command}");
            assert_eq!(run(Mode::All, command), Verdict::Run, "{command}");
        }
        assert_eq!(gate(Mode::Ask, "engine.execute", Some("file.save"), |command| command == "file.save"), Verdict::Run);
    }
}
