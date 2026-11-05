//! Dispatch MCP tools to a headless session or an open desktop editor.

use pcbcraft_engine::{Project, Session, board::Pad, catalog, schematic::Terminal};
use serde_json::{Value, json};

/// Something that answers `engine.commands`, `engine.execute` and `project.inspect`.
pub trait Backend {
    /// Run an engine or project operation and return its result or a tool error.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// Whether this backend exposes the given command in discovery.
    fn permits_command(&self, _id: &str) -> bool {
        true
    }
    /// Where the commands run, for the client's information.
    fn describe(&self) -> String;
}

/// A session of its own, with no window.
#[derive(Debug, Default)]
pub struct Headless {
    session: Session,
}

impl Headless {
    /// Start a separate MCP session from a validated snapshot.
    pub fn with_project(project: std::sync::Arc<Project>) -> Self {
        let mut session = Session::default();
        session.open_snapshot(project);
        Self { session }
    }
}

impl Backend for Headless {
    /// Apply the requested command and return either inspection or the resulting revision.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        call_session(&mut self.session, method, &params)
    }

    fn describe(&self) -> String {
        "a headless session in this process, not an open window".to_owned()
    }
}

/// Run an operation against the supplied session. Its owner controls execution policy.
pub(crate) fn call_session(session: &mut Session, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "engine.commands" => Ok(json!(catalog().iter().map(|c| c.to_json()).collect::<Vec<_>>())),
        "engine.execute" => {
            let id = params["command"].as_str().ok_or("missing `command`")?;
            if params["inspect"].as_bool() == Some(false) {
                session.apply(id, &params["params"]).map(|revision| json!({"revision": revision})).map_err(|e| e.to_string())
            } else {
                session.execute(id, &params["params"]).map_err(|e| e.to_string())
            }
        }
        "project.inspect" => Ok(session.inspect()),
        "project.cross_probe" => cross_probe(session, params),
        _ => Err(format!("unknown method `{method}`")),
    }
}

/// Resolve a pin or pad against the open project's indexes. Missing mappings return null.
fn cross_probe(session: &Session, params: &Value) -> Result<Value, String> {
    let project = session.project().ok_or("no project is open")?;
    match (params.get("terminal"), params.get("pad")) {
        (Some(terminal), None) => {
            let terminal = Terminal::parse(terminal).ok_or("invalid schematic terminal")?;
            Ok(json!({"pad": project.pad_for_terminal(terminal).map(Pad::inspect)}))
        }
        (None, Some(pad)) => {
            let pad = pad
                .as_object()
                .filter(|pad| pad.len() == 2)
                .and_then(|pad| Pad::parse(pad.get("footprint")?.as_str()?, pad.get("pad")?.as_str()?))
                .ok_or("invalid board pad")?;
            Ok(json!({"terminal": project.terminal_for_pad(pad).map(Terminal::inspect)}))
        }
        _ => Err("provide either a terminal or a pad".to_owned()),
    }
}
