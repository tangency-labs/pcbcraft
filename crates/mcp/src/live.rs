//! The live editor's MCP adapter. Document commands run on the editor's own session, sharing
//! its undo history; window commands run through the editor's own handlers.

use std::time::Duration;

use craft_control::loopback::{Connection, Token};
use pcbcraft_engine::{Session, catalog};
use serde_json::{Value, json};

use crate::backend::{Backend, call_session, is_window_command};

/// Environment variable used by the desktop configuration and the MCP CLI.
pub const TOKEN_ENV: &str = "PCBCRAFT_CONTROL_TOKEN";

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY_BYTES: usize = 16 << 20;

/// MCP backend for an explicitly selected local desktop editor.
pub struct Remote {
    port: u16,
    token: Token,
}

impl Remote {
    /// Validate the copied credential. Connections are opened only when tools make requests.
    pub fn new(port: u16, token: &str) -> Result<Self, String> {
        if port == 0 {
            return Err("the editor port must be nonzero".to_owned());
        }
        Ok(Self { port, token: Token::parse(token).map_err(|error| error.to_string())? })
    }
}

impl Backend for Remote {
    /// Open one authenticated connection per call. Never retry a call after a lost reply.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({"method": method, "params": params});
        let mut peer = Connection::connect(self.port, &self.token, request, TIMEOUT)
            .map_err(|error| format!("Could not send request to the PCBCraft editor on port {}: {error}", self.port))?;
        let lost_reply = || "No reply from the PCBCraft editor. Check the connection and inspect the project before retrying an edit.".to_owned();
        let reply = peer.receive(MAX_REPLY_BYTES).map_err(|error| format!("{} ({error})", lost_reply()))?.ok_or_else(lost_reply)?;
        match (reply.get("result"), reply.get("error").and_then(Value::as_str)) {
            (Some(value), None) => Ok(value.clone()),
            (None, Some(error)) => Err(error.to_owned()),
            _ => Err("Invalid reply from the PCBCraft editor".to_owned()),
        }
    }

    /// Name the selected local editor and what its commands reach during MCP initialization.
    fn describe(&self) -> String {
        format!(
            "the open PCBCraft editor on port {}. Project commands share the window's undo history; \
view., window., app. and file. commands change its views, windows and settings. `inspect_editor` shows them",
            self.port
        )
    }
}

/// What an open editor provides to the live connection. The editor runs on its own thread
/// and owns every policy: which commands may run now, and how window commands act.
pub trait Editor {
    /// The editor's project session, which document commands edit directly.
    fn session(&mut self) -> &mut Session;
    /// Why `command` can't run now (a dialog or file operation, say), or `None` when it can.
    /// Consulted before every command, document or window.
    fn refusal(&self, command: &str) -> Option<String>;
    /// The window commands this editor offers, described like engine commands.
    fn window_commands(&self) -> Vec<Value>;
    /// The editor's windows, views, panels and settings, without project data.
    fn inspect(&self) -> Value;
    /// Run one window command through the handlers the editor's own controls use. A failure
    /// leaves the windows as they were; success never changes the project or its history.
    fn run_window_command(&mut self, id: &str, params: &Value) -> Result<(), String>;
}

/// Serve one live request against the open editor. The editor's refusal applies to every
/// command, including callers that bypass the MCP CLI. Window commands answer with editor
/// inspection (or only the unchanged revision when `inspect` is false); document commands
/// answer as in a headless session.
pub fn call_editor(editor: &mut dyn Editor, request: &Value) -> Result<Value, String> {
    let method = request["method"].as_str().ok_or("missing control method")?;
    let params = &request["params"];
    match method {
        "engine.commands" => {
            let mut commands: Vec<Value> = catalog().iter().map(|command| command.to_json()).collect();
            commands.extend(editor.window_commands());
            Ok(Value::Array(commands))
        }
        "project.inspect" | "project.cross_probe" => call_session(editor.session(), method, params),
        "editor.inspect" => Ok(editor.inspect()),
        "engine.execute" => {
            let id = params["command"].as_str().ok_or("missing `command`")?;
            if let Some(reason) = editor.refusal(id) {
                return Err(reason);
            }
            if !is_window_command(id) {
                return call_session(editor.session(), method, params);
            }
            editor.run_window_command(id, &params["params"])?;
            if params["inspect"].as_bool() == Some(false) { Ok(json!({"revision": editor.session().revision()})) } else { Ok(editor.inspect()) }
        }
        _ => Err(format!("Unknown live control method `{method}`")),
    }
}
