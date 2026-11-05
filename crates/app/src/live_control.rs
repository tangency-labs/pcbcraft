//! Desktop opt-in for live MCP. The socket worker hands requests to the editor thread.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, Thread},
    time::{Duration, Instant},
};

use craft_control::loopback::{Listener, Token};
use pcbcraft_engine::Session;
use pcbcraft_mcp::live::{TOKEN_ENV, call_editor};
use serde_json::{Value, json};

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const EDITOR_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// One editor window's local endpoint. Dropping it revokes the port and credential.
pub(super) struct LiveControl {
    port: u16,
    token: Arc<Token>,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
    worker: Thread,
}

/// Keep the response channel and deadline with the request until the editor sees it.
struct Request {
    body: Value,
    deadline: Instant,
    reply: SyncSender<Value>,
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

    /// Run at most one request on the editor thread. Expired requests never reach the session.
    /// Return an error if the listener has stopped so the window can clear its enabled state.
    pub(super) fn poll(&self, session: &mut Session, can_edit: bool) -> Result<(), String> {
        let request = match self.requests.try_recv() {
            Ok(request) => request,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => return Err("The local MCP connection stopped".to_owned()),
        };
        let result = if Instant::now() < request.deadline {
            call_editor(session, &request.body, can_edit)
        } else {
            Err("The editor did not receive the request in time".to_owned())
        };
        let _ = request.reply.try_send(match result {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        });
        Ok(())
    }
}

impl Drop for LiveControl {
    /// Wake the worker so a disabled endpoint stops accepting new connections.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
    }
}

/// Accept one request per connection and keep socket I/O off the editor thread.
fn listen(listener: Listener, token: &Token, stop: &AtomicBool, send: &SyncSender<Request>, ctx: &egui::Context) {
    while !stop.load(Ordering::Acquire) {
        let mut peer = match listener.accept(IO_TIMEOUT) {
            Ok(peer) => peer,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::park_timeout(ACCEPT_POLL);
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        let Ok(body) = peer.authenticate(token) else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            return;
        }
        let (reply, response) = mpsc::sync_channel(1);
        let deadline = Instant::now() + EDITOR_TIMEOUT;
        if send.try_send(Request { body, deadline, reply }).is_err() {
            let _ = peer.send(&json!({"error": "The PCBCraft editor is busy"}));
            continue;
        }
        ctx.request_repaint();
        if let Ok(response) = response.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            let _ = peer.send(&response);
        }
    }
}
