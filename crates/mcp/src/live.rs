//! The live editor's MCP adapter and its deliberately narrow command policy.

use std::time::Duration;

use craft_control::loopback::{Connection, Token};
use pcbcraft_engine::{MOVE_SYMBOL, Session, catalog};
use serde_json::{Value, json};

use crate::backend::{Backend, call_session};

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

    /// Live command discovery exposes only the supported edit.
    fn permits_command(&self, id: &str) -> bool {
        id == MOVE_SYMBOL
    }

    /// Name the selected local editor and its edit scope during MCP initialization.
    fn describe(&self) -> String {
        format!("the local PCBCraft editor on port {}. The supported live edit is {MOVE_SYMBOL}", self.port)
    }
}

/// Serve inspection and a revision-checked symbol move against the editor's own session.
/// The allowlist applies at the editor, including to callers that bypass the MCP CLI.
pub fn call_editor(session: &mut Session, request: &Value, can_edit: bool) -> Result<Value, String> {
    let method = request["method"].as_str().ok_or("missing control method")?;
    let params = &request["params"];
    match method {
        "engine.commands" => {
            Ok(json!(catalog().iter().filter(|command| command.id == MOVE_SYMBOL).map(|command| command.to_json()).collect::<Vec<_>>()))
        }
        "project.inspect" | "project.cross_probe" => call_session(session, method, params),
        "engine.execute" if params["command"].as_str() != Some(MOVE_SYMBOL) => Err(format!("The live editor currently supports only {MOVE_SYMBOL}")),
        "engine.execute" if !can_edit => Err("Finish the current dialog or file operation before making a live edit".to_owned()),
        "engine.execute" => call_session(session, method, params),
        _ => Err(format!("Unknown live control method `{method}`")),
    }
}
