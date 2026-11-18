//! The live editor's MCP adapter. Document commands run on the editor's own session, sharing
//! its undo history; window commands run through the editor's own handlers.

use std::time::Duration;

use craft_control::loopback::{Connection, Token};
use pcbcraft_engine::{
    Session,
    command_api::{self, COMMAND_API_VERSION, DEPRECATIONS},
    error_codes::ErrorCode,
};
use serde_json::{Value, json};

use crate::{
    Failure,
    backend::{Backend, api_metadata, call_session, command_id, describe_engine_commands, execute_warnings, is_window_command},
};

/// Environment variable used by the desktop configuration and the MCP CLI.
pub const TOKEN_ENV: &str = "PCBCRAFT_CONTROL_TOKEN";
/// The longest reply wait [`Remote::with_wait`] allows.
pub const MAX_WAIT: Duration = Duration::from_secs(900);
/// Files bigger than this are not token files.
const MAX_TOKEN_FILE: u64 = 4096;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY_BYTES: usize = 16 << 20;
const VERSIONED_EXECUTE: &str = "engine.execute_versioned";

/// MCP backend for an explicitly selected local desktop editor.
pub struct Remote {
    port: u16,
    token: Token,
    wait: Duration,
}

impl Remote {
    /// Validate the copied credential. Connections are opened only when tools make requests.
    pub fn new(port: u16, token: &str) -> Result<Self, String> {
        if port == 0 {
            return Err("the editor port must be nonzero".to_owned());
        }
        Ok(Self { port, token: Token::parse(token).map_err(|error| error.to_string())?, wait: TIMEOUT })
    }

    /// Read the credential from a token file the editor wrote (the in-app chat's bridge), so it
    /// never passes through the agent that starts this server.
    pub fn from_token_file(port: u16, path: &std::path::Path) -> Result<Self, String> {
        use std::io::Read;
        let unreadable = |error: std::io::Error| format!("could not read the token file {}: {error}", path.display());
        let mut text = String::new();
        std::fs::File::open(path).and_then(|file| file.take(MAX_TOKEN_FILE + 1).read_to_string(&mut text)).map_err(unreadable)?;
        if text.len() as u64 > MAX_TOKEN_FILE {
            return Err(format!("{} is too large to be a token file", path.display()));
        }
        Self::new(port, text.trim())
    }

    /// Wait up to `seconds` (at least the default five, at most [`MAX_WAIT`]) for each reply, for
    /// an editor endpoint that holds requests until the person approves them.
    pub fn with_wait(mut self, seconds: u64) -> Self {
        self.wait = Duration::from_secs(seconds).clamp(TIMEOUT, MAX_WAIT);
        self
    }

    /// Open one authenticated connection per call. Never retry a call after a lost reply.
    /// A connect or send failure reports `connection_failed`; a lost or unreadable reply
    /// reports `no_reply`. Either can follow a delivered request, so inspect before retrying.
    fn request(&self, method: &str, params: Value) -> Result<Value, Failure> {
        // An older editor ignores new fields on engine.execute. It refuses this distinct
        // method instead, so a pinned edit never runs without a version check.
        let method = if method == "engine.execute" && !params["version"].is_null() { VERSIONED_EXECUTE } else { method };
        let request = json!({"method": method, "params": params});
        let mut peer = Connection::connect(self.port, &self.token, request, self.wait).map_err(|error| {
            Failure::new(ErrorCode::ConnectionFailed, format!("Could not send request to the PCBCraft editor on port {}: {error}", self.port))
        })?;
        const LOST_REPLY: &str = "No reply from the PCBCraft editor. Check the connection and inspect the project before retrying an edit.";
        let reply = peer
            .receive(MAX_REPLY_BYTES)
            .map_err(|error| Failure::new(ErrorCode::NoReply, format!("{LOST_REPLY} ({error})")))?
            .ok_or_else(|| Failure::new(ErrorCode::NoReply, LOST_REPLY))?;
        match (reply.get("result"), reply.get("error").and_then(Value::as_str)) {
            (Some(_), None) => Ok(reply),
            (None, Some(error)) => Err(Failure::from_reply(&reply, error)),
            _ => Err(Failure::new(ErrorCode::NoReply, "Invalid reply from the PCBCraft editor")),
        }
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.request(method, params).map(|mut reply| reply["result"].take())
    }

    fn command_catalog(&mut self) -> Result<(Value, Value), Failure> {
        let mut reply = self.request("engine.commands", json!({}))?;
        // Preserve the previous discovery metadata for an old endpoint, but don't promise
        // version checks or warnings it cannot provide.
        let legacy = || json!({"command_api_version": 1, "capabilities": ["command_api", "error_codes"]});
        let metadata = reply["command_api"].take();
        let metadata = if metadata["command_api_version"].as_u64().is_some_and(|version| version > 0) && metadata["capabilities"].is_array() {
            metadata
        } else {
            legacy()
        };
        Ok((reply["result"].take(), metadata))
    }

    fn execute(&mut self, request: Value) -> Result<(Value, Vec<Value>), Failure> {
        let mut reply = self.request("engine.execute", request)?;
        let warnings = reply["warnings"].as_array().into_iter().flatten().filter(|value| value.is_object()).cloned().collect();
        Ok((reply["result"].take(), warnings))
    }

    /// Name the selected local editor and what its commands reach during MCP initialization.
    fn has_live_editor(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!(
            "the open PCBCraft editor on port {}. Project commands share the window's undo history; \
view., window., arrangement., app. and file. commands change its views, windows, arrangements and settings. `inspect_editor` shows them",
            self.port
        )
    }
}

/// What an open editor provides to the live connection. The editor runs on its own thread
/// and owns every policy: which commands may run now, and how window commands act.
pub trait Editor {
    /// Run opt-in UI automation through the live host, with native input lifecycle ownership.
    /// Implementations without input automation retain an explicit refusal.
    fn ui_command(&mut self, _id: &str, _params: &Value) -> Result<Value, Failure> {
        Err(Failure::new(ErrorCode::Unavailable, "This editor does not support UI input automation"))
    }
    /// The editor's project session, which document commands edit directly.
    fn session(&mut self) -> &mut Session;
    /// Why `command` can't run now (a dialog or file operation, say), or `None` when it can.
    /// A refusal fails with `unavailable`.
    /// Consulted before document and presentation commands. UI input uses the host's separate
    /// automation lifecycle so it can operate ordinary modal controls and focused fields.
    fn refusal(&self, command: &str) -> Option<String>;
    /// The window commands this editor offers, described like engine commands.
    fn window_commands(&self) -> Vec<Value>;
    /// The editor's windows, views, panels and settings, without project data.
    fn inspect(&self) -> Value;
    /// Run one window command through the handlers the editor's own controls use. A failure
    /// leaves the windows as they were; success never changes the project or its history.
    fn run_window_command(&mut self, id: &str, params: &Value) -> Result<(), Failure>;
}

/// Normalize the fail-closed wire method before the app applies its ordinary execute policies.
///
/// # Errors
/// A guarded wire request without a stated version is invalid.
pub fn normalize_request(request: &mut Value) -> Result<(), Failure> {
    if request["method"] == VERSIONED_EXECUTE {
        if request["params"]["version"].is_null() {
            return Err(Failure::new(ErrorCode::InvalidRequest, "versioned execution requires `version`"));
        }
        request["method"] = "engine.execute".into();
    }
    Ok(())
}

/// Check the version before deferred engine work reaches its parameter or revision checks.
///
/// # Errors
/// Returns a version failure for a known engine command with a malformed or different version.
pub fn check_deferred_version(id: &str, params: &Value) -> Result<(), Failure> {
    if !params["version"].is_null() && pcbcraft_engine::command(id).is_some() {
        command_api::check_version(id, &params["version"], COMMAND_API_VERSION)?;
    }
    Ok(())
}

/// Form the successful live reply; keep existing execution results intact and add warnings only
/// when used. The legacy catalog array is also unchanged; its API metadata travels beside it.
pub fn success_reply(request: &Value, result: Value) -> Value {
    let mut reply = json!({"result": result});
    if request["method"] == "engine.execute" {
        let warnings = execute_warnings(&request["params"], DEPRECATIONS);
        if !warnings.is_empty() {
            reply["warnings"] = Value::Array(warnings);
        }
    } else if request["method"] == "engine.commands" {
        reply["command_api"] = api_metadata(COMMAND_API_VERSION);
    }
    reply
}

/// Unknown commands keep the window or UI handler's own refusal.
fn check_window_version(editor: &dyn Editor, id: &str, stated: &Value) -> Result<(), Failure> {
    if !stated.is_null() && editor.window_commands().iter().any(|command| command["id"] == id) {
        command_api::check_version(id, stated, COMMAND_API_VERSION)?;
    }
    Ok(())
}

/// Serve one live request against the open editor, including callers that bypass the MCP CLI.
/// Document/presentation commands obey the editor's refusal; UI automation uses its own input
/// lifecycle and returns query data or a queue ticket. Other window commands answer with editor
/// inspection (or only the unchanged revision when `inspect` is false); document commands answer
/// as in a headless session.
pub fn call_editor(editor: &mut dyn Editor, request: &Value) -> Result<Value, Failure> {
    let method = request["method"].as_str().ok_or_else(|| Failure::new(ErrorCode::InvalidRequest, "missing control method"))?;
    let params = &request["params"];
    match method {
        "engine.commands" => {
            let mut commands = describe_engine_commands(DEPRECATIONS);
            commands.extend(editor.window_commands().into_iter().map(|mut command| {
                command_api::mark_deprecations(&mut command, DEPRECATIONS);
                command
            }));
            Ok(Value::Array(commands))
        }
        "project.inspect" | "project.cross_probe" | "project.resolve_counterparts" => call_session(editor.session(), method, params),
        "editor.inspect" => Ok(editor.inspect()),
        "engine.execute" => {
            let id = command_id(params)?;
            if id.starts_with("ui.") {
                check_window_version(editor, id, &params["version"])?;
                return editor.ui_command(id, &params["params"]);
            }
            if let Some(reason) = editor.refusal(id) {
                return Err(Failure::new(ErrorCode::Unavailable, reason));
            }
            if !is_window_command(id) {
                return call_session(editor.session(), method, params);
            }
            check_window_version(editor, id, &params["version"])?;
            editor.run_window_command(id, &params["params"])?;
            if params["inspect"].as_bool() == Some(false) { Ok(json!({"revision": editor.session().revision()})) } else { Ok(editor.inspect()) }
        }
        _ => Err(Failure::new(ErrorCode::InvalidRequest, format!("Unknown live control method `{method}`"))),
    }
}
