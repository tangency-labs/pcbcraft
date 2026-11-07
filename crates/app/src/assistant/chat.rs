//! The conversation with a connected agent, a lighter rewrite of SolidCraft's agent chat state
//! (`crates/ui-egui/src/agent_chat/mod.rs` and `gate.rs`, see `ATTRIBUTION.md`). The agent runs
//! as an ACP backend from `craft-agents` and reaches the open window through its own live MCP
//! endpoint; every request from that endpoint passes [`gate`] before the editor runs it, and the
//! ones that need the person's answer wait here as [`Approval`]s.

use std::time::{Duration, Instant};

use craft_agents::{
    acp::{self, McpBridge, ModelInfo},
    agents::{Entry, Mode},
    backend::{AuthMethod, ChatBackend, ConfigOption, Event, PermissionOption, PromptPart, StartContext, ToolStatus},
};
use serde_json::Value;

use crate::live_control::{LiveControl, Request};

/// Most conversation items kept; older ones scroll away.
const MAX_ITEMS: usize = 400;
/// Longest text kept for one item, in bytes, so a runaway reply stays bounded.
const MAX_TEXT: usize = 64 * 1024;
/// How long a held tool call waits for Allow or Deny before the chat declines it.
const APPROVAL_WAIT: Duration = Duration::from_secs(acp::APPROVAL_WAIT_S);

/// `text` cut to at most `max` bytes on a character boundary, and whether it was cut.
fn capped(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

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
    /// Streamed text; `thought` is the agent's reasoning, shown dimmed. `parsed` holds the
    /// reply's Markdown once drawn, so it is parsed once rather than every frame; it is cleared
    /// whenever more text arrives and goes with the item when old items are dropped.
    Agent {
        thought: bool,
        text: String,
        parsed: std::cell::OnceCell<Vec<craft_agents::markdown::Block>>,
        /// The reply reached [`MAX_TEXT`]: the rest was dropped, and the panel says so.
        cut: bool,
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
    /// When it began waiting; a held tool call is declined after [`APPROVAL_WAIT`].
    pub(super) since: Instant,
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
    command.starts_with("file.") || matches!(command, "project.new" | "window.close" | "arrangement.delete")
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
    /// The private folder holding the endpoint's token file, apart from the agent's working
    /// folder and removed when the chat disconnects.
    token_dir: Option<tempfile::TempDir>,
    /// This connection's MCP server name ([`acp::bridge_name`]), random per chat, by which the
    /// agent's permission questions about PCBCraft's own tools are told apart from others.
    bridge_name: Option<String>,
    /// Whether a message is being answered. Tool calls run only then: once the person stops the
    /// reply or starts a new chat, calls already on their way are refused, not run.
    turn_open: bool,
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
        let key = match (entry.kind.is_api() && entry.key).then(|| super::host::endpoint_key(&entry.id, &entry.base_url)) {
            None => None,
            Some(Ok(Some(key))) => Some(key),
            Some(Ok(None)) => {
                self.state = State::Failed("No key is stored for this endpoint. Edit it in Preferences › AI to add one.".to_owned());
                return;
            }
            Some(Err(error)) => {
                self.state = State::Failed(error);
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
        let control = LiveControl::start_with_wait(ctx, Duration::from_secs(acp::ENDPOINT_WAIT_S)).ok();
        let token = control.as_ref().map(|control| (control.port(), write_token_file(control.port(), control.secret())));
        // A fresh random part per connection, so no other MCP server can share the bridge's name.
        let name = craft_control::loopback::Token::generate().ok().and_then(|nonce| nonce.secret().get(..12).map(acp::bridge_name));
        let (bridge, token_dir) = match (token, cli_path(), name) {
            (Some((port, Ok((dir, token_file)))), Some(cli), Some(name)) => (Some(McpBridge { name, cli, port, token_file }), Some(dir)),
            (_, _, None) => {
                self.items.push(Item::Notice(
                    "No random name could be made for the chat's tools, so the agent can talk but not reach the project.".to_owned(),
                ));
                (None, None)
            }
            (Some((_, Err(error))), Some(_), _) => {
                self.items
                    .push(Item::Notice(format!("Could not write the chat's token file ({error}), so the agent can talk but not reach the project.")));
                (None, None)
            }
            (None, _, _) => {
                self.items.push(Item::Notice("The chat's endpoint did not start, so the agent can talk but not reach the project.".to_owned()));
                (None, None)
            }
            (_, None, _) => {
                self.items
                    .push(Item::Notice("pcbcraft-cli was not found beside PCBCraft, so the agent can talk but not reach the project.".to_owned()));
                (None, None)
            }
        };
        let wake_ctx = ctx.clone();
        let bridge_name = bridge.as_ref().map(|bridge| bridge.name.clone());
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
                self.token_dir = token_dir;
                self.bridge_name = bridge_name;
            }
            Err(error) => self.state = State::Failed(entry.problem_hint().unwrap_or(error)),
        }
    }

    /// Attach a scripted backend with a real chat endpoint and token file, as [`Self::connect`]
    /// would: the endpoint's port and the file the bridge reads.
    #[cfg(test)]
    pub(super) fn attach_with_endpoint(&mut self, ctx: &egui::Context, backend: Box<dyn ChatBackend>, entry: Entry) -> (u16, std::path::PathBuf) {
        self.attach(backend, entry);
        let control = LiveControl::start_with_wait(ctx, Duration::from_secs(acp::ENDPOINT_WAIT_S)).unwrap_or_else(|error| panic!("{error}"));
        let (dir, token_file) = write_token_file(control.port(), control.secret()).unwrap_or_else(|error| panic!("{error}"));
        let port = control.port();
        self.control = Some(control);
        self.token_dir = Some(dir);
        self.bridge_name = Some(acp::bridge_name("0123456789ab"));
        (port, token_file)
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
        self.token_dir = None;
        self.bridge_name = None;
        self.turn_open = false;
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
        self.close_turn();
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
        self.turn_open = true;
        // This reply's requests are a generation of their own: nothing from before runs in it.
        if let Some(control) = &self.control {
            control.next_generation();
        }
        true
    }

    /// Stop the message being answered. Held tool calls are denied with it, and calls still on
    /// their way are refused when they arrive (see [`Self::turn_open`]).
    pub(super) fn stop(&mut self) {
        self.close_turn();
        if let Some(backend) = self.backend.as_mut() {
            backend.cancel();
        }
        self.deny_all("Stopped.");
    }

    /// Bring the connected agent up to its saved settings without a restart: its name, and an
    /// endpoint's request timeout and reasoning effort, which it uses from its next request on.
    pub(super) fn retune(&mut self, saved: &craft_agents::agents::AgentConfig) {
        let (Some(entry), Some(backend)) = (self.agent.as_mut(), self.backend.as_mut()) else { return };
        if entry.request_timeout_s != saved.request_timeout_s {
            backend.set_request_timeout(saved.request_timeout_s);
            entry.request_timeout_s = saved.request_timeout_s;
        }
        if entry.reasoning_effort != saved.reasoning_effort {
            backend.set_reasoning_effort(&saved.reasoning_effort);
            entry.reasoning_effort.clone_from(&saved.reasoning_effort);
        }
        entry.name.clone_from(&saved.name);
    }

    /// Whether a message is being answered, so its tool calls may run.
    pub(super) fn turn_open(&self) -> bool {
        self.turn_open
    }

    /// The endpoint's current request generation; only requests stamped with it may run.
    pub(super) fn generation(&self) -> Option<u64> {
        self.control.as_ref().map(LiveControl::current)
    }

    /// End the turn and revoke every request that arrived in it, wherever it is on its way.
    fn close_turn(&mut self) {
        self.turn_open = false;
        if let Some(control) = &self.control {
            control.next_generation();
        }
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
        let since = Instant::now();
        self.approvals.push(Approval { id, title: format!("Allow `{command}`?"), detail, source: Source::Tool { request, command }, since });
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

    /// Decline held tool calls nobody answered within [`APPROVAL_WAIT`], telling the agent the
    /// command did not run, before its connection gives up waiting. One whose caller already
    /// gave up is dropped the same way; its answer goes nowhere.
    fn decline_overdue(&mut self, now: Instant) {
        let overdue: Vec<u64> = self
            .approvals
            .iter()
            .filter(|approval| match &approval.source {
                Source::Tool { request, .. } => request.expired() || now.saturating_duration_since(approval.since) >= APPROVAL_WAIT,
                Source::Agent { .. } => false,
            })
            .map(|approval| approval.id)
            .collect();
        for id in overdue {
            if let Some(Approval { source: Source::Tool { request, command }, .. }) = self.take_approval(id) {
                let minutes = APPROVAL_WAIT.as_secs() / 60;
                request.answer(Err(format!("Nobody answered within {minutes} minutes, so `{command}` was not run.")));
                self.push(Item::Notice(format!("`{command}` waited {minutes} minutes without an answer and was declined.")));
            }
        }
    }

    /// How long until the next held tool call is declined, so the window can repaint then.
    pub(super) fn next_expiry(&self) -> Option<Duration> {
        let now = Instant::now();
        self.approvals
            .iter()
            .filter(|approval| matches!(approval.source, Source::Tool { .. }))
            .map(|approval| APPROVAL_WAIT.saturating_sub(now.saturating_duration_since(approval.since)))
            .min()
    }

    /// Advance the backend: apply its events to the conversation and the options, answer its
    /// permission questions about PCBCraft's own tools (the gate checks those calls), and drop
    /// held requests whose caller gave up. `mode` decides the agent's other questions; with
    /// `auto_bridge` off, its questions about PCBCraft's tools are asked too.
    pub(super) fn poll(&mut self, mode: Mode, auto_bridge: bool) {
        self.decline_overdue(Instant::now());
        let events = self.backend.as_mut().map(|backend| backend.poll()).unwrap_or_default();
        for event in events {
            // Events after the backend ended (a failed setup, an exit) belong to no session.
            if self.backend.is_none() {
                break;
            }
            self.event(event, mode, auto_bridge);
        }
    }

    /// Apply one backend event.
    fn event(&mut self, event: Event, mode: Mode, auto_bridge: bool) {
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
                let ours = auto_bridge && self.bridge_name.as_deref().is_some_and(|name| acp::is_bridge_tool(&title, kind.as_deref(), name));
                if ours || mode == Mode::All {
                    self.answer_agent(key, allow.as_deref());
                } else {
                    let id = self.next_id();
                    let detail = kind.map(|kind| format!("The agent wants to run a {kind} tool of its own.")).unwrap_or_default();
                    self.approvals.push(Approval { id, title, detail, source: Source::Agent { key, options }, since: Instant::now() });
                }
            }
            Event::AuthRequired { methods } => {
                self.close_turn();
                self.state = State::SignIn(methods);
            }
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
                self.close_turn();
                if self.state == State::Running {
                    self.state = State::Ready;
                }
            }
            Event::SetupFailed(reason) => {
                // The agent may keep running; end it so the panel offers Retry, as on exit.
                self.disconnect();
                self.state = State::Failed(format!("The agent could not start a session: {reason}"));
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
        if let Some(Item::Agent { thought: current, text: reply, parsed, cut }) = self.items.last_mut()
            && *current == thought
        {
            // Once cut, the reply takes nothing more, so it never skips a part and goes on.
            if !*cut {
                let (part, short) = capped(text, MAX_TEXT.saturating_sub(reply.len()));
                reply.push_str(part);
                *cut = short;
                parsed.take();
            }
            return;
        }
        let (part, cut) = capped(text, MAX_TEXT);
        self.push(Item::Agent { thought, text: part.to_owned(), parsed: Default::default(), cut });
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
                    *current_output = Some(capped(&output, MAX_TEXT).0.to_owned());
                }
            }
            None => {
                let output = output.map(|output| capped(&output, MAX_TEXT).0.to_owned());
                self.push(Item::Tool { title: title.unwrap_or_else(|| id.clone()), id, status, input, output });
            }
        }
    }

    /// Add an item, dropping the oldest beyond the limit.
    pub(super) fn push(&mut self, mut item: Item) {
        // Notices carry agent and endpoint messages; they are bounded like replies.
        if let Item::Notice(text) | Item::User(text) = &mut item {
            let (part, short) = capped(text, MAX_TEXT);
            if short {
                *text = format!("{part}…");
            }
        }
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

/// Write `secret` to `<new private folder>/<port>.token` for the agent's MCP bridge, which
/// reads it with `--token-file`, so the credential never goes in the agent's session, arguments
/// or environment. Only the user may read it: on Unix the folder is `0700` and the file `0600`;
/// on Windows the file's inherited permissions are replaced by one entry for the user
/// ([`restrict_to_user`]) while it is still empty, since the temporary folder may be shared.
/// Dropping the folder removes both.
fn write_token_file(port: u16, secret: &str) -> std::io::Result<(tempfile::TempDir, std::path::PathBuf)> {
    #[cfg(windows)]
    let restrict = restrict_to_user;
    #[cfg(not(windows))]
    let restrict = |_: &std::path::Path| Ok(());
    write_token_file_with(port, secret, restrict)
}

/// [`write_token_file`], with `restrict` limiting the new, empty file to the user (Windows). When
/// it fails, the file is removed and no secret is written.
fn write_token_file_with(
    port: u16,
    secret: &str,
    restrict: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> std::io::Result<(tempfile::TempDir, std::path::PathBuf)> {
    use std::io::Write;
    let dir = tempfile::Builder::new().prefix("pcbcraft-control-").tempdir()?;
    let path = dir.path().join(format!("{port}.token"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = restrict(&path) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    file.write_all(secret.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok((dir, path))
}

/// How long `icacls` may take. It finishes at once, but on some computers security software
/// leaves the new process suspended, and waiting without a limit would freeze the window.
#[cfg(windows)]
const ICACLS_LIMIT: Duration = Duration::from_secs(5);

/// Windows: drop the file's inherited entries and grant only the current user full access, with
/// the system's own `icacls` (by full path, so nothing on PATH stands in for it). As SolidCraft's
/// token file does, but failing closed: an `icacls` that fails or doesn't finish in time leaves
/// no token, since nothing is known about who else may read the temporary folder.
#[cfg(windows)]
fn restrict_to_user(path: &std::path::Path) -> std::io::Result<()> {
    use std::{io::Error, os::windows::process::CommandExt};
    let user = std::env::var("USERNAME").map_err(|_| Error::other("USERNAME is not set"))?;
    let who = match std::env::var("USERDOMAIN").ok().filter(|domain| !domain.is_empty()) {
        Some(domain) => format!("{domain}\\{user}"),
        None => user,
    };
    let system = std::env::var_os("SystemRoot").ok_or_else(|| Error::other("SystemRoot is not set"))?;
    let icacls = std::path::Path::new(&system).join("System32").join("icacls.exe");
    let mut child = std::process::Command::new(icacls)
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("{who}:F")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => return Err(Error::other(format!("icacls could not restrict {}", path.display()))),
            None if start.elapsed() >= ICACLS_LIMIT => {
                let _ = child.kill();
                return Err(Error::new(std::io::ErrorKind::TimedOut, "icacls did not finish in time"));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
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
        for command in ["file.save", "project.new", "window.close", "arrangement.delete"] {
            assert_eq!(run(Mode::Edits, command), Verdict::Ask, "{command}");
            assert_eq!(run(Mode::All, command), Verdict::Run, "{command}");
        }
        assert_eq!(gate(Mode::Ask, "engine.execute", Some("file.save"), |command| command == "file.save"), Verdict::Run);
    }

    /// A reply's parsed Markdown is kept until more of it streams in, so drawing doesn't reparse it.
    #[test]
    fn streamed_text_clears_the_parsed_reply() {
        let mut chat = Chat::default();
        chat.stream(false, "**Hello**");
        let parsed = |chat: &Chat| match chat.items.last() {
            Some(Item::Agent { parsed, .. }) => parsed.get().cloned(),
            _ => None,
        };
        if let Some(Item::Agent { text, parsed, .. }) = chat.items.last() {
            parsed.get_or_init(|| craft_agents::markdown::parse(text));
        }
        assert!(parsed(&chat).is_some());
        chat.stream(false, " world");
        assert_eq!(parsed(&chat), None, "new text is parsed again when drawn");
        assert!(matches!(chat.items.as_slice(), [Item::Agent { text, .. }] if text == "**Hello** world"));
    }

    /// Replies are capped in bytes on a character boundary, from their first chunk on, and once
    /// cut take nothing more; notices are capped the same way.
    #[test]
    fn long_replies_are_cut_once_in_bytes() {
        let mut chat = Chat::default();
        chat.stream(false, &"ä".repeat(MAX_TEXT));
        let reply = |chat: &Chat| match chat.items.last() {
            Some(Item::Agent { text, cut, .. }) => Some((text.len(), *cut)),
            _ => None,
        };
        assert_eq!(reply(&chat), Some((MAX_TEXT, true)), "two-byte characters fill exactly the cap");
        chat.stream(false, "more");
        assert_eq!(reply(&chat), Some((MAX_TEXT, true)), "a cut reply takes nothing more");

        let mut chat = Chat::default();
        chat.stream(false, &"a".repeat(MAX_TEXT - 3));
        chat.stream(false, "ääää");
        assert_eq!(reply(&chat), Some((MAX_TEXT - 1, true)), "the part that fits is kept, on a character boundary");
        chat.stream(false, "b");
        assert_eq!(reply(&chat), Some((MAX_TEXT - 1, true)), "no later chunk fills the gap");

        chat.push(Item::Notice("x".repeat(MAX_TEXT * 2)));
        assert!(matches!(chat.items.last(), Some(Item::Notice(text)) if text.len() <= MAX_TEXT + '…'.len_utf8()));
    }

    /// Only the bridge's real tools are taken for its tools: the list in `craft-agents` is the
    /// MCP server's own.
    #[test]
    fn the_bridge_tool_list_matches_the_mcp_server() {
        let served: Vec<String> = pcbcraft_mcp::tool_definitions().iter().filter_map(|tool| tool["name"].as_str().map(str::to_owned)).collect();
        let mut listed: Vec<&str> = acp::BRIDGE_TOOLS.to_vec();
        let mut served: Vec<&str> = served.iter().map(String::as_str).collect();
        listed.sort_unstable();
        served.sort_unstable();
        assert_eq!(listed, served);
    }

    /// The token file holds the secret, is the user's alone and goes when its folder is dropped.
    #[test]
    fn the_token_file_is_private_and_removed_with_its_folder() -> std::io::Result<()> {
        let (dir, path) = write_token_file(41234, "0123456789abcdef0123456789abcdef")?;
        assert_eq!(std::fs::read_to_string(&path)?, "0123456789abcdef0123456789abcdef\n");
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("41234.token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.path())?.permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        }
        drop(dir);
        assert!(!path.exists());
        Ok(())
    }

    /// The file is restricted while empty; when that fails, it is removed and no secret is written.
    #[test]
    fn a_token_file_that_could_not_be_restricted_gets_no_secret() {
        let mut seen = None;
        let result = write_token_file_with(41234, "0123456789abcdef0123456789abcdef", |path| {
            seen = Some((path.to_path_buf(), std::fs::read(path)));
            Err(std::io::Error::other("icacls could not restrict it"))
        });
        assert!(result.is_err());
        let (path, contents) = seen.unwrap_or_else(|| panic!("the file was not restricted"));
        assert_eq!(contents.ok(), Some(Vec::new()), "restricted before the secret goes in");
        assert!(!path.exists(), "an unrestricted file is removed");
    }
}
