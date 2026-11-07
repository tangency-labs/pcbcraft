//! The ACP backend (desktop only): starts the agent the user picked as a child process and holds
//! the protocol's side of the conversation (initialize, the session, prompts, permission questions,
//! sign-in). The panel sees only [`Event`]s.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::acp::{self, Incoming};
use super::backend::{ChatBackend, Event, PromptPart, StartContext, TestError};
use super::process::{AgentProcess, FromAgent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Initialize,
    SessionNew,
    Prompt,
    Authenticate,
    SetMode,
    SetModel,
    SetConfig,
}

/// Give a cancelled prompt time to finish before disconnecting an unresponsive agent.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// How long the agent may take to answer `initialize` or `session/new` before setup fails and
/// the panel offers Retry. Signing in (`authenticate`), which waits for the person, has no limit.
const SETUP_LIMIT: Duration = Duration::from_secs(120);

/// Variables the agent doesn't inherit: a control token set for the app's own `--control` port
/// would otherwise reach the agent and outrank the chat endpoint's own token file.
const ENV_REMOVE: &[&str] = &["PCBCRAFT_CONTROL_TOKEN"];

/// A chat backend that drives an installed agent over the Agent Client Protocol, as a child
/// process speaking JSON-RPC on its stdin and stdout.
pub struct AcpBackend {
    proc: AgentProcess,
    cwd: std::path::PathBuf,
    bridge: Option<acp::McpBridge>,
    /// The agent's `initialize` reply: which routes the system note can take into a session.
    init: Value,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    session: Option<String>,
    agent: String,
    images: bool,
    documents: bool,
    auth: Vec<acp::AuthMethod>,
    /// Open permission questions: our key → the agent's request id.
    permissions: HashMap<u64, Value>,
    next_key: u64,
    cancelling: Option<(u64, Instant)>,
    exited: bool,
    config_pending: Option<(u64, String)>,
    modern_config: bool,
    /// When the `initialize` or `session/new` request waiting for its reply was sent.
    setup_since: Option<Instant>,
}

impl AcpBackend {
    /// Start `argv` and say hello (`initialize`); the session follows when the agent answers.
    pub fn start(argv: &[String], ctx: StartContext) -> Result<AcpBackend, String> {
        let wake = ctx.wake;
        let proc = AgentProcess::spawn(argv, &ctx.cwd, ENV_REMOVE, wake)?;
        // An agent inside WSL is told the same folder and bridge as WSL shows them (#5546).
        let (cwd, bridge) =
            if acp::runs_in_wsl(argv) { (acp::wsl_path(&ctx.cwd), ctx.bridge.as_ref().map(acp::McpBridge::in_wsl)) } else { (ctx.cwd, ctx.bridge) };
        let mut b = AcpBackend {
            proc,
            cwd,
            bridge,
            init: Value::Null,
            next_id: 0,
            pending: HashMap::new(),
            session: None,
            agent: String::new(),
            images: false,
            documents: false,
            auth: Vec::new(),
            permissions: HashMap::new(),
            next_key: 0,
            cancelling: None,
            exited: false,
            config_pending: None,
            modern_config: false,
            setup_since: None,
        };
        b.request(Pending::Initialize, "initialize", acp::initialize_params(env!("CARGO_PKG_VERSION")));
        Ok(b)
    }

    /// Send a request and remember what its reply answers; a setup request starts the setup clock.
    fn request(&mut self, what: Pending, method: &str, params: Value) {
        if matches!(what, Pending::Initialize | Pending::SessionNew) {
            self.setup_since = Some(Instant::now());
        }
        self.next_id += 1;
        self.pending.insert(self.next_id, what);
        self.proc.send(&acp::request(self.next_id, method, params));
    }

    fn new_session(&mut self) {
        self.config_pending = None;
        self.pending.retain(|_, request| *request != Pending::SetConfig);
        let params = acp::session_new_params(&self.cwd, self.bridge.as_ref(), &self.init);
        self.request(Pending::SessionNew, "session/new", params);
    }

    /// Apply one message from the agent: match a reply to its request, or pass on what it streams and asks.
    fn handle(&mut self, msg: Incoming, out: &mut Vec<Event>) {
        match msg {
            Incoming::Reply { id, result } => {
                let Some(what) = self.pending.remove(&id) else { return };
                if matches!(what, Pending::Initialize | Pending::SessionNew) {
                    self.setup_since = None;
                }
                if self.cancelling.is_some_and(|(request, _)| request == id) {
                    self.cancelling = None;
                }
                match (what, result) {
                    (Pending::Initialize, Ok(v)) => {
                        self.agent = acp::agent_info(&v);
                        self.images = acp::takes_images(&v);
                        self.documents = acp::takes_documents(&v);
                        self.auth = acp::auth_methods(&v);
                        self.init = v;
                        self.new_session();
                    }
                    (Pending::SessionNew, Ok(v)) => match v.get("sessionId").and_then(Value::as_str) {
                        Some(s) => {
                            // New chat has opened successfully: retire the old turn before Ready.
                            // Its late reply and cancellation deadline must not end the new turn.
                            if let Some((request, _)) = self.cancelling.take() {
                                self.pending.remove(&request);
                                out.push(Event::TurnEnded { reason: "cancelled".into() });
                            }
                            self.session = Some(s.to_string());
                            match acp::config_options(&v) {
                                acp::ConfigParse::Snapshot(options) => {
                                    self.modern_config = true;
                                    out.push(Event::ConfigOptions(options));
                                }
                                _ => {
                                    self.modern_config = false;
                                    if let Some((current, available)) = acp::modes(&v) {
                                        out.push(Event::Modes { current, available });
                                    }
                                    if let Some((current, available)) = acp::models(&v) {
                                        out.push(Event::Models { current, available });
                                    }
                                }
                            }
                            out.push(Event::Documents(self.documents));
                            out.push(Event::Ready { agent: self.agent.clone(), images: self.images });
                        }
                        None => out.push(Event::SetupFailed("the agent opened no session (its reply had no sessionId)".into())),
                    },
                    (Pending::Prompt, Ok(v)) => {
                        let reason = v.get("stopReason").and_then(Value::as_str).unwrap_or("");
                        out.push(Event::TurnEnded { reason: if reason == "end_turn" { String::new() } else { reason.to_string() } });
                    }
                    // Signed in: try the session again.
                    (Pending::Authenticate, Ok(_)) => self.new_session(),
                    (Pending::SetMode | Pending::SetModel, Ok(_)) => {}
                    (Pending::SetConfig, Ok(v)) => {
                        if let Some((request_id, config_id)) = self.config_pending.take()
                            && request_id == id
                        {
                            let result = match acp::config_options(&v) {
                                acp::ConfigParse::Snapshot(o) => Ok(o),
                                _ => Err("The agent returned no valid configuration options.".into()),
                            };
                            out.push(Event::ConfigOptionFinished { id: config_id, result });
                        }
                    }
                    // `session/set_model` is still unstable in ACP: an agent can list `models` and
                    // still not serve it (#9970). It keeps its own model; that is no error to show.
                    (Pending::SetModel, Err((acp::METHOD_NOT_FOUND, message))) => {
                        log::debug!("agent does not serve session/set_model: {message}");
                    }
                    (what, Err((code, message))) => {
                        let config_error = what == Pending::SetConfig;
                        if what == Pending::SetConfig
                            && let Some((request_id, config_id)) = self.config_pending.take()
                            && request_id == id
                        {
                            out.push(Event::ConfigOptionFinished { id: config_id, result: Err(message.clone()) });
                        }
                        let auth = acp::needs_sign_in(code, &message);
                        if auth {
                            self.session = None;
                            self.modern_config = false;
                            self.config_pending = None;
                            self.pending.retain(|_, request| *request != Pending::SetConfig);
                            if what == Pending::Prompt {
                                out.push(Event::TurnEnded { reason: "auth".into() });
                            }
                            // Say why, unless it is only the protocol's generic wording, which the
                            // sign-in prompt already says. A failed sign-in always says why.
                            if what == Pending::Authenticate {
                                out.push(Event::Error(format!("Sign-in failed: {message}")));
                            } else if !message.trim().is_empty() && !message.trim().eq_ignore_ascii_case("authentication required") {
                                out.push(Event::AuthStatus(message));
                            }
                            out.push(Event::AuthRequired { methods: self.auth.clone() });
                        } else if what == Pending::SetModel {
                            out.push(Event::ModelRejected(message));
                        } else if matches!(what, Pending::Initialize | Pending::SessionNew) {
                            out.push(Event::SetupFailed(message));
                        } else if what == Pending::Authenticate {
                            // The agent still runs: offer its sign-in methods again, with the reason.
                            out.push(Event::Error(format!("Sign-in failed: {message}")));
                            out.push(Event::AuthRequired { methods: self.auth.clone() });
                        } else if !config_error {
                            if what == Pending::Prompt {
                                out.push(Event::TurnEnded { reason: "error".into() });
                            }
                            out.push(Event::Error(message));
                        }
                    }
                }
            }
            Incoming::Text { thought, text } => out.push(Event::Text { thought, text }),
            Incoming::Tool { id, title, kind, status, input, output } => out.push(Event::Tool { id, title, kind, status, input, output }),
            Incoming::Plan(p) => out.push(Event::Plan(p)),
            Incoming::Permission { rpc, title, kind, options } => {
                self.next_key += 1;
                self.permissions.insert(self.next_key, rpc);
                out.push(Event::Permission { key: self.next_key, title, kind, options });
            }
            Incoming::AuthStatus(s) => out.push(Event::AuthStatus(s)),
            Incoming::ModeChanged(m) if !self.modern_config => out.push(Event::Modes { current: m, available: Vec::new() }),
            Incoming::ModelChanged(m) if !self.modern_config => out.push(Event::Models { current: m, available: Vec::new() }),
            Incoming::ConfigOptions { session, options } if self.session.as_deref() == Some(session.as_str()) => {
                out.push(Event::ConfigOptions(options))
            }
            Incoming::ModeChanged(_) | Incoming::ModelChanged(_) | Incoming::ConfigOptions { .. } => {}
            Incoming::Unsupported { rpc, method } => {
                self.proc.send(&acp::reply_error(rpc, acp::METHOD_NOT_FOUND, &format!("PCBCraft does not serve `{method}`")));
            }
            Incoming::Ignored => {}
        }
    }

    fn finish_exit(&mut self, tail: Vec<String>) -> Event {
        self.exited = true;
        self.session = None;
        self.pending.clear();
        self.cancelling = None;
        self.config_pending = None;
        Event::Exited { tail }
    }

    /// What arrived by `now` (at most 512 lines), ending an agent that ignored a cancel past the grace period.
    fn poll_at(&mut self, now: Instant) -> Vec<Event> {
        let mut out = Vec::new();
        if self.exited {
            return out;
        }
        for _ in 0..512 {
            match self.proc.from_agent.try_recv() {
                Ok(FromAgent::Line(l)) => {
                    if let Some(m) = acp::parse(&l) {
                        self.handle(m, &mut out);
                    }
                }
                Ok(FromAgent::Exited(tail)) => {
                    out.push(self.finish_exit(tail));
                    break;
                }
                Err(_) => break,
            }
        }
        // Drain replies first: a setup reply or acknowledgement already waiting counts.
        if self.setup_since.is_some_and(|since| now.saturating_duration_since(since) >= SETUP_LIMIT) {
            self.setup_since = None;
            self.pending.retain(|_, request| !matches!(request, Pending::Initialize | Pending::SessionNew));
            out.push(Event::SetupFailed(format!("the agent didn't answer within {} seconds", SETUP_LIMIT.as_secs())));
        }
        if self.cancelling.is_some_and(|(_, started)| now.saturating_duration_since(started) >= CANCEL_GRACE) {
            self.proc.shutdown(false);
            out.push(self.finish_exit(vec![crate::i18n::tr("ui.agent_chat_cancel_timeout").to_string()]));
        }
        out
    }
}

impl ChatBackend for AcpBackend {
    fn poll(&mut self) -> Vec<Event> {
        self.poll_at(Instant::now())
    }

    /// Send a message in the current session; nothing happens before one exists.
    fn prompt(&mut self, parts: &[PromptPart]) {
        if let Some(s) = self.session.clone() {
            self.request(Pending::Prompt, "session/prompt", acp::prompt_params(&s, parts));
        }
    }

    /// Cancel open permission questions and the running prompt, starting the grace period.
    fn cancel(&mut self) {
        // Open permission questions are answered "cancelled" first, as the protocol asks.
        for (_, rpc) in self.permissions.drain() {
            self.proc.send(&acp::permission_reply(rpc, None));
        }
        if let Some(s) = &self.session {
            self.proc.send(&acp::notification("session/cancel", json!({"sessionId": s})));
            // Repeated Stop presses must not postpone the fallback. An idle session needs none.
            if self.cancelling.is_none() {
                self.cancelling =
                    self.pending.iter().find_map(|(&id, request)| (*request == Pending::Prompt).then_some(id)).map(|id| (id, Instant::now()));
            }
        }
    }

    /// Cancel a running prompt and open a fresh session on the same agent.
    fn new_session(&mut self) {
        if self.pending.values().any(|p| *p == Pending::Prompt) {
            self.cancel();
        }
        self.session = None;
        self.modern_config = false;
        self.config_pending = None;
        self.pending.retain(|_, request| *request != Pending::SetConfig);
        self.new_session();
    }

    /// Answer the permission question `key` with `option` (`None`: cancelled).
    fn answer_permission(&mut self, key: u64, option: Option<&str>) {
        if let Some(rpc) = self.permissions.remove(&key) {
            self.proc.send(&acp::permission_reply(rpc, option));
        }
    }

    fn authenticate(&mut self, method: &str) {
        self.request(Pending::Authenticate, "authenticate", json!({"methodId": method}));
    }

    /// Ask the agent to switch the session to `mode`.
    fn set_mode(&mut self, mode: &str) {
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetMode, "session/set_mode", json!({"sessionId": s, "modeId": mode}));
        }
    }

    /// Ask the agent to use `model` in the session.
    fn set_model(&mut self, model: &str) {
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetModel, "session/set_model", acp::set_model_params(&s, model));
        }
    }

    /// Ask the agent to set a configuration option, one change at a time.
    fn set_config_option(&mut self, id: &str, value: &str) {
        if self.config_pending.is_some() {
            return;
        }
        if let Some(s) = self.session.clone() {
            self.request(Pending::SetConfig, "session/set_config_option", acp::set_config_option_params(&s, id, value));
            self.config_pending = Some((self.next_id, id.to_string()));
        }
    }

    fn pid(&self) -> Option<u32> {
        self.proc.pid()
    }

    fn shutdown(&mut self, wait: bool) {
        self.proc.shutdown(wait);
    }
}

/// Test connection: start the agent, say hello, open a session without tools, end it. What it
/// answered (its name and version), or what went wrong (it isn't signed in, it didn't answer).
pub fn test_agent(argv: &[String], cwd: &std::path::Path) -> Result<String, TestError> {
    let mut p = AgentProcess::spawn(argv, cwd, ENV_REMOVE, || {}).map_err(TestError::Other)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let wait_for = |p: &AgentProcess, id: u64| -> Result<Value, TestError> {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match p.from_agent.recv_timeout(left) {
                Ok(FromAgent::Line(l)) => {
                    if let Some(Incoming::Reply { id: got, result }) = acp::parse(&l)
                        && got == id
                    {
                        return result.map_err(|(code, m)| if acp::needs_sign_in(code, &m) { TestError::AuthRequired } else { TestError::Other(m) });
                    }
                }
                Ok(FromAgent::Exited(tail)) => {
                    return Err(TestError::Other(tail.last().cloned().unwrap_or_else(|| crate::i18n::tr("ui.agent_chat_exited").to_string())));
                }
                Err(_) => return Err(TestError::Other(crate::i18n::tr("ui.ai_no_answer").to_string())),
            }
        }
    };
    p.send(&acp::request(1, "initialize", acp::initialize_params(env!("CARGO_PKG_VERSION"))));
    let init = wait_for(&p, 1);
    let session_cwd = if acp::runs_in_wsl(argv) { acp::wsl_path(cwd) } else { cwd.to_path_buf() };
    let result = init.and_then(|v| {
        p.send(&acp::request(2, "session/new", acp::session_new_params(&session_cwd, None, &v)));
        wait_for(&p, 2).map(|_| {
            let who = acp::agent_info(&v);
            crate::i18n::format("ui.ai_test_acp_ok", &[("agent", if who.is_empty() { "?" } else { &who })])
        })
    });
    p.shutdown(true);
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::backend::StartContext;

    /// A backend with an agent that answered `initialize` and opened session `s1`.
    fn ready_backend() -> Result<AcpBackend, String> {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx)?;
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        Ok(backend)
    }

    /// An unresponsive cancel disconnects the agent.
    #[test]
    fn an_unresponsive_cancel_disconnects_the_agent() {
        let mut backend = ready_backend().unwrap();
        backend.prompt(&[PromptPart::Text("stalled search".into())]);
        backend.cancel();
        let started = backend.cancelling.unwrap().1;
        assert!(backend.poll_at(started + CANCEL_GRACE - Duration::from_millis(1)).is_empty());
        backend.cancel();
        let events = backend.poll_at(started + CANCEL_GRACE);
        assert!(matches!(events.as_slice(), [Event::Exited { .. }]), "Stop must not wait indefinitely: {events:?}");
        assert!(backend.pid().is_none());
        assert!(backend.session.is_none() && backend.pending.is_empty());
        assert!(backend.poll().is_empty(), "the exit is reported only once");
    }

    /// A cancelled prompt reply keeps the connection.
    #[test]
    fn a_cancelled_prompt_reply_keeps_the_connection() {
        for reply in [Ok(json!({"stopReason":"cancelled"})), Err((-32603, "operation aborted".into()))] {
            let mut backend = ready_backend().unwrap();
            backend.prompt(&[PromptPart::Text("search".into())]);
            let request = backend.next_id;
            backend.cancel();
            let mut events = Vec::new();
            backend.handle(Incoming::Reply { id: request, result: reply }, &mut events);
            assert!(events.iter().any(|e| matches!(e, Event::TurnEnded { .. })));
            assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty());
            assert!(backend.pid().is_some() && !backend.exited);
            backend.shutdown(true);
        }
    }

    /// Cancelling an idle session does not disconnect it.
    #[test]
    fn cancelling_an_idle_session_does_not_disconnect_it() {
        let mut backend = ready_backend().unwrap();
        backend.cancel();
        assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty());
        assert!(backend.pid().is_some() && !backend.exited);
        backend.shutdown(true);
    }

    /// A new session retires the old cancelled prompt.
    #[test]
    fn a_new_session_retires_the_old_cancelled_prompt() {
        let mut backend = ready_backend().unwrap();
        backend.prompt(&[PromptPart::Text("old search".into())]);
        let old_prompt = backend.next_id;
        backend.cancel();
        ChatBackend::new_session(&mut backend);
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId":"s2"})) }, &mut events);
        assert!(matches!(events.first(), Some(Event::TurnEnded { reason }) if reason == "cancelled"));
        assert!(matches!(events.last(), Some(Event::Ready { .. })));
        backend.prompt(&[PromptPart::Text("new search".into())]);
        assert!(backend.poll_at(Instant::now() + CANCEL_GRACE).is_empty(), "a new session has no stale cancellation deadline");
        backend.cancel();
        events.clear();
        backend.handle(Incoming::Reply { id: old_prompt, result: Ok(json!({"stopReason":"cancelled"})) }, &mut events);
        assert!(events.is_empty(), "an old reply must not end the new turn or clear its cancellation deadline");
        assert!(matches!(backend.poll_at(Instant::now() + CANCEL_GRACE).as_slice(), [Event::Exited { .. }]));
    }

    fn snapshot(session: &str, value: &str) -> Value {
        json!({"sessionId":session,"configOptions":[{"id":"provider.model","category":"model","name":"Model","type":"select","currentValue":value,"options":[{"value":"fast","name":"Fast"},{"value":"smart","name":"Smart"}]}]})
    }

    /// Config updates and replies are isolated to the active session.
    #[test]
    fn config_updates_and_replies_are_isolated_to_the_active_session() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(snapshot("s1", "fast")) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s1"));
        events.clear();
        backend.set_config_option("provider.model", "smart");
        let old_request = backend.config_pending.as_ref().map(|p| p.0).unwrap();
        ChatBackend::new_session(&mut backend);
        let new_session_request = backend.next_id;
        assert!(backend.config_pending.is_none());
        let old_notification = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"config_option_update","configOptions":[]}}}"#;
        backend.handle(acp::parse(old_notification).unwrap(), &mut events);
        assert!(events.is_empty());
        backend.handle(Incoming::Reply { id: new_session_request, result: Ok(snapshot("s2", "fast")) }, &mut events);
        assert_eq!(backend.session.as_deref(), Some("s2"));
        events.clear();
        backend.set_config_option("provider.model", "smart");
        let new_request = backend.config_pending.as_ref().map(|p| p.0).unwrap();
        backend.handle(Incoming::Reply { id: old_request, result: Ok(snapshot("s1", "smart")) }, &mut events);
        backend.handle(Incoming::Reply { id: old_request, result: Err((-32001, "old rejected request".into())) }, &mut events);
        backend.handle(acp::parse(old_notification).unwrap(), &mut events);
        assert!(events.is_empty(), "old requests and session notifications are ignored");
        assert_eq!(backend.config_pending.as_ref().map(|p| p.0), Some(new_request), "an old reply cannot clear the new session's pending request");
        let new_notification = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s2","update":{"sessionUpdate":"config_option_update","configOptions":[]}}}"#;
        backend.handle(acp::parse(new_notification).unwrap(), &mut events);
        assert!(matches!(events.as_slice(), [Event::ConfigOptions(options)] if options.is_empty()));
        assert_eq!(backend.config_pending.as_ref().map(|p| p.0), Some(new_request), "a notification is not an acknowledgement");
        events.clear();
        backend.handle(Incoming::Reply { id: new_request, result: Ok(snapshot("s2", "smart")) }, &mut events);
        assert!(backend.config_pending.is_none());
        assert!(matches!(events.as_slice(), [Event::ConfigOptionFinished { result: Ok(options), .. }] if options[0].current_value == "smart"));
        backend.shutdown(true);
    }

    /// A refused model is the picker's business, not an error of the chat; signing in stays as it was.
    #[test]
    fn a_refused_model_is_reported_as_such() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({"sessionId":"s1"})) }, &mut events);
        events.clear();
        backend.set_model("zzz");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32602, "no such model".into())) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::ModelRejected(message)] if message == "no such model"), "{events:?}");
        events.clear();
        backend.set_model("fast");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({})) }, &mut events);
        assert!(events.is_empty(), "{events:?}");
        backend.set_model("zzz");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((acp::AUTH_REQUIRED, "Authentication required".into())) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::AuthRequired { .. }]), "{events:?}");
        backend.shutdown(true);
    }

    /// Setup that can't finish fails, so the panel offers Retry: a session reply without an ID, or
    /// an agent silent past the setup limit. A failed sign-in offers the methods again instead,
    /// since the agent still runs.
    #[test]
    fn setup_that_cannot_finish_fails_and_a_failed_sign_in_asks_again() {
        let start = || {
            let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
            AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx).unwrap()
        };
        let mut backend = start();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Ok(json!({})) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::SetupFailed(_)]), "{events:?}");
        backend.shutdown(true);

        let mut backend = start();
        assert!(backend.poll_at(Instant::now() + SETUP_LIMIT / 2).is_empty());
        let late = backend.poll_at(Instant::now() + SETUP_LIMIT + Duration::from_secs(1));
        assert!(matches!(late.as_slice(), [Event::SetupFailed(reason)] if reason.contains("didn't answer")), "{late:?}");
        assert!(backend.poll_at(Instant::now() + SETUP_LIMIT * 3).is_empty(), "it fails once");
        backend.shutdown(true);

        let mut backend = start();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Err((acp::AUTH_REQUIRED, "sign in".into())) }, &mut events);
        events.clear();
        backend.authenticate("browser");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32603, "the browser could not open".into())) }, &mut events);
        assert!(
            matches!(events.as_slice(), [Event::Error(message), Event::AuthRequired { .. }] if message.contains("browser could not open")),
            "{events:?}"
        );
        assert!(backend.poll_at(Instant::now() + SETUP_LIMIT * 3).is_empty(), "signing in has no time limit");
        // A failed sign-in whose reason is itself about signing in still says why.
        events.clear();
        backend.authenticate("browser");
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32603, "authentication failed: code expired".into())) }, &mut events);
        assert!(
            matches!(events.as_slice(), [Event::Error(message), Event::AuthRequired { .. }] if message == "Sign-in failed: authentication failed: code expired"),
            "{events:?}"
        );
        backend.shutdown(true);
    }

    /// A reply that needs sign-in says why when the agent says more than the protocol's generic
    /// "Authentication required", which the sign-in prompt already shows.
    #[test]
    fn sign_in_requests_keep_the_agents_reason() {
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx).unwrap();
        let mut events = Vec::new();
        backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
        backend.handle(Incoming::Reply { id: 2, result: Err((acp::AUTH_REQUIRED, "Authentication required".into())) }, &mut events);
        assert!(matches!(events.as_slice(), [Event::AuthRequired { .. }]), "{events:?}");
        events.clear();
        backend.new_session();
        backend.handle(Incoming::Reply { id: backend.next_id, result: Ok(json!({"sessionId": "s1"})) }, &mut events);
        events.clear();
        backend.prompt(&[PromptPart::Text("hi".into())]);
        backend.handle(Incoming::Reply { id: backend.next_id, result: Err((-32603, "401 Unauthorized: token expired".into())) }, &mut events);
        assert!(
            matches!(events.as_slice(), [Event::TurnEnded { .. }, Event::AuthStatus(reason), Event::AuthRequired { .. }] if reason.contains("token expired")),
            "{events:?}"
        );
        backend.shutdown(true);
    }

    /// An agent that refuses the handshake for another reason fails setup, so the panel can offer
    /// Retry, while an error that only mentions an author is no sign-in request.
    #[test]
    fn a_refused_handshake_fails_setup() {
        for (id, reply) in [(1, Err((-32603, "Unknown author in config".to_owned()))), (2, Err((-32603, "no workspace".to_owned())))] {
            let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
            let mut backend = AcpBackend::start(&["sh".into(), "-c".into(), "cat >/dev/null".into()], ctx).unwrap();
            let mut events = Vec::new();
            if id == 2 {
                backend.handle(Incoming::Reply { id: 1, result: Ok(json!({"agentInfo":{"name":"fake"}})) }, &mut events);
                events.clear();
            }
            let expected = reply.clone().err().map(|(_, message)| message);
            backend.handle(Incoming::Reply { id, result: reply.map(|()| Value::Null) }, &mut events);
            assert!(matches!(events.as_slice(), [Event::SetupFailed(message)] if Some(message) == expected.as_ref()), "{events:?}");
            backend.shutdown(true);
        }
    }
}
