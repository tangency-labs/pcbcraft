//! Desktop opt-in for live MCP. The socket worker hands requests to the editor thread.

use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, ScopedJoinHandle, Thread},
    time::{Duration, Instant},
};

use craft_control::loopback::{AcceptedPeer, Listener, Token};
use pcbcraft_engine::{Session, error_codes::ErrorCode};
use pcbcraft_mcp::{
    Failure,
    live::{Editor, TOKEN_ENV, call_editor_with_package_commit, check_deferred_version, normalize_request, success_reply},
};
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RequestDecision {
    #[default]
    Pending,
    Cancelled,
    Committing,
}

/// Serialize server cancellation and publication authorization. Never hold this lock while
/// preparing an edit, publishing it, waiting for a response or talking to the peer.
#[derive(Default)]
struct RequestAuthority(Mutex<RequestDecision>);

impl RequestAuthority {
    /// A disconnect or listener stop can cancel only before the editor has acquired the grant.
    fn cancel(&self) {
        let mut decision = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *decision == RequestDecision::Pending {
            *decision = RequestDecision::Cancelled;
        }
    }

    fn cancelled(&self) -> bool {
        *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) == RequestDecision::Cancelled
    }

    /// Sample the deadline under the same lock used by the socket wait, including when its
    /// next poll has not yet run. Only the first decision can authorize publication.
    fn grant_commit(&self, deadline: Instant) -> bool {
        let mut decision = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *decision != RequestDecision::Pending {
            return false;
        }
        if Instant::now() >= deadline {
            *decision = RequestDecision::Cancelled;
            return false;
        }
        *decision = RequestDecision::Committing;
        true
    }

    /// Pending requests retain the original deadline. Once granted, wait for the editor's
    /// response or actual transport loss instead of abandoning at that deadline.
    fn next_wait(&self, deadline: Instant) -> Option<Duration> {
        let mut decision = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match *decision {
            RequestDecision::Cancelled => None,
            RequestDecision::Committing => Some(ACCEPT_POLL),
            RequestDecision::Pending => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    *decision = RequestDecision::Cancelled;
                    None
                } else {
                    Some(remaining.min(ACCEPT_POLL))
                }
            }
        }
    }
}

/// Keep the response channel and deadline with the request until the editor answers it.
pub(super) struct Request {
    body: Value,
    deadline: Instant,
    reply: SyncSender<Value>,
    /// The endpoint's generation when the request authenticated.
    generation: u64,
    authority: Arc<RequestAuthority>,
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
        Instant::now() >= self.deadline || self.authority.cancelled()
    }

    /// Dispatch on the editor thread and acquire the socket's grant only for an effectful
    /// package completion, after validation and preparation have finished.
    pub(super) fn dispatch(&self, app: &mut PcbApp, ctx: &egui::Context) -> Result<Value, Failure> {
        call_editor_with_package_commit(&mut LiveEditor { app, ctx }, self.body(), || {
            if self.authority.grant_commit(self.deadline) {
                Ok(())
            } else {
                Err(Failure::new(ErrorCode::Cancelled, "Package completion request expired or disconnected before publication"))
            }
        })
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

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    /// Cancel any still-running background preparation on teardown without blocking the editor.
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Background preparation result: `None` when the worker observed cancellation.
type PackageCompletionOutcome = Result<Option<pcbcraft_engine::package_completion::PreparedPackageCompletion>, pcbcraft_engine::CommandError>;

struct PendingPackageCompletion {
    source: pcbcraft_engine::package_completion::PackageCompletionSource,
    _cancel: CancelOnDrop,
    result: Receiver<PackageCompletionOutcome>,
    request: Request,
    chat: bool,
    port: u16,
}

/// One deferred package-completion preparation running off the editor thread so large schematics
/// do not stall the UI frame while planning or reconciling nets.
#[derive(Default)]
pub(super) struct RemotePackageCompletion {
    pending: Option<PendingPackageCompletion>,
    #[cfg(test)]
    reply_limit: Option<usize>,
}

impl std::fmt::Debug for RemotePackageCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemotePackageCompletion").field("busy", &self.pending.is_some()).finish()
    }
}

impl RemotePackageCompletion {
    fn busy(&self) -> bool {
        self.pending.is_some()
    }

    fn reply_limit(&self) -> usize {
        #[cfg(test)]
        if let Some(limit) = self.reply_limit {
            return limit;
        }
        pcbcraft_mcp::live::MAX_REPLY_BYTES
    }
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

    /// Use the engine's prepared path where available. Parameter and policy checks remain shared.
    pub(super) fn defer_copper(&self, body: &Value) -> bool {
        let call = &body["params"];
        body["method"] == "engine.execute"
            && call["command"]
                .as_str()
                .is_some_and(|command| pcbcraft_engine::board::track::preparation::can_prepare_copper_edit(command, &call["params"]))
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
                    let (revision, report) = self.session.commit_copper_edit_with_report(edit)?;
                    Ok(if let Some(report) = report {
                        report
                    } else if request.body()["params"]["inspect"].as_bool() == Some(false) {
                        json!({"revision":revision})
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

    /// Recognize package completion so planning and net reconciliation run off the editor thread.
    pub(super) fn defer_package_completion(&self, body: &Value) -> bool {
        body["method"] == "engine.execute" && body["params"]["command"] == pcbcraft_engine::COMPLETE_PACKAGES
    }

    /// Validate policy, version, scope and revision on the editor thread, then move planning and
    /// net reconciliation onto a background worker without mutating session state.
    pub(super) fn start_remote_package_completion(&mut self, ctx: &egui::Context, request: Request, chat: bool) {
        if request.expired() {
            request.answer(Err(Failure::new(ErrorCode::Cancelled, "Package completion request expired or disconnected")));
            return;
        }
        if self.remote_package_completion.busy() {
            request.answer(Err(Failure::new(ErrorCode::Busy, "Package completion is already preparing; retry after it completes")));
            return;
        }
        if let Some(reason) = (LiveEditor { app: self, ctx }).refusal(pcbcraft_engine::COMPLETE_PACKAGES) {
            request.answer(Err(Failure::new(ErrorCode::Unavailable, reason)));
            return;
        }
        if let Err(failure) = check_deferred_version(pcbcraft_engine::COMPLETE_PACKAGES, &request.body()["params"]) {
            request.answer(Err(failure));
            return;
        }
        match self.session.prepare_package_completion(&request.body()["params"]["params"]) {
            Ok(job) => {
                let source = job.source().clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = Arc::clone(&cancel);
                let worker_ctx = ctx.clone();
                let (tx, result) = mpsc::sync_channel(1);
                match thread::Builder::new().name("pcbcraft-pkg-complete".to_owned()).spawn(move || {
                    let outcome = job.run_cancellable(&worker_cancel);
                    let _ = tx.try_send(outcome);
                    worker_ctx.request_repaint();
                }) {
                    Ok(_) => {
                        let port = self.live_control.as_ref().map_or(0, LiveControl::port);
                        self.remote_package_completion.pending =
                            Some(PendingPackageCompletion { source, _cancel: CancelOnDrop(cancel), result, request, chat, port });
                        ctx.request_repaint();
                    }
                    Err(error) => {
                        request.answer(Err(Failure::new(ErrorCode::Unavailable, format!("Could not start package completion worker: {error}"))));
                    }
                }
            }
            Err(error) => request.answer(Err(error.into())),
        }
    }

    /// Recheck endpoint, turn, modal, snapshot currency, reply size and commit grant immediately
    /// before atomic publication. Refused, oversized, stale or canceled runs never alter history.
    pub(super) fn poll_remote_package_completion(&mut self, ctx: &egui::Context) {
        let Some((valid, current)) = self.remote_package_completion.pending.as_ref().map(|pending| {
            let valid = !pending.request.expired()
                && if pending.chat {
                    self.assistant.turn_open() && self.assistant.generation() == Some(pending.request.generation())
                } else {
                    self.live_control.as_ref().is_some_and(|live| live.port() == pending.port && live.current() == pending.request.generation())
                };
            (valid, pending.source.is_current(&self.session))
        }) else {
            return;
        };
        if !valid || !current {
            if let Some(pending) = self.remote_package_completion.pending.take() {
                let PendingPackageCompletion { request, .. } = pending;
                request.answer(Err(obsolete(valid, "Package completion canceled, disconnected or stale", self.session.revision())));
            }
            return;
        }
        let outcome = match self.remote_package_completion.pending.as_ref().map(|pending| pending.result.try_recv()) {
            Some(Ok(outcome)) => Ok(outcome),
            Some(Err(mpsc::TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(8));
                return;
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Err(Failure::new(ErrorCode::Unavailable, "Package completion worker stopped before completing"))
            }
            None => return,
        };
        let Some(PendingPackageCompletion { source, request, chat, .. }) = self.remote_package_completion.pending.take() else {
            return;
        };
        let revision = self.session.revision();
        let refusal = (LiveEditor { app: self, ctx }).refusal(pcbcraft_engine::COMPLETE_PACKAGES);
        let reply_limit = self.remote_package_completion.reply_limit();
        let result = if request.expired() {
            Err(Failure::new(ErrorCode::Cancelled, "Package completion canceled, disconnected or stale"))
        } else if !source.is_current(&self.session) {
            Err(Failure::new(ErrorCode::StaleRevision, "Package completion canceled, disconnected or stale")
                .with_details(serde_json::Map::from_iter([("current_revision".into(), revision.into())])))
        } else if let Some(reason) = refusal {
            Err(Failure::new(ErrorCode::Unavailable, reason))
        } else {
            outcome.and_then(|worker_result| {
                let prepared =
                    worker_result.map_err(Failure::from)?.ok_or_else(|| Failure::new(ErrorCode::Cancelled, "Package completion canceled"))?;
                pcbcraft_mcp::live::check_package_completion_reply_size_limit(
                    request.body(),
                    prepared.report(),
                    prepared.is_effectful(),
                    reply_limit,
                )?;
                if prepared.is_effectful() && !request.authority.grant_commit(request.deadline) {
                    return Err(Failure::new(ErrorCode::Cancelled, "Package completion request expired or disconnected before publication"));
                }
                self.session.commit_package_completion(prepared).map_err(Failure::from)
            })
        };
        if self.session.revision() != revision {
            self.views.sync(&self.session);
            let title = pcbcraft_engine::command(pcbcraft_engine::COMPLETE_PACKAGES).map_or("Edit", |c| c.title);
            self.message = None;
            self.file_notice = Some(format!("{title} {}", if chat { "by the Assistant" } else { "through MCP" }));
            if chat {
                self.assistant.note(format!("Edited: {title}. Undo reverts it."));
            }
            self.repaint_editors(ctx);
        }
        request.answer(result);
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
        if self.defer_package_completion(request.body()) {
            self.start_remote_package_completion(ctx, request, true);
            return;
        }
        let revision = self.session.revision();
        let result = request.dispatch(self, ctx);
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
    /// Removal revokes every addressed board-view worker through the shared command path.
    fn zone_unfill(&mut self, params: &Value) -> Option<Result<Value, pcbcraft_engine::CommandError>> {
        let result = self.app.queue_zone_unfill(params);
        self.ctx.request_repaint();
        Some(result)
    }
    /// Refill returns source-qualified pending state while the existing bounded worker runs.
    fn zone_refill(&mut self, params: &Value) -> Option<Result<Value, pcbcraft_engine::CommandError>> {
        let result = self.app.queue_zone_refill(params);
        self.ctx.request_repaint();
        Some(result)
    }
    /// UI actions retain normal dialog and text-field interaction rather than bypassing them
    /// through project commands. The queue validates native focus and endpoint ownership.
    /// UI handler refusals read as unavailable, apart from parameters the catalog refuses.
    fn ui_command(&mut self, id: &str, params: &Value) -> Result<Value, Failure> {
        crate::ui_automation::check_command(id, params)?;
        self.app.automation_command(self.ctx, id, params).map_err(Failure::from)
    }
    /// Finish a local color Undo or Redo inside the request that moved the history cursor.
    fn restore_local_history(&mut self) {
        self.app.restore_local_history();
        self.app.repaint_editors(self.ctx);
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
    let authority = Arc::new(RequestAuthority::default());
    if send.try_send(Request { body, deadline, reply, generation, authority: authority.clone() }).is_err() {
        let _ = peer.send(&json!({"error": "The PCBCraft editor is busy", "code": ErrorCode::Busy.as_str(), "details": {}}));
        return;
    }
    ctx.request_repaint();
    while !stop.load(Ordering::Acquire) {
        if peer.peer_closed_or_pending_input().unwrap_or(true) {
            break;
        }
        let Some(wait) = authority.next_wait(deadline) else {
            break;
        };
        match response.recv_timeout(wait) {
            Ok(response) => {
                let _ = peer.send(&response);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    authority.cancel();
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpStream};

    use craft_control::loopback::Connection;

    use super::*;

    /// A server timeout cannot abandon a granted edit, and either ordering has one winner.
    #[test]
    fn package_commit_and_socket_timeout_are_one_decision() {
        let expired = Instant::now();
        let live = expired + Duration::from_secs(86_400);
        let socket_won = RequestAuthority::default();
        assert_eq!(socket_won.next_wait(expired), None);
        assert!(!socket_won.grant_commit(live));

        let editor_saw_deadline = RequestAuthority::default();
        assert!(!editor_saw_deadline.grant_commit(expired));
        assert_eq!(editor_saw_deadline.next_wait(live), None);

        let editor_won = RequestAuthority::default();
        assert!(editor_won.grant_commit(live));
        assert_eq!(editor_won.next_wait(expired), Some(ACCEPT_POLL));
        assert!(!editor_won.grant_commit(live));
    }

    /// A dequeued request reaches validation and preparation, but a lost grant cannot edit or
    /// add an undo step. A live grant on the same request publishes exactly one undoable edit.
    #[test]
    fn package_completion_dispatch_requires_the_live_commit_grant() {
        use pcbcraft_engine::{COMPLETE_PACKAGES, PLACE_SYMBOL, library::sources};

        let mut h = crate::tests::harness();
        let ctx = h.ctx.clone();
        let app = h.state_mut();
        app.session.apply("project.new", &json!({})).unwrap();
        app.session
            .execute(
                sources::LOAD,
                &json!({"source":"completion-test","documents":{
                    "library.json":include_str!("../../../examples/native-parts/library.json"),
                    "symbols/dual.json":include_str!("../../../examples/native-parts/symbols/dual-amplifier.json"),
                }}),
            )
            .unwrap();
        let definition = app.session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
        let digest = sources::symbol_digest(&definition).unwrap();
        for (unit, x) in [("a", 0), ("b", 25_400_000)] {
            app.session
                .apply(
                    PLACE_SYMBOL,
                    &json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,
                            "definition_digest":digest,"unit":unit,"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}),
                )
                .unwrap();
        }
        let before = app.session.snapshot().unwrap();
        let inspect = app.session.inspect();
        let revision = app.session.revision();
        let now = Instant::now();
        for (deadline, cancelled) in [(now, false), (now + Duration::from_secs(86_400), true), (now + Duration::from_secs(86_400), false)] {
            let (reply, response) = mpsc::sync_channel(1);
            let authority = Arc::new(RequestAuthority::default());
            if cancelled {
                authority.cancel();
            }
            let request = Request {
                body: json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"inspect":false,
                    "params":{"scope":"project","expected_revision":revision}}}),
                deadline,
                reply,
                generation: 0,
                authority,
            };
            let result = request.dispatch(app, &ctx);
            request.answer(result);
            let answer = response.try_recv().unwrap();
            if deadline == now || cancelled {
                assert_eq!(answer["code"], "cancelled", "{answer}");
                assert_eq!(app.session.inspect(), inspect);
                assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
            } else {
                assert_eq!(answer["result"]["changes"].as_array().unwrap().len(), 1);
                assert_eq!(answer["result"]["revision"], revision + 1);
                app.session.apply("edit.undo", &Value::Null).unwrap();
                assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
            }
        }
    }

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
        let authority = request.authority.clone();
        let (send, receive) = mpsc::sync_channel(1);
        send.try_send(request).unwrap();
        live.requests = receive;
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !authority.cancelled() {
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
                    authority: Arc::new(RequestAuthority::default()),
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
    use pcbcraft_engine::board::setup::{LayerPurpose, Side};
    use std::net::{Ipv4Addr, TcpStream};

    /// Existing commands provide a board and front copper for top-side pads. The layer
    /// catalog is sorted by ID, so its first copper layer can be the back.
    fn fixture() -> (PcbApp, egui::Context, Value) {
        let ctx = egui::Context::default();
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({"create_board":true})).unwrap();
        app.session.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":app.session.revision()})).unwrap();
        app.live_control = Some(LiveControl::start(&ctx).unwrap());
        let p = app.session.project().unwrap();
        let view = p.board_view(p.board_target().unwrap()).unwrap();
        let layer = view.layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)).unwrap().id;
        let body = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::ADD_TRACKS,"inspect":false,"params":{
            "board":view.target.board,"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1_000_000,"y_nm":0}],"expected_revision":app.session.revision()}}});
        (app, ctx, body)
    }
    /// Keep the request's response and cancellation authority available for failure-path tests.
    fn request(app: &PcbApp, body: Value) -> (Request, Receiver<Value>, Arc<RequestAuthority>) {
        let (reply, receive) = mpsc::sync_channel(1);
        let authority = Arc::new(RequestAuthority::default());
        (
            Request {
                body,
                deadline: Instant::now() + Duration::from_secs(10),
                reply,
                generation: app.live_control.as_ref().unwrap().current(),
                authority: authority.clone(),
            },
            receive,
            authority,
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
            let (r, reply, authority) = request(&app, body.clone());
            app.start_remote_copper(&ctx, r, false);
            assert_eq!(app.session.revision(), revision);
            match mode {
                1 => {
                    app.session.apply("project.rename", &json!({"name":"Concurrent MCP edit"})).unwrap();
                }
                2 => authority.cancel(),
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
    /// A valid unroute is deferred and cannot publish after its lease expires. A fresh
    /// request returns the same report as headless preview and publishes exactly once.
    #[test]
    fn deferred_unroute_honors_expiry_and_returns_report() {
        use pcbcraft_engine as engine;
        let (mut app, ctx, body) = fixture();
        let board = body["params"]["params"]["board"].clone();
        let layer = body["params"]["params"]["layer"].clone();
        let apply = |session: &mut Session, command, mut params: Value| {
            params["expected_revision"] = json!(session.revision());
            session.apply(command, &params).unwrap();
        };
        for x_nm in [0, 10_000_000] {
            apply(&mut app.session, engine::PLACE_RESISTOR, json!({"x_nm":x_nm,"y_nm":0}));
            let symbol = app.session.project().unwrap().symbols().last().unwrap().id();
            apply(
                &mut app.session,
                engine::PLACE_FOOTPRINT,
                json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":x_nm,"y_nm":0}),
            );
        }
        let symbols = app.session.project().unwrap().symbols();
        let connection = json!({"from":{"symbol":symbols[0].id(),"terminal":"1"},"to":{"symbol":symbols[1].id(),"terminal":"1"}});
        apply(&mut app.session, engine::CONNECT_PINS, connection);
        apply(&mut app.session, engine::INITIALIZE_UPDATE_BASELINE, json!({}));
        let line = app.session.project().unwrap().boards().next().unwrap().ratsnest().unwrap().lines.remove(0);
        let (a, b) = (line.from.position, line.to.position);
        apply(
            &mut app.session,
            engine::ADD_TRACKS,
            json!({"board":board,"layer":layer,"width_nm":250_000,
            "points":[a,{"x_nm":a.x_nm,"y_nm":2_000_000},{"x_nm":b.x_nm,"y_nm":2_000_000},b]}),
        );
        let revision = app.session.revision();
        let snapshot = app.session.snapshot().unwrap();
        let mut params = json!({"board":board,"expected_revision":revision,"dry_run":true,
            "from":{"footprint":line.from.footprint,"pad":"1"},"to":{"footprint":line.to.footprint,"pad":"1"}});
        let mut expected = app.session.execute(engine::UNROUTE_CONNECTION, &params).unwrap();
        params["dry_run"] = json!(false);
        let body = json!({"method":"engine.execute","params":{"command":engine::UNROUTE_CONNECTION,"inspect":false,"params":params}});
        assert!(app.defer_copper(&body));
        let (first, cancelled, _) = request(&app, body.clone());
        app.start_remote_copper(&ctx, first, false);
        assert!(app.remote_copper.queue.busy());
        app.remote_copper.request.as_mut().unwrap().0.deadline = Instant::now();
        for _ in 0..100 {
            app.poll_remote_copper(&ctx);
        }
        assert_eq!(cancelled.try_recv().unwrap()["code"], "cancelled");
        assert!(Arc::ptr_eq(&snapshot, &app.session.snapshot().unwrap()));
        assert_eq!(app.session.revision(), revision);

        let (next, reply, _) = request(&app, body);
        app.start_remote_copper(&ctx, next, false);
        for _ in 0..100 {
            app.poll_remote_copper(&ctx);
        }
        expected["revision"] = json!(revision + 1);
        expected["dry_run"] = json!(false);
        assert_eq!(reply.try_recv().unwrap()["result"], expected);
        assert_eq!(app.session.revision(), revision + 1);
        assert!(!app.remote_copper.queue.busy());
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

    /// A two-footprint pitch edit on a routed board yields across bounded contact steps.
    /// No revision or copper changes before completion; a concurrent edit invalidates the job.
    #[test]
    fn live_pitch_yields_on_a_large_routed_board_and_rejects_stale_completion() {
        use pcbcraft_engine::{ADD_TRACKS, ALIGN_FOOTPRINTS, PLACE_FOOTPRINT, PLACE_RESISTOR};
        let (mut app, ctx, base) = fixture();
        let mut route = base["params"]["params"].clone();
        // One command creates 255 segments away from the two placements.
        route["points"] = json!((0..256).map(|i| json!({"x_nm":i * 1_000_000,"y_nm":30_000_000})).collect::<Vec<_>>());
        app.session.apply(ADD_TRACKS, &route).unwrap();
        for x in [0, 10_000_000] {
            let revision = app.session.revision();
            app.session.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":revision})).unwrap();
            let symbol = app.session.project().unwrap().symbols().last().unwrap().id();
            let revision = app.session.revision();
            app.session.apply(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0,"expected_revision":revision})).unwrap();
        }
        let before = app.session.snapshot().unwrap();
        let view = before.boards().next().unwrap();
        assert_eq!(view.tracks().len(), 255);
        let mut body = json!({"method":"engine.execute","params":{"command":ALIGN_FOOTPRINTS,"inspect":false,"params":{
            "board":view.target.board,"footprints":view.footprint_ids().collect::<Vec<_>>(),"mode":"distribute_x","pitch_nm":5_080_000}}});
        let mut legacy = body.clone();
        legacy["params"]["params"].as_object_mut().unwrap().remove("pitch_nm");
        assert!(!app.defer_copper(&legacy));
        assert!(app.defer_copper(&body));
        for stale in [false, true] {
            let pending_snapshot = app.session.snapshot().unwrap();
            let revision = app.session.revision();
            body["params"]["params"]["expected_revision"] = json!(revision);
            let (r, reply, _) = request(&app, body.clone());
            app.start_remote_copper(&ctx, r, false);
            assert!(app.remote_copper.queue.busy());
            // At least one indexed contact batch must yield, not just the two setup phases.
            for _ in 0..4 {
                app.poll_remote_copper(&ctx);
                assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
                assert_eq!(app.session.revision(), revision);
                assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &pending_snapshot));
            }
            let concurrent_snapshot = if stale {
                app.session.apply("project.rename", &json!({"name":"Stale pitch fixture"})).unwrap();
                app.session.snapshot()
            } else {
                None
            };
            let completed = (0..300)
                .find_map(|_| {
                    app.poll_remote_copper(&ctx);
                    reply.try_recv().ok()
                })
                .expect("copper preparation finishes within the bounded tick count");
            let view = app.session.project().unwrap().boards().next().unwrap();
            let expected_x = if stale { 10_000_000 } else { 5_080_000 };
            assert_eq!(view.footprints().last().unwrap().position.x_nm, expected_x);
            assert_eq!(view.tracks(), before.boards().next().unwrap().tracks());
            assert_eq!(app.session.revision(), revision + 1);
            if let Some(concurrent_snapshot) = concurrent_snapshot {
                assert_eq!(completed["code"], "stale_revision", "{completed}");
                assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &concurrent_snapshot));
            } else {
                assert_eq!(completed["result"], json!({"revision":revision+1}));
                app.session.apply("edit.undo", &Value::Null).unwrap();
                // Undo may adopt an equivalent Arc after retaining its allocation watermark.
                assert_eq!(app.session.project().unwrap().boards().next().unwrap().footprints().last().unwrap().position.x_nm, 10_000_000);
            }
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
            let authority = r.authority.clone();
            let revision = app.session.revision();
            app.start_remote_copper(&ctx, r, false);
            client.write_all(trailing).unwrap();
            drop(client);
            let deadline = Instant::now() + Duration::from_secs(2);
            while !authority.cancelled() {
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

#[cfg(test)]
mod package_completion_tests {
    use super::*;
    use pcbcraft_engine::{COMPLETE_PACKAGES, PLACE_SYMBOL, library::sources};

    fn fixture() -> (PcbApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PcbApp::default();
        app.session.apply("project.new", &json!({})).unwrap();
        app.session
            .execute(
                sources::LOAD,
                &json!({"source":"completion-test","documents":{
                    "library.json":include_str!("../../../examples/native-parts/library.json"),
                    "symbols/dual.json":include_str!("../../../examples/native-parts/symbols/dual-amplifier.json"),
                }}),
            )
            .unwrap();
        let definition = app.session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
        let digest = sources::symbol_digest(&definition).unwrap();
        for (unit, x) in [("a", 0), ("b", 25_400_000)] {
            app.session
                .apply(
                    PLACE_SYMBOL,
                    &json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,
                        "definition_digest":digest,"unit":unit,"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}),
                )
                .unwrap();
        }
        app.live_control = Some(LiveControl::start(&ctx).unwrap());
        (app, ctx)
    }

    fn request(app: &PcbApp, body: Value) -> (Request, Receiver<Value>, Arc<RequestAuthority>) {
        let (reply, receive) = mpsc::sync_channel(1);
        let authority = Arc::new(RequestAuthority::default());
        (
            Request {
                body,
                deadline: Instant::now() + Duration::from_secs(10),
                reply,
                generation: app.live_control.as_ref().unwrap().current(),
                authority: authority.clone(),
            },
            receive,
            authority,
        )
    }

    fn poll_until_reply(app: &mut PcbApp, ctx: &egui::Context, reply: &Receiver<Value>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll_remote_package_completion(ctx);
            if let Ok(value) = reply.try_recv() {
                return value;
            }
            assert!(Instant::now() < deadline, "timed out waiting for deferred package completion");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Live package completion prepares off the editor thread for both Preview and Apply,
    /// while rechecking revision, endpoint lease, cancellation, deadline and modal policy
    /// immediately before atomic publication.
    #[test]
    fn deferred_package_completion_runs_off_editor_thread_and_rechecks_before_commit() {
        let (mut app, ctx) = fixture();
        let revision = app.session.revision();
        let before = app.session.snapshot().unwrap();
        let preview_body = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"inspect":false,"params":{
            "scope":"project","dry_run":true,"expected_revision":revision}}});
        assert!(app.defer_package_completion(&preview_body));
        let (req, reply, _) = request(&app, preview_body);
        app.start_remote_package_completion(&ctx, req, false);
        assert!(app.remote_package_completion.busy());
        let preview = poll_until_reply(&mut app, &ctx, &reply);
        assert_eq!(preview["result"]["revision"], revision);
        assert_eq!(preview["result"]["changes"].as_array().unwrap().len(), 1);
        assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
        assert!(!app.remote_package_completion.busy());

        for mode in 0..7 {
            let (mut app, ctx) = fixture();
            let revision = app.session.revision();
            let before = app.session.snapshot().unwrap();
            let body = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"inspect":false,"params":{
                "scope":"project","dry_run":false,"expected_revision":revision}}});
            let (req, reply, authority) = request(&app, body.clone());
            app.start_remote_package_completion(&ctx, req, false);
            assert!(app.remote_package_completion.busy());
            assert_eq!(app.session.revision(), revision);
            match mode {
                1 => {
                    app.session.apply("project.rename", &json!({"name":"Concurrent MCP edit"})).unwrap();
                }
                2 => authority.cancel(),
                3 => app.remote_package_completion.pending.as_mut().unwrap().request.deadline = Instant::now(),
                4 => app.live_control = None,
                5 => app.dialog = Dialog::About,
                6 => {
                    let (second_req, second_reply, _) = request(&app, body);
                    app.start_remote_package_completion(&ctx, second_req, false);
                    let busy = second_reply.try_recv().unwrap();
                    assert_eq!(busy["code"], "busy");
                }
                _ => {}
            }
            let result = poll_until_reply(&mut app, &ctx, &reply);
            if mode == 0 || mode == 6 {
                assert_eq!(result["result"]["revision"], revision + 1, "{mode}: {result}");
                assert_eq!(result["result"]["changes"].as_array().unwrap().len(), 1);
                assert_eq!(app.session.revision(), revision + 1);
                app.session.apply("edit.undo", &Value::Null).unwrap();
                assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
            } else {
                let code = match mode {
                    1 => "stale_revision",
                    5 => "unavailable",
                    _ => "cancelled",
                };
                assert_eq!(result["code"], code, "{mode}: {result}");
                if mode != 1 {
                    assert_eq!(app.session.revision(), revision);
                    assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
                }
            }
            assert!(!app.remote_package_completion.busy());
        }
    }

    /// An oversized live package completion returns `cap_exceeded` before publication on Apply
    /// and on dry runs, leaving revision, snapshot and Undo history unchanged.
    #[test]
    fn deferred_package_completion_refuses_oversized_apply_and_dry_run_before_commit() {
        let (mut app, ctx) = fixture();
        let before_rename = app.session.snapshot().unwrap();
        app.session.apply("project.rename", &json!({"name":"Prior edit"})).unwrap();
        let before = app.session.snapshot().unwrap();
        let revision = app.session.revision();
        app.remote_package_completion.reply_limit = Some(128);

        for dry_run in [true, false] {
            let body = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"inspect":false,"params":{
                "scope":"project","dry_run":dry_run,"expected_revision":revision}}});
            let (req, reply, _) = request(&app, body);
            app.start_remote_package_completion(&ctx, req, false);
            let answer = poll_until_reply(&mut app, &ctx, &reply);
            assert_eq!(answer["code"], "cap_exceeded", "dry_run={dry_run}: {answer}");
            assert!(answer["error"].as_str().unwrap().contains("16 MiB"), "{answer}");
            assert_eq!(app.session.revision(), revision);
            assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
            assert!(!app.remote_package_completion.busy());
        }

        // Undo reverts the prior rename rather than a refused package completion.
        app.session.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before_rename, &app.session.snapshot().unwrap()));
        app.session.apply("edit.redo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
        let redo_revision = app.session.revision();

        app.remote_package_completion.reply_limit = None;
        let apply_body = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"inspect":false,"params":{
            "scope":"project","dry_run":false,"expected_revision":redo_revision}}});
        let (req, reply, _) = request(&app, apply_body);
        app.start_remote_package_completion(&ctx, req, false);
        let answer = poll_until_reply(&mut app, &ctx, &reply);
        assert_eq!(answer["result"]["changes"].as_array().unwrap().len(), 1);
        assert_eq!(answer["result"]["revision"], redo_revision + 1);
        assert_eq!(app.session.revision(), redo_revision + 1);
    }
}
