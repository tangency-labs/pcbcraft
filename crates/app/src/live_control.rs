//! Desktop opt-in for live MCP. The socket worker hands requests to the editor thread.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, ScopedJoinHandle, Thread},
    time::{Duration, Instant},
};

use craft_control::loopback::{AcceptedPeer, Listener, Token};
use pcbcraft_engine::Session;
use pcbcraft_mcp::live::{Editor, TOKEN_ENV, call_editor};
use serde_json::{Value, json};

use crate::{Dialog, PcbApp, window_commands};

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const EDITOR_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_POLL: Duration = Duration::from_millis(20);
const MAX_PEERS: usize = 4;

/// One editor window's local endpoint. Dropping it revokes the port and credential.
pub(super) struct LiveControl {
    port: u16,
    token: Arc<Token>,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
    /// The current request generation ([`Self::next_generation`]); each request is stamped with
    /// it when it authenticates.
    generation: Arc<AtomicU64>,
    worker: Thread,
}

/// Keep the response channel and deadline with the request until the editor sees it.
pub(super) struct Request {
    body: Value,
    deadline: Instant,
    reply: SyncSender<Value>,
    /// The endpoint's generation when the request authenticated.
    generation: u64,
    connected: Arc<AtomicBool>,
}

impl Request {
    /// The authenticated request, as `{"method": ..., "params": ...}`.
    pub(super) fn body(&self) -> &Value {
        &self.body
    }

    /// The endpoint's generation when this request arrived, for [`LiveControl::current`].
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the peer has given up waiting, so an answer would be dropped.
    pub(super) fn expired(&self) -> bool {
        Instant::now() >= self.deadline || !self.connected.load(Ordering::Acquire)
    }

    /// Send the editor's answer. A peer that has already given up is ignored.
    pub(super) fn answer(self, result: Result<Value, String>) {
        let _ = self.reply.try_send(match result {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        });
    }
}

impl std::fmt::Debug for LiveControl {
    /// Identify the endpoint in diagnostics without logging its credential.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("LiveControl").field("port", &self.port).finish_non_exhaustive()
    }
}

impl LiveControl {
    /// Bind a fresh loopback endpoint and start the socket worker after explicit opt-in.
    pub(super) fn start(ctx: &egui::Context) -> io::Result<Self> {
        Self::start_with_wait(ctx, EDITOR_TIMEOUT)
    }

    /// Like [`Self::start`], but each request may wait up to `wait` for the editor: long enough
    /// for the in-app chat to hold one until the person allows or denies it. The peer gives up
    /// at the same time, so a late answer is dropped.
    pub(super) fn start_with_wait(ctx: &egui::Context, wait: Duration) -> io::Result<Self> {
        let listener = Listener::bind(0)?;
        listener.set_nonblocking(true)?;
        let port = listener.port()?;
        let token = Arc::new(Token::generate()?);
        let stop = Arc::new(AtomicBool::new(false));
        let (send, requests) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_token = Arc::clone(&token);
        let worker_stop = Arc::clone(&stop);
        let worker_generation = Arc::clone(&generation);
        let worker = thread::Builder::new().name("pcbcraft-live-mcp".to_owned()).spawn(move || {
            listen(listener, &worker_token, &worker_stop, &worker_generation, &send, &ctx, wait);
            drop(send);
            ctx.request_repaint();
        })?;
        Ok(Self { port, token, requests, stop, generation, worker: worker.thread().clone() })
    }

    /// Start a new request generation, revoking every request that arrived before: the chat
    /// calls it when a reply starts, stops or the chat resets, so a request belonging to an
    /// earlier turn never runs in a later one, however it was delayed on its way.
    pub(super) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// The current request generation.
    pub(super) fn current(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Retain revocation identity for queued input even if a replacement listener reuses its port.
    pub(super) fn automation_owner(&self, chat: bool, generation: u64) -> crate::ui_automation::Owner {
        crate::ui_automation::Owner { port: self.port, generation, chat, stopped: self.stop.clone() }
    }

    /// The endpoint's port, for a bridge the app configures itself.
    pub(super) fn port(&self) -> u16 {
        self.port
    }

    /// The endpoint's credential, for a bridge the app configures itself. Never log it.
    pub(super) fn secret(&self) -> &str {
        self.token.secret()
    }

    /// Build a configuration for clients that accept an `mcpServers` object.
    pub(super) fn client_configuration(&self) -> String {
        let config = json!({
            "mcpServers": {
                "pcbcraft-live": {
                    "command": "pcbcraft-cli",
                    "args": ["mcp", "--connect", self.port.to_string()],
                    "env": { (TOKEN_ENV): self.token.secret() }
                }
            }
        });
        format!("{config:#}")
    }

    /// Take at most one request for the editor thread to run. Expired requests are answered
    /// here and never reach the editor. Return an error if the listener has stopped so the
    /// window can clear its enabled state.
    pub(super) fn poll(&self) -> Result<Option<Request>, String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("The local MCP connection stopped".to_owned());
        }
        let request = match self.requests.try_recv() {
            Ok(request) => request,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => return Err("The local MCP connection stopped".to_owned()),
        };
        if !request.expired() {
            return Ok(Some(request));
        }
        request.answer(Err("The live request expired or disconnected before the editor received it".to_owned()));
        Ok(None)
    }
}

/// One deferred read-only MCP check. The existing endpoint bounds additional peers;
/// another check is refused while this slot is occupied, rather than growing a queue.
#[derive(Debug, Default)]
pub(super) struct RemoteCheck {
    queue: crate::board_drc::jobs::Queue,
    request: Option<(Request, bool, u16)>,
}

/// One deferred copper mutation shares the checker worker limit, but retains its own
/// authenticated request lease. Capturing or canceling it never changes authored state.
#[derive(Debug, Default)]
pub(super) struct RemoteCopper {
    queue: crate::board_drc::jobs::Queue<pcbcraft_engine::board::track::preparation::CopperEditJob>,
    request: Option<(Request, bool, u16)>,
}

impl std::fmt::Debug for Request {
    /// Diagnostic output omits request contents and response channels.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRequest").field("generation", &self.generation).finish_non_exhaustive()
    }
}

impl PcbApp {
    /// Recognize the existing read-only command while preserving the live dispatcher's policy.
    pub(super) fn defer_design_check(&mut self, _ctx: &egui::Context, body: &Value) -> bool {
        body["method"] == "engine.execute" && body["params"]["command"] == pcbcraft_engine::CHECK_DESIGN
    }

    /// Validate through the shared command path, then hand only the captured job to the queue.
    pub(super) fn start_remote_check(&mut self, ctx: &egui::Context, request: Request, chat: bool) {
        use pcbcraft_mcp::live::Editor as _;
        if request.expired() {
            request.answer(Err("Design check request expired".into()));
            return;
        }
        if self.remote_check.queue.busy() {
            request.answer(Err("A design check is already running; retry after it completes".into()));
            return;
        }
        if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(pcbcraft_engine::CHECK_DESIGN) {
            request.answer(Err(reason));
            return;
        }
        let params = &request.body()["params"]["params"];
        let job = self.session.apply(pcbcraft_engine::CHECK_DESIGN, params).and_then(|_| self.session.design_check_job(params));
        match job {
            Ok(job) => {
                let port = self.live_control.as_ref().map_or(0, LiveControl::port);
                self.remote_check.queue.submit(crate::board_drc::jobs::Request { id: 0, job });
                self.remote_check.request = Some((request, chat, port));
                ctx.request_repaint();
            }
            Err(error) => request.answer(Err(error.to_string())),
        }
    }

    /// Retire expired/revoked/stale checks without blocking other MCP edits or window input.
    pub(super) fn poll_remote_check(&mut self, ctx: &egui::Context) {
        let valid = self.remote_check.request.as_ref().is_some_and(|(request, chat, port)| {
            !request.expired()
                && if *chat {
                    self.assistant.turn_open() && self.assistant.generation() == Some(request.generation())
                } else {
                    self.live_control.as_ref().is_some_and(|live| live.port() == *port && live.current() == request.generation())
                }
        });
        // The captured source validates its own board; no focused-view dependency is imposed on MCP.
        if !valid || !self.remote_check.queue.authority_current(&self.session) {
            self.remote_check.queue.cancel();
            if let Some((request, _, _)) = self.remote_check.request.take() {
                request.answer(Err("Design check canceled or request expired".into()));
            }
        }
        if let Some(completion) = self.remote_check.queue.poll()
            && let Some((request, _, _)) = self.remote_check.request.take()
        {
            let result = if !completion.source.is_current(&self.session) {
                Err("Project changed during design check; retry against the current revision".into())
            } else {
                completion.report.and_then(|report| {
                    report
                        .map(|report| json!({"revision":completion.source.revision(),"design_check":report}))
                        .ok_or_else(|| "Design check canceled".into())
                })
            };
            request.answer(result);
        }
        if self.remote_check.queue.busy() {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }

    /// Recognize only existing copper commands; all policy and parameter checks remain shared.
    pub(super) fn defer_copper(&self, body: &Value) -> bool {
        body["method"] == "engine.execute"
            && body["params"]["command"].as_str().is_some_and(pcbcraft_engine::board::track::preparation::is_copper_edit)
    }

    /// Capture a read-only preparation job after policy checks, without executing a mutation.
    pub(super) fn start_remote_copper(&mut self, ctx: &egui::Context, request: Request, chat: bool) {
        if request.expired() {
            request.answer(Err("Copper edit request expired or disconnected".into()));
            return;
        }
        if self.remote_copper.queue.busy() {
            request.answer(Err("A copper edit is already preparing; retry after it completes".into()));
            return;
        }
        let command = request.body()["params"]["command"].as_str().unwrap_or_default();
        if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(command) {
            request.answer(Err(reason));
            return;
        }
        match self.session.prepare_copper_edit(command, &request.body()["params"]["params"]) {
            Ok(job) => {
                let port = self.live_control.as_ref().map_or(0, LiveControl::port);
                self.remote_copper.queue.submit(crate::board_drc::jobs::Request { id: 0, job });
                self.remote_copper.request = Some((request, chat, port));
                ctx.request_repaint();
            }
            Err(error) => request.answer(Err(error.to_string())),
        }
    }

    /// Recheck endpoint, turn, modal and snapshot authority immediately before the shared atomic
    /// transaction. Failed, canceled or obsolete work returns an error without partial state.
    pub(super) fn poll_remote_copper(&mut self, ctx: &egui::Context) {
        let valid = self.remote_copper.request.as_ref().is_some_and(|(request, chat, port)| {
            !request.expired()
                && if *chat {
                    self.assistant.turn_open() && self.assistant.generation() == Some(request.generation())
                } else {
                    self.live_control.as_ref().is_some_and(|live| live.port() == *port && live.current() == request.generation())
                }
        });
        if !valid || !self.remote_copper.queue.authority_current(&self.session) {
            self.remote_copper.queue.cancel();
            if let Some((request, _, _)) = self.remote_copper.request.take() {
                request.answer(Err("Copper preparation canceled, disconnected or stale".into()));
            }
        }
        if let Some(completion) = self.remote_copper.queue.poll()
            && let Some((request, chat, _)) = self.remote_copper.request.take()
        {
            let revision = self.session.revision();
            let command = request.body()["params"]["command"].as_str().unwrap_or_default();
            let refusal = (LiveEditor { app: self, ctx }).refusal(command);
            let result = if request.expired() || !completion.source.is_current(&self.session) {
                Err("Copper preparation canceled or project changed".into())
            } else if let Some(reason) = refusal {
                Err(reason)
            } else {
                completion.report.and_then(|edit| {
                    let edit = edit.ok_or("Copper preparation canceled")?;
                    self.session.commit_copper_edit(edit).map_err(|e| e.to_string())?;
                    Ok(if request.body()["params"]["inspect"].as_bool() == Some(false) {
                        json!({"revision":self.session.revision()})
                    } else {
                        self.session.inspect()
                    })
                })
            };
            if self.session.revision() != revision {
                self.views.sync(&self.session);
                let title = pcbcraft_engine::command(command).map_or("Edit", |c| c.title);
                self.message = None;
                self.file_notice = Some(format!("{title} {}", if chat { "by the Assistant" } else { "through MCP" }));
                if chat {
                    self.assistant.note(format!("Edited: {title}. Undo reverts it."));
                }
                self.repaint_editors(ctx);
            }
            request.answer(result);
        }
        if self.remote_copper.queue.busy() {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }

    /// Serve the in-app chat's endpoint on the editor thread while a message is being answered:
    /// run the tool calls the person answered, then gate the next request. Reading and allowed commands run through the same
    /// dispatcher as live MCP, with its refusals; the rest wait in the chat for Allow or Deny.
    /// A project edit is noted in the conversation and the status bar.
    pub(super) fn serve_chat(&mut self, ctx: &egui::Context) {
        // Calls belong to the message being answered. After Stop or a new chat (a project switch
        // starts one), anything still queued, answered or arriving is refused unrun, so nothing
        // the person stopped happens later, on this project or the next.
        let open = self.assistant.turn_open();
        let current = self.assistant.generation();
        let belongs = |request: &Request| open && Some(request.generation()) == current;
        let stopped = |what: &str| format!("No reply is in progress (it was stopped, or a new chat started), so {what} did not run.");
        for (approval, answer) in self.assistant.take_answers() {
            let crate::assistant::Source::Tool { request, command } = approval.source else { continue };
            if answer == crate::assistant::Answer::Deny {
                request.answer(Err(format!("The person denied `{command}`. Ask them before trying again.")));
            } else if !belongs(&request) {
                request.answer(Err(stopped(&format!("`{command}`"))));
            } else {
                self.run_chat_request(ctx, request);
            }
        }
        // Requests from an earlier turn are refused as they come; the first current one is gated.
        let request = loop {
            let Some(request) = self.assistant.take_request() else { return };
            if belongs(&request) {
                break request;
            }
            let what = request.body()["params"]["command"].as_str().map_or_else(|| "the request".to_owned(), |command| format!("`{command}`"));
            request.answer(Err(stopped(&what)));
        };
        let method = request.body()["method"].as_str().unwrap_or_default().to_owned();
        let command = request.body()["params"]["command"].as_str().map(str::to_owned);
        let verdict = crate::assistant::gate(self.assistant.mode(), &method, command.as_deref(), |command| self.assistant.always_allowed(command));
        match (verdict, command) {
            (crate::assistant::Verdict::Ask, Some(command)) if !request.expired() => {
                let params = request.body()["params"]["params"].to_string();
                let detail = if params == "null" { String::new() } else { params.chars().take(300).collect() };
                self.assistant.hold(request, command, detail);
                self.repaint_editors(ctx);
            }
            _ => self.run_chat_request(ctx, request),
        }
    }

    /// Run one chat request through the live dispatcher and answer it. An expired request is
    /// dropped unrun: its caller has already given up.
    fn run_chat_request(&mut self, ctx: &egui::Context, request: Request) {
        if request.expired() {
            return;
        }
        if self.defer_design_check(ctx, request.body()) {
            self.start_remote_check(ctx, request, true);
            return;
        }
        if self.defer_copper(request.body()) {
            self.start_remote_copper(ctx, request, true);
            return;
        }
        let revision = self.session.revision();
        let result = call_editor(&mut LiveEditor { app: self, ctx }, request.body());
        if let Some(owner) = self.assistant.automation_owner(request.generation()) {
            self.automation.authorize(owner);
        }
        if self.session.revision() != revision {
            self.views.sync(&self.session);
            let title = request.body()["params"]["command"].as_str().and_then(pcbcraft_engine::command).map_or("Edit", |command| command.title);
            self.message = None;
            self.file_notice = Some(format!("{title} by the Assistant"));
            self.assistant.note(format!("Edited: {title}. Undo reverts it."));
        }
        request.answer(result);
        self.repaint_editors(ctx);
    }
}

/// The open editor as the live connection sees it, for one request on the editor thread.
pub(super) struct LiveEditor<'a> {
    pub(super) app: &'a mut PcbApp,
    pub(super) ctx: &'a egui::Context,
}

impl Editor for LiveEditor<'_> {
    /// UI actions retain normal dialog and text-field interaction rather than bypassing them
    /// through project commands. The queue validates native focus and endpoint ownership.
    fn ui_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        self.app.automation_command(self.ctx, id, params)
    }
    fn session(&mut self) -> &mut Session {
        &mut self.app.session
    }

    /// Hold commands while an inline edit, dialog, file operation or project close owns the editor, as the
    /// GUI disables its controls. Catalog mode/panel commands can use its own modal controls,
    /// including closing the catalog through the same panel command that opened it.
    /// New Project waits for unsaved changes to be saved or
    /// discarded in the editor, which is where the GUI asks.
    fn refusal(&self, command: &str) -> Option<String> {
        let app = &*self.app;
        if app.automation.busy() {
            return Some("Wait for the queued UI input to complete, or cancel it, before running another command".into());
        }
        if (app.library_mutation_busy() || app.library_sources.installing())
            && (command.starts_with("library.") || command.starts_with("app.library_"))
        {
            return Some("Finish the current library folder operation before changing its sources or editors".into());
        }
        if app.views.editing_text() {
            return Some(format!("Finish the current text edit before running `{command}`"));
        }
        if !matches!(app.dialog, Dialog::None) || app.files.busy() || app.project_action_pending() {
            return Some(format!("Finish the current dialog or file operation in the editor before running `{command}`"));
        }
        if app.parts.is_modal() && !matches!(command, "window.set_catalog_mode" | "window.set_panel") {
            return Some(format!("Close the Parts Catalog or switch it to compact mode before running `{command}`"));
        }
        if command == "project.new" && app.files.dirty(&app.session) {
            return Some("The open project has unsaved changes. Save or discard them in the editor before starting a new project.".to_owned());
        }
        None
    }

    fn window_commands(&self) -> Vec<Value> {
        let mut commands = window_commands::window_catalog();
        commands.extend(crate::ui_automation::catalog());
        commands
    }

    fn inspect(&self) -> Value {
        window_commands::inspect_editor(self.app)
    }

    fn run_window_command(&mut self, id: &str, params: &Value) -> Result<(), String> {
        self.app.run_window_command(self.ctx, id, params)
    }
}

impl Drop for LiveControl {
    /// Wake the worker so a disabled endpoint stops accepting new connections.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
    }
}

/// Accept peers independently of authentication and replies. At capacity, leave new peers
/// in the OS backlog until a worker finishes. Close the listener before waiting for workers.
fn listen(
    listener: Listener,
    token: &Token,
    stop: &AtomicBool,
    generation: &AtomicU64,
    send: &SyncSender<Request>,
    ctx: &egui::Context,
    wait: Duration,
) {
    thread::scope(|scope| {
        let mut workers: Vec<ScopedJoinHandle<'_, ()>> = Vec::with_capacity(MAX_PEERS);
        while !stop.load(Ordering::Acquire) {
            workers.retain(|worker| !worker.is_finished());
            if workers.len() == MAX_PEERS {
                thread::park_timeout(ACCEPT_POLL);
                continue;
            }
            let peer = match listener.accept_peer() {
                Ok(peer) => peer,
                Err(error) => match error.kind() {
                    io::ErrorKind::WouldBlock => {
                        thread::park_timeout(ACCEPT_POLL);
                        continue;
                    }
                    io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset => continue,
                    _ => break,
                },
            };
            match thread::Builder::new().spawn_scoped(scope, move || serve_peer(peer, token, stop, generation, send, ctx, wait)) {
                Ok(worker) => workers.push(worker),
                Err(_) => thread::park_timeout(ACCEPT_POLL),
            }
        }
        stop.store(true, Ordering::Release);
        drop(listener);
        ctx.request_repaint();
    });
}

/// Configure and authenticate one peer before submitting its request to the editor.
/// Failed authentication, I/O or unexpected trailing input closes only this peer. A full editor
/// queue gets a busy reply; the single-request exchange never leaves trailing bytes unread.
fn serve_peer(
    peer: AcceptedPeer,
    token: &Token,
    stop: &AtomicBool,
    generation: &AtomicU64,
    send: &SyncSender<Request>,
    ctx: &egui::Context,
    wait: Duration,
) {
    let Ok(mut peer) = peer.configure(IO_TIMEOUT) else {
        return;
    };
    let Ok(body) = peer.authenticate(token) else {
        return;
    };
    if stop.load(Ordering::Acquire) {
        return;
    }
    let (reply, response) = mpsc::sync_channel(1);
    let deadline = Instant::now() + wait;
    // Stamped once authenticated: a request belongs to the generation it arrived in.
    let generation = generation.load(Ordering::Acquire);
    let connected = Arc::new(AtomicBool::new(true));
    if send.try_send(Request { body, deadline, reply, generation, connected: connected.clone() }).is_err() {
        let _ = peer.send(&json!({"error": "The PCBCraft editor is busy"}));
        return;
    }
    ctx.request_repaint();
    while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        if peer.peer_closed_or_pending_input().unwrap_or(true) {
            break;
        }
        match response.recv_timeout(ACCEPT_POLL.min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(response) => {
                let _ = peer.send(&response);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    connected.store(false, Ordering::Release);
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpStream};

    use craft_control::loopback::Connection;

    use super::*;

    /// A silent peer must not hold up a valid request until the authentication timeout.
    #[test]
    fn accepts_authenticated_requests_while_another_peer_is_silent() -> io::Result<()> {
        let endpoint = LiveControl::start(&egui::Context::default())?;
        let _silent = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port))?;
        let request = json!({"method": "project.inspect", "params": {}});
        let timeout = IO_TIMEOUT / 2;
        let _client = Connection::connect(endpoint.port, &endpoint.token, request.clone(), timeout)?;
        assert_eq!(endpoint.requests.recv_timeout(timeout).unwrap().body, request);
        Ok(())
    }

    /// Observe the worker's disconnect flag before letting the editor poll the queued request.
    /// A bounded channel handoff preserves production poll/expiry behavior without a fixed delay.
    #[test]
    fn a_client_disconnected_before_acceptance_cannot_leave_queued_ui_input() {
        let mut h = crate::tests::harness();
        crate::tests::resistor_at_origin(&mut h);
        let ctx = h.ctx.clone();
        h.state_mut().live_control = Some(LiveControl::start_with_wait(&ctx, Duration::from_secs(10)).unwrap());
        h.state_mut().automation.observing = true;
        h.run();
        let live = h.state_mut().live_control.as_mut().unwrap();
        let client = Connection::connect(
            live.port(),
            &live.token,
            json!({"method":"engine.execute","params":{"command":"ui.key","params":{"window":0,"key":"W"}}}),
            Duration::from_secs(2),
        )
        .unwrap();
        let request = live.requests.recv_timeout(Duration::from_secs(2)).expect("socket worker did not queue the request");
        let connected = request.connected.clone();
        let (send, receive) = mpsc::sync_channel(1);
        send.try_send(request).unwrap();
        live.requests = receive;
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(2);
        while connected.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "socket worker did not publish peer disconnect");
            thread::sleep(Duration::from_millis(1));
        }
        h.run_steps(4);
        assert!(!h.state().automation.busy());
        assert!(h.state().views.active.schematic.tool.is_none());
    }
}

#[cfg(test)]
mod checker_tests {
    use super::*;
    /// Deferred MCP reports preserve the existing response shape and never block a later edit.
    /// Revoked endpoints, deadlines and changed snapshots produce errors rather than stale passes.
    #[test]
    fn deferred_check_allows_edits_and_rejects_stale_expired_and_revoked_requests() {
        let ctx = egui::Context::default();
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"create_board":true})).unwrap();
        app.live_control = Some(LiveControl::start(&ctx).unwrap());
        let request = |app: &PcbApp, expired: bool| {
            let (reply, receive) = mpsc::sync_channel(1);
            let deadline = if expired { Instant::now() - Duration::from_secs(1) } else { Instant::now() + Duration::from_secs(10) };
            (
                Request {
                    body: json!({"method":"engine.execute","params":{"command":pcbcraft_engine::CHECK_DESIGN,"params":{}}}),
                    deadline,
                    reply,
                    generation: app.live_control.as_ref().unwrap().current(),
                    connected: Arc::new(AtomicBool::new(true)),
                },
                receive,
            )
        };
        let (r, reply) = request(&app, false);
        app.start_remote_check(&ctx, r, false);
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        assert_eq!(reply.try_recv().unwrap()["result"]["design_check"]["total"], 0);
        let (r, reply) = request(&app, false);
        app.start_remote_check(&ctx, r, false);
        let rev = app.session.revision();
        app.session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":rev})).unwrap();
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        assert!(reply.try_recv().unwrap()["error"].is_string());
        assert_eq!(app.session.revision(), rev + 1);
        let (r, reply) = request(&app, true);
        app.start_remote_check(&ctx, r, false);
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        assert!(reply.try_recv().unwrap()["error"].is_string());
        let (r, reply) = request(&app, false);
        app.start_remote_check(&ctx, r, false);
        app.live_control = None;
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        assert!(reply.try_recv().unwrap()["error"].is_string());
        assert!(!app.remote_check.queue.busy());
    }
}

#[cfg(test)]
mod copper_tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpStream};

    /// Existing commands provide a board and copper parameters without fixture file surgery.
    fn fixture() -> (PcbApp, egui::Context, Value) {
        let ctx = egui::Context::default();
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"create_board":true})).unwrap();
        app.session.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":app.session.revision()})).unwrap();
        app.live_control = Some(LiveControl::start(&ctx).unwrap());
        let p = app.session.project().unwrap();
        let view = p.board_view(p.board_target().unwrap()).unwrap();
        let layer = view.layers().into_iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper).unwrap().id;
        let body = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::ADD_TRACKS,"inspect":false,"params":{
            "board":view.target.board,"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1_000_000,"y_nm":0}],"expected_revision":app.session.revision()}}});
        (app, ctx, body)
    }
    /// Keep the request's response and connection lease available for failure-path tests.
    fn request(app: &PcbApp, body: Value) -> (Request, Receiver<Value>, Arc<AtomicBool>) {
        let (reply, receive) = mpsc::sync_channel(1);
        let connected = Arc::new(AtomicBool::new(true));
        (
            Request {
                body,
                deadline: Instant::now() + Duration::from_secs(10),
                reply,
                generation: app.live_control.as_ref().unwrap().current(),
                connected: connected.clone(),
            },
            receive,
            connected,
        )
    }
    /// Deferred GUI/MCP commands have identical engine authority and response shape. Full slots,
    /// concurrent edits, expired/disconnected clients and modal ownership cannot publish copper.
    #[test]
    fn remote_preparation_rechecks_lease_policy_and_revision_before_atomic_publication() {
        for mode in 0..7 {
            let (mut app, ctx, body) = fixture();
            let revision = app.session.revision();
            let (r, reply, connected) = request(&app, body.clone());
            app.start_remote_copper(&ctx, r, false);
            assert_eq!(app.session.revision(), revision);
            match mode {
                1 => {
                    app.session.apply("project.rename", &json!({"name":"Concurrent MCP edit"})).unwrap();
                }
                2 => connected.store(false, Ordering::Release),
                3 => app.remote_copper.request.as_mut().unwrap().0.deadline = Instant::now(),
                4 => app.live_control = None,
                5 => app.dialog = Dialog::About,
                6 => {
                    let (r, second, _) = request(&app, body);
                    app.start_remote_copper(&ctx, r, false);
                    assert!(second.try_recv().unwrap()["error"].as_str().unwrap().contains("already"));
                }
                _ => {}
            }
            for _ in 0..100 {
                app.poll_remote_copper(&ctx);
            }
            let result = reply.try_recv().unwrap();
            if mode == 0 || mode == 6 {
                assert_eq!(result["result"], json!({"revision":revision+1}));
                assert_eq!(app.session.project().unwrap().boards().next().unwrap().tracks().len(), 1);
                app.session.apply("edit.undo", &Value::Null).unwrap();
            } else {
                assert!(result["error"].is_string(), "{mode}: {result}");
            }
            assert!(app.session.project().unwrap().boards().next().unwrap().tracks().is_empty());
            assert!(!app.remote_copper.queue.busy());
        }
    }
    /// The actual socket worker revokes a deferred request when it observes a client disconnect,
    /// rather than keeping that mutation alive until the original request timeout. Trailing
    /// input, including an incomplete frame, cannot conceal the shutdown.
    #[test]
    fn socket_disconnect_revokes_copper_preparation() {
        use std::io::Write;
        for trailing in [b"".as_slice(), b"x", b"{}\n"] {
            let (mut app, ctx, mut body) = fixture();
            let endpoint = app.live_control.as_ref().unwrap();
            body["token"] = json!(endpoint.token.secret());
            let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
            let mut frame = serde_json::to_vec(&body).unwrap();
            frame.push(b'\n');
            client.write_all(&frame).unwrap();
            let r = endpoint.requests.recv_timeout(Duration::from_secs(2)).unwrap();
            let connected = r.connected.clone();
            let revision = app.session.revision();
            app.start_remote_copper(&ctx, r, false);
            client.write_all(trailing).unwrap();
            drop(client);
            let deadline = Instant::now() + Duration::from_secs(2);
            while connected.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }
            for _ in 0..100 {
                app.poll_remote_copper(&ctx);
            }
            assert_eq!(app.session.revision(), revision);
            assert!(app.session.project().unwrap().boards().next().unwrap().tracks().is_empty());
            assert!(!app.remote_copper.queue.busy());
        }
    }
}
