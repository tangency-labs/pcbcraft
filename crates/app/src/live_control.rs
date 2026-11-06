//! Desktop opt-in for live MCP. The socket worker hands requests to the editor thread.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, ScopedJoinHandle, Thread},
    time::{Duration, Instant},
};

use craft_control::loopback::{AcceptedPeer, Listener, Token};
use pcbcraft_engine::Session;
use pcbcraft_mcp::live::{Editor, TOKEN_ENV};
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
    worker: Thread,
}

/// Keep the response channel and deadline with the request until the editor sees it.
pub(super) struct Request {
    body: Value,
    deadline: Instant,
    reply: SyncSender<Value>,
}

impl Request {
    /// The authenticated request, as `{"method": ..., "params": ...}`.
    pub(super) fn body(&self) -> &Value {
        &self.body
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
        let listener = Listener::bind(0)?;
        listener.set_nonblocking(true)?;
        let port = listener.port()?;
        let token = Arc::new(Token::generate()?);
        let stop = Arc::new(AtomicBool::new(false));
        let (send, requests) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        let worker_token = Arc::clone(&token);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new().name("pcbcraft-live-mcp".to_owned()).spawn(move || {
            listen(listener, &worker_token, &worker_stop, &send, &ctx);
            drop(send);
            ctx.request_repaint();
        })?;
        Ok(Self { port, token, requests, stop, worker: worker.thread().clone() })
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
        if Instant::now() < request.deadline {
            return Ok(Some(request));
        }
        request.answer(Err("The editor did not receive the request in time".to_owned()));
        Ok(None)
    }
}

/// The open editor as the live connection sees it, for one request on the editor thread.
pub(super) struct LiveEditor<'a> {
    pub(super) app: &'a mut PcbApp,
    pub(super) ctx: &'a egui::Context,
}

impl Editor for LiveEditor<'_> {
    fn session(&mut self) -> &mut Session {
        &mut self.app.session
    }

    /// Hold every command while a dialog, file operation or project close owns the editor, as
    /// the GUI disables its controls. New Project waits for unsaved changes to be saved or
    /// discarded in the editor, which is where the GUI asks.
    fn refusal(&self, command: &str) -> Option<String> {
        let app = &*self.app;
        if !matches!(app.dialog, Dialog::None) || app.files.busy() || app.project_action_pending() {
            return Some(format!("Finish the current dialog or file operation in the editor before running `{command}`"));
        }
        if command == "project.new" && app.files.dirty(&app.session) {
            return Some("The open project has unsaved changes. Save or discard them in the editor before starting a new project.".to_owned());
        }
        None
    }

    fn window_commands(&self) -> Vec<Value> {
        window_commands::window_catalog()
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
fn listen(listener: Listener, token: &Token, stop: &AtomicBool, send: &SyncSender<Request>, ctx: &egui::Context) {
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
            match thread::Builder::new().spawn_scoped(scope, move || serve_peer(peer, token, stop, send, ctx)) {
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
/// Failed authentication or I/O closes only this peer. A full editor queue gets a busy reply.
fn serve_peer(peer: AcceptedPeer, token: &Token, stop: &AtomicBool, send: &SyncSender<Request>, ctx: &egui::Context) {
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
    let deadline = Instant::now() + EDITOR_TIMEOUT;
    if send.try_send(Request { body, deadline, reply }).is_err() {
        let _ = peer.send(&json!({"error": "The PCBCraft editor is busy"}));
        return;
    }
    ctx.request_repaint();
    if let Ok(response) = response.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let _ = peer.send(&response);
    }
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
}
