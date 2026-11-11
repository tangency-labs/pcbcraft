//! Dispatch MCP tools to a headless session or an open desktop editor.

use pcbcraft_engine::{Project, Session, board::Pad, catalog, schematic::Terminal};
use serde_json::{Value, json};

/// Command namespaces that act on an open editor window instead of the project document.
/// Only a live editor can run them, and they never change the project, its revision or its undo
/// history; only `file.save` changes the unsaved state, by saving. The editor publishes their
/// catalog during live discovery.
pub const WINDOW_NAMESPACES: &[&str] = &["view.", "window.", "arrangement.", "app.", "file."];

/// Whether `id` names a window command, which only an open editor can run.
pub fn is_window_command(id: &str) -> bool {
    WINDOW_NAMESPACES.iter().any(|namespace| id.starts_with(namespace))
}

/// The refusal a session without a window gives for a window command or editor inspection.
pub(crate) fn needs_live_editor(what: &str) -> String {
    format!(
        "{what} acts on an open PCBCraft editor window, and this MCP session has none. \
In the desktop app choose Help → Connect AI assistants…, then connect with `pcbcraft-cli mcp --connect PORT`."
    )
}

/// Something that answers `engine.commands`, `engine.execute`, `project.inspect`,
/// `project.cross_probe` and `editor.inspect`.
pub trait Backend {
    /// Run an engine or project operation and return its result or a tool error.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
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
        "a headless session in this process, not an open window. Window commands (view., window., arrangement., app. and file.) need a live editor"
            .to_owned()
    }
}

/// Run an operation against the supplied session. Its owner controls execution policy.
/// Window commands and editor inspection fail with a reason instead of silently succeeding,
/// because a bare session has no window to act on. Clipboard/export queries retain their
/// requested data even when project inspection is suppressed.
pub(crate) fn call_session(session: &mut Session, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "engine.commands" => Ok(json!(catalog().iter().map(|c| c.to_json()).collect::<Vec<_>>())),
        "engine.execute" => {
            let id = params["command"].as_str().ok_or("missing `command`")?;
            if is_window_command(id) {
                return Err(needs_live_editor(&format!("`{id}`")));
            }
            if params["inspect"].as_bool() == Some(false)
                && !matches!(
                    id,
                    pcbcraft_engine::PREVIEW_UPDATE
                        | pcbcraft_engine::CHECK_DESIGN
                        | pcbcraft_engine::EXPORT_FABRICATION
                        | pcbcraft_engine::clipboard::COPY
                        | pcbcraft_engine::clipboard::CUT
                        | pcbcraft_engine::board::clipboard::COPY
                        | pcbcraft_engine::board::clipboard::CUT
                        | pcbcraft_engine::clipboard_text::LABELS
                        | pcbcraft_engine::clipboard_text::OUTLINE
                        | pcbcraft_engine::library::sources::RESOLVE_PART
                        | pcbcraft_engine::library::sources::PARSE_QUANTITY
                )
            {
                session.apply(id, &params["params"]).map(|revision| json!({"revision": revision})).map_err(|e| e.to_string())
            } else {
                session.execute(id, &params["params"]).map_err(|e| e.to_string())
            }
        }
        "project.inspect" => Ok(session.inspect()),
        "project.cross_probe" => cross_probe(session, params),
        "editor.inspect" => Err(needs_live_editor("Editor inspection")),
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
            let object = pad
                .as_object()
                .filter(|object| object.keys().all(|key| ["board", "footprint", "pad"].contains(&key.as_str())))
                .ok_or("invalid board pad")?;
            let board = match object.get("board") {
                None => project.resolve_board(None),
                Some(id) => project.resolve_board(Some(id.as_str().ok_or("invalid board ID")?)),
            }
            .map_err(|error| error.to_string())?;
            let pad = Pad::parse(
                object.get("footprint").and_then(Value::as_str).ok_or("invalid footprint")?,
                object.get("pad").and_then(Value::as_str).ok_or("invalid pad")?,
            )
            .ok_or("invalid board pad")?
            .on_board(board.board);
            Ok(json!({"terminal": project.terminal_for_pad(pad).map(Terminal::inspect)}))
        }
        _ => Err("provide either a terminal or a pad".to_owned()),
    }
}
