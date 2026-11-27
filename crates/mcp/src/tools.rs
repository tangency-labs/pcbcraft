//! MCP tool definitions and dispatch. Every tool wraps an engine or window command, or an inspection.

use pcbcraft_engine::{SessionScope, command_api::catalog_revision, error_codes::ErrorCode};
use serde_json::{Value, json};

use crate::{
    Failure,
    backend::{Backend, is_window_command, needs_live_editor},
};

/// What a tool may do, for the client's approval prompts (MCP tool annotations). Clients that
/// auto-approve on these hints must not be told less than the truth.
#[derive(Clone, Copy)]
enum Effect {
    /// Only looks.
    ReadOnly,
    /// Changes the project or operates a live UI, which can reach file/close controls.
    Edits,
}

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str], effect: Effect) -> Value {
    let read_only = matches!(effect, Effect::ReadOnly);
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": {"type": "object", "properties": props, "required": required, "additionalProperties": false},
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": !read_only, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// Every tool, as `tools/list` gives them.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "list_commands",
            "List commands",
            "Every available command, with its summary and parameters. Set `detail` to false for a compact list of IDs and opening summary sentences. `describe_command` gives the full descriptor for one command. `filter` keeps those whose id or summary contains it. A second part gives the command API version, the catalog revision and the optional features this build offers.",
            json!({"filter": {"type": "string"}, "detail": {"type": "boolean", "default": true}}),
            &[],
            Effect::ReadOnly,
        ),
        tool(
            "describe_command",
            "Describe a command",
            "One command's summary and parameters.",
            json!({"command": {"type": "string"}}),
            &["command"],
            Effect::ReadOnly,
        ),
        tool(
            "execute",
            "Run a command",
            "Run a project or live-window command. ui. commands locate controls or send ordinary input, which can edit, save or close documents; input returns a ticket to poll with ui.inspect. Other window commands (view., window., arrangement., app., file.) answer with the editor; set inspect to false for only the revision. Project edits answer with the project or just the revision. Board-update, annotation and sheet-split previews return review data even with inspect set to false. Optionally pin `version` to the discovered command API version. Successful use of deprecated commands or parameters adds warnings.",
            json!({"command": {"type": "string"}, "params": {"type": "object"}, "inspect": {"type": "boolean", "default": true}, "version": {"type": "integer", "minimum": 1}}),
            &["command"],
            Effect::Edits,
        ),
        tool(
            "cross_probe",
            "Find the linked schematic pin or board pad",
            "Provide either terminal or pad using the address shown by inspect_project.",
            json!({"terminal": {"type":"object"}, "pad": {"type":"object"}}),
            &[],
            Effect::ReadOnly,
        ),
        tool(
            "resolve_counterparts",
            "Resolve all linked pins and pads",
            "Read complete accepted pin/pad correspondence or an explicit unavailable status at expected_revision. Source is a terminal with project/sheet/symbol/terminal, or a pad with project/board/footprint/pad; include kind: terminal or pad. Does not apply pending board updates.",
            json!({"source":pcbcraft_engine::board::correspondence::Source::schema(),"expected_revision":{"type":"integer","minimum":0}}),
            &["source", "expected_revision"],
            Effect::ReadOnly,
        ),
        tool("inspect_project", "Inspect the project", "The open project and whether undo and redo can run.", json!({}), &[], Effect::ReadOnly),
        tool(
            "inspect_editor",
            "Inspect the editor",
            "The open editor's windows, views (with the IDs window commands take), panels, selection, camera and settings. Needs a live editor.",
            json!({}),
            &[],
            Effect::ReadOnly,
        ),
    ]
}

/// Check a call against its tool's schema: no unknown arguments, required ones there, right types.
pub(crate) fn check_arguments(name: &str, args: &Value) -> Result<(), String> {
    let tools = tool_definitions();
    let tool = tools.iter().find(|t| t["name"] == name).ok_or_else(|| format!("unknown tool `{name}`"))?;
    let args = args.as_object().ok_or("arguments must be an object")?;
    let schema = &tool["inputSchema"];
    for (key, value) in args {
        let kind = schema["properties"][key]["type"].as_str().ok_or_else(|| format!("`{name}` has no argument `{key}`"))?;
        let fits = match kind {
            "string" => value.is_string(),
            "object" => value.is_object(),
            "boolean" => value.is_boolean(),
            "integer" => value.is_i64() || value.is_u64(),
            _ => false,
        };
        if !fits {
            return Err(format!("`{key}` must be of type {kind}"));
        }
        if name == "execute" && key == "version" && value.as_u64().is_none_or(|version| version == 0) {
            return Err("`version` must be a positive integer".into());
        }
    }
    for key in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !args.contains_key(key) {
            return Err(format!("`{name}` needs `{key}`"));
        }
    }
    Ok(())
}

fn text(value: &Value) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}]})
}

/// The command list as clients have always read it, followed by the command API version, the
/// revision of the whole unfiltered catalog `all` and the optional features this build offers,
/// which are also the structured result.
fn listing(commands: &Value, all: &[Value], mut about: Value, scope: Option<SessionScope>) -> Value {
    about["catalog_revision"] = Value::String(catalog_revision(all));
    if let Some(scope) = scope {
        about["scope"] = json!(scope.as_str());
    }
    json!({
        "content": [{"type": "text", "text": commands.to_string()}, {"type": "text", "text": about.to_string()}],
        "structuredContent": about,
    })
}

/// A command's descriptor without its parameters and title, and with only the first sentence of
/// its summary; `describe_command` gives the rest. Full descriptors for the whole live catalog
/// exceed what agents whose clients cannot open long tool results can read (#636).
fn brief(command: &Value) -> Value {
    let mut command = command.clone();
    if let Some(fields) = command.as_object_mut() {
        fields.remove("params");
        fields.remove("title");
        if let Some(Value::String(summary)) = fields.get_mut("summary")
            && let Some(end) = summary.find(". ")
        {
            summary.truncate(end + 1);
        }
    }
    command
}

/// A failed command is a tool result the agent can read, not a protocol error. The message
/// stays the first text part as clients have always read it; the second part and the
/// structured result carry the stable code and details.
pub(crate) fn failure(failure: &Failure) -> Value {
    let error = json!({"error": failure.to_json()});
    json!({
        "isError": true,
        "content": [{"type": "text", "text": failure.message()}, {"type": "text", "text": error.to_string()}],
        "structuredContent": error,
    })
}

/// Describe a command from the backend's own discovery, so a live editor's window commands are
/// described too. Keep the descriptor identical to the catalog and append session metadata
/// separately. A window command a headless session lacks says what it needs.
fn describe_command(backend: &mut dyn Backend, id: &str) -> Result<Value, Failure> {
    let commands = backend.call("engine.commands", json!({}))?;
    match commands.as_array().into_iter().flatten().find(|command| command["id"] == id) {
        Some(command) => Ok(match backend.scope() {
            Some(scope) => {
                let about = json!({"scope":scope.as_str()});
                json!({
                    "content": [{"type":"text","text":command.to_string()}, {"type":"text","text":about.to_string()}],
                    "structuredContent": about,
                })
            }
            None => text(command),
        }),
        None if is_window_command(id) && !backend.has_live_editor() => Err(needs_live_editor(&format!("`{id}`"))),
        None => Err(Failure::new(ErrorCode::UnknownCommand, format!("unknown command `{id}`"))),
    }
}

/// Preserve the first text part and append warning metadata only on warned success.
fn execute(backend: &mut dyn Backend, args: &Value) -> Value {
    let mut request = json!({"command": args["command"], "params": args["params"], "inspect": args["inspect"]});
    if let Some(version) = args.get("version") {
        request["version"] = version.clone();
    }
    match backend.execute(request) {
        Ok((result, warnings)) if warnings.is_empty() => text(&result),
        Ok((result, warnings)) => {
            let metadata = json!({"warnings": warnings});
            json!({"content": [{"type": "text", "text": result.to_string()}, {"type": "text", "text": metadata.to_string()}], "structuredContent": metadata})
        }
        Err(error) => failure(&error),
    }
}

/// Run a tool whose arguments have been checked.
pub fn call_tool(backend: &mut dyn Backend, name: &str, args: &Value) -> Value {
    let result = match name {
        "list_commands" => {
            return match backend.command_catalog() {
                Ok((all, metadata)) => {
                    let all = all.as_array().cloned().unwrap_or_default();
                    let filter = args["filter"].as_str().unwrap_or_default().to_lowercase();
                    let keeps = |c: &&Value| {
                        let field = |key: &str| c[key].as_str().unwrap_or_default().to_lowercase();
                        field("id").contains(&filter) || field("summary").contains(&filter)
                    };
                    let detail = args["detail"].as_bool().unwrap_or(true);
                    let listed = all.iter().filter(keeps).map(|command| if detail { command.clone() } else { brief(command) }).collect();
                    listing(&Value::Array(listed), &all, metadata, backend.scope())
                }
                Err(message) => failure(&message),
            };
        }
        "describe_command" => {
            return describe_command(backend, args["command"].as_str().unwrap_or_default()).unwrap_or_else(|error| failure(&error));
        }
        "execute" => return execute(backend, args),
        "cross_probe" => backend.call("project.cross_probe", args.clone()),
        "resolve_counterparts" => backend.call("project.resolve_counterparts", args.clone()),
        "inspect_project" => backend.call("project.inspect", json!({})),
        "inspect_editor" => backend.call("editor.inspect", json!({})),
        _ => Err(Failure::new(ErrorCode::InvalidRequest, format!("unknown tool `{name}`"))),
    };
    match result {
        Ok(value) => text(&value),
        Err(error) => failure(&error),
    }
}
