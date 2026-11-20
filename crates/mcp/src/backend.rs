//! Dispatch MCP tools to a headless session or an open desktop editor.

use pcbcraft_engine::{
    Project, Session, SessionScope,
    board::Pad,
    catalog,
    command_api::{self, CAPABILITIES, COMMAND_API_VERSION, DEPRECATIONS, Deprecation},
    error_codes::ErrorCode,
    schematic::Terminal,
};
use serde_json::{Value, json};

use crate::Failure;

/// Command namespaces that act on an open editor window instead of the project document.
/// Only a live editor can run them. Presentation commands retain document history; `file.save`
/// saves and `ui.` can invoke any ordinary UI action, including authored edits or file controls.
/// The editor publishes their catalog during live discovery.
pub const WINDOW_NAMESPACES: &[&str] = &["view.", "window.", "arrangement.", "app.", "file.", "ui."];

/// Whether `id` names a window command, which only an open editor can run.
pub fn is_window_command(id: &str) -> bool {
    WINDOW_NAMESPACES.iter().any(|namespace| id.starts_with(namespace))
}

/// The refusal a session without a window gives for a window command or editor inspection.
pub(crate) fn needs_live_editor(what: &str) -> Failure {
    Failure::new(
        ErrorCode::NeedsLiveEditor,
        format!(
            "{what} acts on an open PCBCraft editor window, and this MCP session has none. \
In the desktop app choose Help → Connect AI assistants…, then connect with `pcbcraft-cli mcp --connect PORT`."
        ),
    )
}

/// The `command` of an `engine.execute` request.
pub(crate) fn command_id(params: &Value) -> Result<&str, Failure> {
    params["command"].as_str().ok_or_else(|| Failure::new(ErrorCode::InvalidRequest, "missing `command`"))
}

/// Something that answers `engine.commands`, `engine.execute`, `project.inspect`,
/// `project.cross_probe`, `project.resolve_counterparts` and `editor.inspect`.
pub trait Backend {
    /// Run an engine or project operation and return its result or a coded tool error.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure>;
    /// Whether discovery describes an editor, rather than a headless project session.
    fn has_live_editor(&self) -> bool {
        false
    }
    /// The fixed command scope, if this backend owns and can attest to it.
    fn scope(&self) -> Option<SessionScope> {
        None
    }
    /// Where the commands run, for the client's information.
    fn describe(&self) -> String;
    /// Discover commands and their serving backend's API metadata. The default retains the
    /// legacy metadata; a backend must explicitly opt in to version checks or warnings.
    fn command_catalog(&mut self) -> Result<(Value, Value), Failure> {
        self.call("engine.commands", json!({})).map(|commands| (commands, legacy_api_metadata()))
    }
    /// Execute a command, returning its ordinary result and any deprecation warnings separately.
    /// The default only accepts unpinned requests, since a legacy handler may ignore a pin.
    fn execute(&mut self, request: Value) -> Result<(Value, Vec<Value>), Failure> {
        if request.get("version").is_some() {
            return Err(Failure::new(ErrorCode::InvalidRequest, "This backend does not support command API version pins"));
        }
        self.call("engine.execute", request).map(|result| (result, Vec::new()))
    }
}

/// Discovery metadata before version pinning or deprecation warnings were supported.
pub(crate) fn legacy_api_metadata() -> Value {
    json!({"command_api_version": 1, "capabilities": ["command_api", "error_codes"]})
}

pub(crate) fn api_metadata(version: u32) -> Value {
    json!({"command_api_version": version, "capabilities": CAPABILITIES})
}

pub(crate) fn execute_warnings(request: &Value, deprecations: &[Deprecation]) -> Vec<Value> {
    request["command"].as_str().map_or_else(Vec::new, |id| command_api::deprecation_warnings(id, &request["params"], deprecations))
}

pub(crate) fn describe_engine_commands(deprecations: &[Deprecation]) -> Vec<Value> {
    catalog()
        .iter()
        .map(|command| {
            let mut described = command.to_json();
            command_api::mark_deprecations(&mut described, deprecations);
            described
        })
        .collect()
}

/// A session of its own, with no window.
#[derive(Debug)]
pub struct Headless {
    session: Session,
    api_version: u32,
    deprecations: &'static [Deprecation],
}

impl Default for Headless {
    fn default() -> Self {
        Self { session: Session::default(), api_version: COMMAND_API_VERSION, deprecations: DEPRECATIONS }
    }
}

impl Headless {
    /// Start a separate MCP session from a validated snapshot.
    pub fn with_project(project: std::sync::Arc<Project>) -> Self {
        Self::with_scope(SessionScope::Edit, Some(project))
    }

    /// Start a headless MCP session with fixed access and an optional validated project.
    pub fn with_scope(scope: SessionScope, project: Option<std::sync::Arc<Project>>) -> Self {
        let mut session = Session::with_scope(scope);
        if let Some(project) = project {
            session.open_snapshot(project);
        }
        Self { session, ..Self::default() }
    }

    /// Install a trusted in-memory fixture source without granting MCP catalog mutation.
    #[cfg(test)]
    pub(crate) fn install_fixture_source(&mut self, id: &str, documents: &pcbcraft_engine::library::sources::Documents) {
        use pcbcraft_engine::library::sources::NativeSource;
        self.session.install_library_source(id, std::sync::Arc::new(NativeSource::load(documents).unwrap())).unwrap();
    }

    #[cfg(test)]
    pub(crate) fn with_command_api(api_version: u32, deprecations: &'static [Deprecation]) -> Self {
        Self { api_version, deprecations, ..Self::default() }
    }
}

impl Backend for Headless {
    /// Apply the requested command and return either inspection or the resulting revision.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        if method == "engine.commands" {
            return Ok(Value::Array(describe_engine_commands(self.deprecations)));
        }
        serve_session(&mut self.session, method, &params, self.api_version)
    }

    fn command_catalog(&mut self) -> Result<(Value, Value), Failure> {
        self.call("engine.commands", json!({})).map(|commands| (commands, api_metadata(self.api_version)))
    }

    fn execute(&mut self, request: Value) -> Result<(Value, Vec<Value>), Failure> {
        let result = serve_session(&mut self.session, "engine.execute", &request, self.api_version)?;
        Ok((result, execute_warnings(&request, self.deprecations)))
    }

    /// Report the engine's fixed scope, not a client-provided preference.
    fn scope(&self) -> Option<SessionScope> {
        Some(self.session.scope())
    }

    /// Describe this headless process and the scope it actually enforces.
    fn describe(&self) -> String {
        format!(
            "a headless session in this process with scope `{}`, not an open window. Window commands (view., window., arrangement., app. and file.) need a live editor",
            self.session.scope().as_str()
        )
    }
}

/// Run an operation against the supplied session. Its owner controls execution policy.
/// Window commands and editor inspection fail with a reason instead of silently succeeding,
/// because a bare session has no window to act on. Clipboard/export queries and provisional
/// property and annotation previews retain their data even when inspection is suppressed.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn call_session(session: &mut Session, method: &str, params: &Value) -> Result<Value, Failure> {
    serve_session(session, method, params, COMMAND_API_VERSION)
}

/// Dispatch with the serving backend's version before invoking engine handlers.
/// Symbol-field previews return the engine result even when inspection is disabled.
fn serve_session(session: &mut Session, method: &str, params: &Value, api_version: u32) -> Result<Value, Failure> {
    match method {
        "engine.commands" => Ok(Value::Array(describe_engine_commands(DEPRECATIONS))),
        "engine.execute" => {
            let id = command_id(params)?;
            if is_window_command(id) {
                return Err(needs_live_editor(&format!("`{id}`")));
            }
            if !params["version"].is_null() && pcbcraft_engine::command(id).is_some() {
                command_api::check_version(id, &params["version"], api_version)?;
            }
            if params["inspect"].as_bool() == Some(false)
                && !(id == pcbcraft_engine::SET_SYMBOL_FIELDS && params["params"]["dry_run"].as_bool() == Some(true))
                && !matches!(
                    id,
                    pcbcraft_engine::nets::INSPECT_CLASSES
                        | pcbcraft_engine::board::class_rules::ROUTE_RULES
                        | pcbcraft_engine::board::route::PLAN_ROUTE
                        | pcbcraft_engine::net_classes::INSPECT
                        | pcbcraft_engine::net_classes::RESOLVE
                        | pcbcraft_engine::part_groups::INSPECT
                        | pcbcraft_engine::part_groups::INSPECT_GROUPS
                        | pcbcraft_engine::PREVIEW_UPDATE
                        | pcbcraft_engine::annotation::PREVIEW
                        | pcbcraft_engine::PREVIEW_ASSEMBLY_PROFILE
                        | pcbcraft_engine::PREVIEW_BOARD_SETUP
                        | pcbcraft_engine::no_connect::CHECK
                        | pcbcraft_engine::CHECK_DESIGN
                        | pcbcraft_engine::EXPORT_FABRICATION
                        | pcbcraft_engine::board::arc::SET_ARCS
                        | pcbcraft_engine::netlist::EXPORT_NETLIST
                        | pcbcraft_engine::netlist::EXPORT_BOARD_NETLIST
                        | pcbcraft_engine::netlist::COMPARE_NETLIST
                        | pcbcraft_engine::clipboard::COPY
                        | pcbcraft_engine::clipboard::CUT
                        | pcbcraft_engine::board::clipboard::COPY
                        | pcbcraft_engine::board::clipboard::CUT
                        | pcbcraft_engine::clipboard_text::LABELS
                        | pcbcraft_engine::clipboard_text::OUTLINE
                        | pcbcraft_engine::library::sources::AVAILABLE_REVISIONS
                        | pcbcraft_engine::library::sources::PREVIEW_FOOTPRINT_REVISION
                        | pcbcraft_engine::library::sources::RESOLVE_PART
                        | pcbcraft_engine::library::sources::PARSE_QUANTITY
                        | pcbcraft_engine::library::sources::PREPARE_PART_PUBLICATION
                        | pcbcraft_engine::library::sources::PREPARE_PUBLICATION
                        | pcbcraft_engine::library::sources::PREVIEW_PIN_REMAP
                )
            {
                session.apply(id, &params["params"]).map(|revision| json!({"revision": revision})).map_err(Failure::from)
            } else {
                session.execute(id, &params["params"]).map_err(Failure::from)
            }
        }
        "project.inspect" => Ok(session.inspect()),
        "project.cross_probe" => cross_probe(session, params),
        "project.resolve_counterparts" => {
            let source = pcbcraft_engine::board::correspondence::Source::parse(&params["source"])
                .map_err(|message| Failure::new(ErrorCode::InvalidParams, message))?;
            let revision = params["expected_revision"]
                .as_u64()
                .ok_or_else(|| Failure::new(ErrorCode::InvalidParams, "expected_revision must be an unsigned integer"))?;
            let mut result = session.resolve_counterparts(source, revision).inspect();
            result["revision"] = json!(session.revision());
            result["opening"] = json!(session.history_root_revision());
            Ok(result)
        }
        "editor.inspect" => Err(needs_live_editor("Editor inspection")),
        _ => Err(Failure::new(ErrorCode::InvalidRequest, format!("unknown method `{method}`"))),
    }
}

/// Resolve a pin or pad against the open project's indexes. Missing mappings return null.
fn cross_probe(session: &Session, params: &Value) -> Result<Value, Failure> {
    let invalid = |message: &str| Failure::new(ErrorCode::InvalidParams, message);
    let project = session.project().ok_or_else(|| Failure::new(ErrorCode::Unavailable, "no project is open"))?;
    match (params.get("terminal"), params.get("pad")) {
        (Some(terminal), None) => {
            let terminal = Terminal::parse(terminal).ok_or_else(|| invalid("invalid schematic terminal"))?;
            Ok(json!({"pad": project.pad_for_terminal(terminal).map(Pad::inspect)}))
        }
        (None, Some(pad)) => {
            let object = pad
                .as_object()
                .filter(|object| object.keys().all(|key| ["board", "footprint", "pad"].contains(&key.as_str())))
                .ok_or_else(|| invalid("invalid board pad"))?;
            let board = match object.get("board") {
                None => project.resolve_board(None),
                Some(id) => project.resolve_board(Some(id.as_str().ok_or_else(|| invalid("invalid board ID"))?)),
            }
            .map_err(Failure::from)?;
            let pad = Pad::parse(
                object.get("footprint").and_then(Value::as_str).ok_or_else(|| invalid("invalid footprint"))?,
                object.get("pad").and_then(Value::as_str).ok_or_else(|| invalid("invalid pad"))?,
            )
            .ok_or_else(|| invalid("invalid board pad"))?
            .on_board(board.board);
            Ok(json!({"terminal": project.terminal_for_pad(pad).map(Terminal::inspect)}))
        }
        _ => Err(invalid("provide either a terminal or a pad")),
    }
}
