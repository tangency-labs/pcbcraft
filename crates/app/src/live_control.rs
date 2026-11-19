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
use pcbcraft_engine::{Session, error_codes::ErrorCode, nets};
use pcbcraft_mcp::{
    Failure,
    live::{Editor, TOKEN_ENV, call_editor, check_deferred_version, normalize_request, success_reply},
};
use serde_json::{Value, json};

use crate::{Dialog, PcbApp, board_drc::jobs, ui_automation::Owner, window_commands};

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

    /// Send the editor's answer. A failure keeps its message in `error` and adds its stable
    /// `code` and `details`. A peer that has already given up is ignored.
    pub(super) fn answer(self, result: Result<Value, Failure>) {
        let _ = self.reply.try_send(match result {
            Ok(result) => success_reply(&self.body, result),
            Err(failure) => failed_reply(&failure),
        });
    }
}

fn failed_reply(failure: &Failure) -> Value {
    json!({"error": failure.message(), "code": failure.code(), "details": failure.details()})
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
        request.answer(Err(Failure::new(ErrorCode::Cancelled, "The live request expired or disconnected before the editor received it")));
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

/// One pair preparation retains the original endpoint identity and request lease until commit.
#[derive(Debug, Default)]
pub(super) struct RemotePair {
    queue: jobs::Queue<nets::PairEditJob>,
    request: Option<(Request, Owner)>,
}

impl std::fmt::Debug for Request {
    /// Diagnostic output omits request contents and response channels.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRequest").field("generation", &self.generation).finish_non_exhaustive()
    }
}

/// Only exhausted worker capacity is a busy refusal; spawn and runtime failures remain unavailable.
pub(crate) fn snapshot_worker_failure(kind: Option<crate::board_drc::jobs::ScheduleFailure>, message: String) -> Failure {
    let code = match kind {
        Some(crate::board_drc::jobs::ScheduleFailure::Capacity) => ErrorCode::Busy,
        _ => ErrorCode::Unavailable,
    };
    Failure::new(code, message)
}

/// Why deferred work was dropped: the project changed under a still-valid request, or the
/// request was canceled, expired or lost its endpoint.
fn obsolete(valid: bool, message: &str, revision: u64) -> Failure {
    if valid {
        Failure::new(ErrorCode::StaleRevision, message).with_details(serde_json::Map::from_iter([("current_revision".into(), revision.into())]))
    } else {
        Failure::new(ErrorCode::Cancelled, message)
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
            request.answer(Err(Failure::new(ErrorCode::Cancelled, "Design check request expired")));
            return;
        }
        if self.remote_check.queue.busy() {
            request.answer(Err(Failure::new(ErrorCode::Busy, "A design check is already running; retry after it completes")));
            return;
        }
        if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(pcbcraft_engine::CHECK_DESIGN) {
            request.answer(Err(Failure::new(ErrorCode::Unavailable, reason)));
            return;
        }
        if let Err(failure) = check_deferred_version(pcbcraft_engine::CHECK_DESIGN, &request.body()["params"]) {
            request.answer(Err(failure));
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
            Err(error) => request.answer(Err(error.into())),
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
                request.answer(Err(obsolete(valid, "Design check canceled or request expired", self.session.revision())));
            }
        }
        if let Some(completion) = self.remote_check.queue.poll()
            && let Some((request, _, _)) = self.remote_check.request.take()
        {
            let result = if !completion.source.is_current(&self.session) {
                Err(Failure::new(ErrorCode::StaleRevision, "Project changed during design check; retry against the current revision")
                    .with_details(serde_json::Map::from_iter([("current_revision".into(), self.session.revision().into())])))
            } else {
                completion.report.map_err(|message| snapshot_worker_failure(completion.schedule_failure, message)).and_then(|report| {
                    report
                        .map(|report| json!({"revision":completion.source.revision(),"design_check":report}))
                        .ok_or_else(|| Failure::new(ErrorCode::Cancelled, "Design check canceled"))
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
            request.answer(Err(Failure::new(ErrorCode::Cancelled, "Copper edit request expired or disconnected")));
            return;
        }
        if self.remote_copper.queue.busy() {
            request.answer(Err(Failure::new(ErrorCode::Busy, "A copper edit is already preparing; retry after it completes")));
            return;
        }
        let command = request.body()["params"]["command"].as_str().unwrap_or_default();
        if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(command) {
            request.answer(Err(Failure::new(ErrorCode::Unavailable, reason)));
            return;
        }
        if let Err(failure) = check_deferred_version(command, &request.body()["params"]) {
            request.answer(Err(failure));
            return;
        }
        match self.session.prepare_copper_edit(command, &request.body()["params"]["params"]) {
            Ok(job) => {
                let port = self.live_control.as_ref().map_or(0, LiveControl::port);
                self.remote_copper.queue.submit(crate::board_drc::jobs::Request { id: 0, job });
                self.remote_copper.request = Some((request, chat, port));
                ctx.request_repaint();
            }
            Err(error) => request.answer(Err(error.into())),
        }
    }

    /// Recheck endpoint, turn, modal and snapshot authority immediately before the shared atomic
    /// transaction. Failed, canceled or obsolete work returns an error without partial state;
    /// engine refusals retain their code across the worker boundary.
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
                request.answer(Err(obsolete(valid, "Copper preparation canceled, disconnected or stale", self.session.revision())));
            }
        }
        if let Some(completion) = self.remote_copper.queue.poll()
            && let Some((request, chat, _)) = self.remote_copper.request.take()
        {
            let revision = self.session.revision();
            let command = request.body()["params"]["command"].as_str().unwrap_or_default();
            let refusal = (LiveEditor { app: self, ctx }).refusal(command);
            const OBSOLETE: &str = "Copper preparation canceled or project changed";
            let result = if request.expired() {
                Err(Failure::new(ErrorCode::Cancelled, OBSOLETE))
            } else if !completion.source.is_current(&self.session) {
                Err(Failure::new(ErrorCode::StaleRevision, OBSOLETE)
                    .with_details(serde_json::Map::from_iter([("current_revision".into(), revision.into())])))
            } else if let Some(reason) = refusal {
                Err(Failure::new(ErrorCode::Unavailable, reason))
            } else {
                completion.report.map_err(|message| snapshot_worker_failure(completion.schedule_failure, message)).and_then(|edit| {
                    let edit = edit.ok_or_else(|| Failure::new(ErrorCode::Cancelled, "Copper preparation canceled"))??;
                    self.session.commit_copper_edit(edit)?;
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

    /// Pair commands, including read-only previews, must not drain the Save encoder inline.
    pub(super) fn defer_pair(&self, body: &Value) -> bool {
        body["method"] == "engine.execute" && body["params"]["command"].as_str().is_some_and(nets::is_pair_edit)
    }

    /// Capture the same stop flag used for ordinary queued UI input, not just the port number.
    fn pair_owner(&self, chat: bool, generation: u64) -> Option<Owner> {
        if chat {
            if !self.assistant.turn_open() || self.assistant.generation() != Some(generation) {
                return None;
            }
            self.assistant.automation_owner(generation)
        } else {
            let live = self.live_control.as_ref()?;
            (live.current() == generation).then(|| live.automation_owner(false, generation))
        }
    }

    /// Reject endpoint replacement, turn rollover and peer expiration even after a job completed.
    fn pair_lease_current(&self, request: &Request, owner: &Owner) -> bool {
        self.pair_owner(owner.chat, request.generation()).is_some_and(|current| Arc::ptr_eq(&current.stopped, &owner.stopped))
            && !owner.stopped.load(Ordering::Acquire)
            && !request.expired()
    }

    /// Run policy and version checks before retaining a snapshot; never execute the edit here.
    pub(super) fn start_remote_pair(&mut self, ctx: &egui::Context, request: Request, chat: bool) {
        let Some(owner) = self.pair_owner(chat, request.generation()).filter(|owner| self.pair_lease_current(&request, owner)) else {
            request.answer(Err(Failure::new(ErrorCode::Cancelled, "Pair request expired or lost its endpoint")));
            return;
        };
        let command = request.body()["params"]["command"].as_str().unwrap_or_default();
        let result = if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(command) {
            Err(Failure::new(ErrorCode::Unavailable, reason))
        } else if let Err(failure) = check_deferred_version(command, &request.body()["params"]) {
            Err(failure)
        } else if self.remote_pair.queue.busy() {
            Err(Failure::new(ErrorCode::Busy, "A pair edit is already preparing; retry after it completes"))
        } else {
            self.session.prepare_pair_edit(command, &request.body()["params"]["params"]).map_err(Failure::from)
        };
        match result {
            Ok(job) => {
                self.remote_pair.queue.submit(jobs::Request { id: 0, job });
                self.remote_pair.request = Some((request, owner));
                ctx.request_repaint();
            }
            Err(failure) => request.answer(Err(failure)),
        }
    }

    /// Retire obsolete work before polling and recheck the live lease at the publication boundary.
    pub(super) fn poll_remote_pair(&mut self, ctx: &egui::Context) {
        const OBSOLETE: &str = "Pair preparation canceled, disconnected or project changed";
        let valid = self.remote_pair.request.as_ref().is_some_and(|(request, owner)| self.pair_lease_current(request, owner));
        if !valid || !self.remote_pair.queue.source().is_some_and(|source| source.is_current(&self.session)) {
            self.remote_pair.queue.cancel();
            if let Some((request, _)) = self.remote_pair.request.take() {
                request.answer(Err(obsolete(valid, OBSOLETE, self.session.revision())));
            }
        }
        if let Some(completion) = self.remote_pair.queue.poll()
            && let Some((request, owner)) = self.remote_pair.request.take()
        {
            let revision = self.session.revision();
            let command = request.body()["params"]["command"].as_str().unwrap_or_default();
            let result = if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(command) {
                Err(Failure::new(ErrorCode::Unavailable, reason))
            } else if !completion.source.is_current(&self.session) {
                Err(obsolete(self.pair_lease_current(&request, &owner), OBSOLETE, revision))
            } else {
                completion.report.map_err(|message| snapshot_worker_failure(completion.schedule_failure, message)).and_then(|edit| {
                    let edit = edit.ok_or_else(|| Failure::new(ErrorCode::Cancelled, OBSOLETE))??;
                    let inspect = request.body()["params"]["inspect"].as_bool() != Some(false);
                    if !self.pair_lease_current(&request, &owner) {
                        return Err(Failure::new(ErrorCode::Cancelled, OBSOLETE));
                    }
                    self.session.complete_pair_edit(edit, inspect).map_err(Failure::from)
                })
            };
            if revision != self.session.revision() {
                self.views.sync(&self.session);
                let title = pcbcraft_engine::command(command).map_or("Edit", |c| c.title);
                self.message = None;
                self.file_notice = Some(format!("{title} {}", if owner.chat { "by the Assistant" } else { "through MCP" }));
                if owner.chat {
                    self.assistant.note(format!("Edited: {title}. Undo reverts it."));
                }
                self.repaint_editors(ctx);
            }
            request.answer(result);
        }
        if self.remote_pair.queue.busy() {
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
                request.answer(Err(Failure::new(ErrorCode::Denied, format!("The person denied `{command}`. Ask them before trying again."))));
            } else if !belongs(&request) {
                request.answer(Err(Failure::new(ErrorCode::Cancelled, stopped(&format!("`{command}`")))));
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
            request.answer(Err(Failure::new(ErrorCode::Cancelled, stopped(&what))));
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
        if self.defer_pair(request.body()) {
            self.start_remote_pair(ctx, request, true);
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
    /// UI handler refusals read as unavailable, apart from parameters the catalog refuses.
    fn ui_command(&mut self, id: &str, params: &Value) -> Result<Value, Failure> {
        crate::ui_automation::check_command(id, params)?;
        self.app.automation_command(self.ctx, id, params).map_err(Failure::from)
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
        if (app.library_sources.mutating() || app.library_sources.installing())
            && (command.starts_with("library.") || command.starts_with("app.library_") || command.starts_with("app.part_"))
        {
            return Some("Finish the current library folder operation before changing its sources or editors".into());
        }
        if app.library_editors.publisher.busy()
            && matches!(command, pcbcraft_engine::library::sources::LOAD | pcbcraft_engine::library::sources::REMOVE | "app.library_source")
        {
            return Some("Finish the current publication before changing library sources".into());
        }
        if app.views.editing_text() {
            return Some(format!("Finish the current text edit before running `{command}`"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        let item_delete_review = command == "app.library_delete_part" && matches!(app.dialog, Dialog::DeleteLibraryItem(_));
        #[cfg(target_arch = "wasm32")]
        let item_delete_review = false;
        let pin_remap_review = command == "app.library_pin_remap" && matches!(app.dialog, Dialog::LibraryPinRemap(_));
        // file.abandon is how a client releases the editor from a stalled file job.
        if command != "file.abandon"
            && ((!matches!(app.dialog, Dialog::None) && !item_delete_review && !pin_remap_review) || app.files.busy() || app.project_action_pending())
        {
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

    /// An unknown id or parameters the window catalog refuses fail with their own codes;
    /// a handler's refusal reads as unavailable.
    fn run_window_command(&mut self, id: &str, params: &Value) -> Result<(), Failure> {
        window_commands::check_command(id, params)?;
        self.app.run_window_command(self.ctx, id, params).map_err(Failure::from)
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
    let Ok(mut body) = peer.authenticate(token) else {
        return;
    };
    if stop.load(Ordering::Acquire) {
        return;
    }
    if let Err(failure) = normalize_request(&mut body) {
        let _ = peer.send(&failed_reply(&failure));
        return;
    }
    let (reply, response) = mpsc::sync_channel(1);
    let deadline = Instant::now() + wait;
    // Stamped once authenticated: a request belongs to the generation it arrived in.
    let generation = generation.load(Ordering::Acquire);
    let connected = Arc::new(AtomicBool::new(true));
    if send.try_send(Request { body, deadline, reply, generation, connected: connected.clone() }).is_err() {
        let _ = peer.send(&json!({"error": "The PCBCraft editor is busy", "code": ErrorCode::Busy.as_str(), "details": {}}));
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

    /// A valid wire pin enters the ordinary policy path; a missing one never reaches the queue.
    #[test]
    fn guarded_execution_is_normalized_before_editor_dispatch() -> io::Result<()> {
        let endpoint = LiveControl::start(&egui::Context::default())?;
        let timeout = IO_TIMEOUT / 2;
        let body = json!({"method":"engine.execute_versioned", "params":{"command":"project.new", "version":1}});
        let _client = Connection::connect(endpoint.port, &endpoint.token, body.clone(), timeout)?;
        let request = endpoint.requests.recv_timeout(timeout).unwrap();
        assert_eq!(request.body()["method"], "engine.execute");
        assert_eq!(request.body()["params"], body["params"]);
        let missing = json!({"method":"engine.execute_versioned", "params":{"command":"project.new"}});
        let mut client = Connection::connect(endpoint.port, &endpoint.token, missing, timeout)?;
        let response = client.receive(4096)?.unwrap();
        assert_eq!(response["code"], "invalid_request");
        assert_eq!(endpoint.requests.try_recv().unwrap_err(), mpsc::TryRecvError::Empty);
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
impl PcbApp {
    /// Deliver a synthetic, still-connected peer through the production native dispatcher.
    pub(super) fn begin_test_live_request(&mut self, ctx: &egui::Context, body: Value) -> (Receiver<Value>, Arc<AtomicBool>) {
        let live = self.live_control.get_or_insert_with(|| LiveControl::start(ctx).unwrap());
        let (reply, replies) = mpsc::sync_channel(1);
        let connected = Arc::new(AtomicBool::new(true));
        let request =
            Request { body, deadline: Instant::now() + Duration::from_secs(10), reply, generation: live.current(), connected: connected.clone() };
        self.run_live_request(ctx, request);
        (replies, connected)
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
        let changed = reply.try_recv().unwrap();
        assert_eq!(changed["error"], "Design check canceled or request expired");
        assert_eq!(changed["code"], "stale_revision", "{changed}");
        assert_eq!(changed["details"], json!({"current_revision": rev + 1}));
        assert_eq!(app.session.revision(), rev + 1);
        let (r, reply) = request(&app, true);
        app.start_remote_check(&ctx, r, false);
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        let expired = reply.try_recv().unwrap();
        assert_eq!(expired["error"], "Design check request expired");
        assert_eq!(expired["code"], "cancelled", "{expired}");
        let (r, reply) = request(&app, false);
        app.start_remote_check(&ctx, r, false);
        app.live_control = None;
        for _ in 0..100 {
            app.poll_remote_check(&ctx);
        }
        let revoked = reply.try_recv().unwrap();
        assert_eq!(revoked["code"], "cancelled");
        assert_eq!(revoked["error"], "Design check canceled or request expired");
        assert_eq!(app.session.revision(), rev + 1);
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
    /// Both deferred command paths refuse a mismatched pin without queuing work or editing.
    #[test]
    fn version_pins_reach_deferred_checks_and_copper_preparation() {
        use pcbcraft_engine::command_api::COMMAND_API_VERSION;
        let (mut app, ctx, mut copper) = fixture();
        let version = COMMAND_API_VERSION + 1;
        let revision = app.session.revision();
        copper["params"]["version"] = json!(version);
        let check = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::CHECK_DESIGN,"version":version}});
        let (r, check_reply, _) = request(&app, check);
        app.start_remote_check(&ctx, r, false);
        let (r, copper_reply, _) = request(&app, copper);
        app.start_remote_copper(&ctx, r, false);
        for reply in [check_reply, copper_reply] {
            let refusal = reply.try_recv().unwrap();
            assert_eq!(refusal["code"], "version_mismatch");
            assert_eq!(refusal["details"]["current_version"], COMMAND_API_VERSION);
            assert!(refusal.get("warnings").is_none());
        }
        assert!(!app.remote_check.queue.busy());
        assert!(!app.remote_copper.queue.busy());
        assert_eq!(app.session.revision(), revision);
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
                    let busy = second.try_recv().unwrap();
                    assert!(busy["error"].as_str().unwrap().contains("already"));
                    assert_eq!(busy["code"], "busy");
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
                let reason = if mode == 5 {
                    format!("Finish the current dialog or file operation in the editor before running `{}`", pcbcraft_engine::ADD_TRACKS)
                } else {
                    "Copper preparation canceled, disconnected or stale".to_owned()
                };
                assert_eq!(result["error"], reason, "{mode}: {result}");
                let code = match mode {
                    1 => "stale_revision",
                    5 => "unavailable",
                    _ => "cancelled",
                };
                assert_eq!(result["code"], code, "{mode}: {result}");
            }
            assert!(app.session.project().unwrap().boards().next().unwrap().tracks().is_empty());
            assert!(!app.remote_copper.queue.busy());
        }
    }
    /// Deferred admission shares headless validation, after live lease/queue/host checks.
    #[test]
    fn malformed_deferred_copper_matches_headless_before_queuing() {
        use pcbcraft_mcp::{Backend, Headless};
        let (mut app, ctx, _) = fixture();
        let snapshot = app.session.snapshot().unwrap();
        app.session = Session::default();
        app.session.open_snapshot(snapshot.clone());
        let mut headless = Headless::with_project(snapshot);
        let before = app.session.inspect();
        let revision = app.session.revision();
        for (expected, code, reason) in
            [(revision, "invalid_params", "has no parameter `no_such_parameter`"), (revision + 1, "stale_revision", "project changed")]
        {
            let body = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::RESHAPE_TRACKS,"params":{
                "board":uuid::Uuid::from_u128(1),"tracks":[],"expected_revision":expected,"no_such_parameter":true}}});
            assert!(app.defer_copper(&body));
            let (request, reply, _) = request(&app, body.clone());
            app.start_remote_copper(&ctx, request, false);
            let live = reply.try_recv().unwrap();
            let failure = headless.call("engine.execute", body["params"].clone()).unwrap_err();
            assert_eq!(failure.code(), code);
            assert!(failure.message().contains(reason), "{failure}");
            assert_eq!(live["code"], failure.code());
            assert_eq!(live["error"], failure.message());
            assert_eq!(live["details"], *failure.details());
            assert!(!app.remote_copper.queue.busy());
            assert!(app.remote_copper.request.is_none());
            assert_eq!(app.session.inspect(), before);
            assert_eq!(headless.call("project.inspect", Value::Null).unwrap(), before);
        }
    }

    /// A missing track fails during queued preparation, with the same code and reason as the
    /// headless command on the same snapshot, revision and input. Neither route changes history.
    #[test]
    fn deferred_copper_engine_failure_matches_headless_without_changing_history() {
        use pcbcraft_mcp::{Backend, Headless};
        let (mut app, ctx, _) = fixture();
        let snapshot = app.session.snapshot().unwrap();
        app.session = pcbcraft_engine::Session::default();
        app.session.open_snapshot(snapshot.clone());
        let mut headless = Headless::with_project(snapshot);
        for name in ["First copper edit", "Copper error fixture"] {
            let params = json!({"name":name});
            app.session.apply("project.rename", &params).unwrap();
            headless.call("engine.execute", json!({"command":"project.rename","params":params})).unwrap();
        }
        app.session.apply("edit.undo", &Value::Null).unwrap();
        headless.call("engine.execute", json!({"command":"edit.undo","params":null})).unwrap();
        let before = app.session.inspect();
        assert_eq!(headless.call("project.inspect", Value::Null).unwrap(), before);
        assert_eq!(before["can_undo"], true);
        assert_eq!(before["can_redo"], true);
        let snapshot = app.session.snapshot().unwrap();
        let board = app.session.project().unwrap().board_target().unwrap().board;
        let body = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::RESHAPE_TRACKS,"inspect":false,"params":{
            "board":board,"tracks":[{"track":uuid::Uuid::from_u128(1),"start":{"x_nm":0,"y_nm":0},"end":{"x_nm":1_000_000,"y_nm":0}}],
            "expected_revision":app.session.revision()}}});
        assert!(app.defer_copper(&body));
        let (r, reply, _) = request(&app, body.clone());
        app.start_remote_copper(&ctx, r, false);
        assert!(app.remote_copper.queue.busy());
        assert!(app.remote_copper.request.is_some());
        assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
        for _ in 0..100 {
            app.poll_remote_copper(&ctx);
        }
        let result = reply.try_recv().unwrap();
        let failure = headless.call("engine.execute", body["params"].clone()).unwrap_err();
        assert_eq!(failure.code(), "not_found");
        assert_eq!(failure.message(), "unknown track");
        assert_eq!(failure.details(), &json!({}));
        assert_eq!(result["code"], failure.code(), "{result}");
        assert_eq!(result["error"], failure.message());
        assert_eq!(result["details"], *failure.details());
        assert_eq!(app.session.inspect(), before);
        assert_eq!(headless.call("project.inspect", Value::Null).unwrap(), before);
        assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &snapshot));
        assert!(!app.remote_copper.queue.busy());
        assert!(app.remote_copper.request.is_none());
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

#[cfg(all(test, unix))]
mod item_delete_tests {
    use super::*;
    /// Only the reviewed part-deletion command passes its own modal guard; other commands and
    /// unrelated dialogs retain ordinary live-control refusal.
    #[test]
    fn deletion_confirmation_is_reachable_without_bypassing_other_dialogs() {
        let (_temp, mut app, key) = crate::native_sources::items::tests::fixture();
        let ctx = egui::Context::default();
        app.begin_library_item_delete(&ctx, "deletable-item", &key).unwrap();
        crate::native_sources::items::tests::finish(&mut app, &ctx);
        let editor = LiveEditor { app: &mut app, ctx: &ctx };
        assert!(editor.refusal("app.library_delete_part").is_none());
        assert!(editor.refusal("app.part_edit").is_some());
        editor.app.dialog = Dialog::About;
        assert!(editor.refusal("app.library_delete_part").is_some());
    }
}
