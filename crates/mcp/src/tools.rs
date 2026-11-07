//! MCP tool definitions and dispatch. Every tool wraps an engine or window command, or an inspection.

use serde_json::{Value, json};

use crate::backend::{Backend, is_window_command, needs_live_editor};

/// What a tool may do, for the client's approval prompts (MCP tool annotations). Clients that
/// auto-approve on these hints must not be told less than the truth.
#[derive(Clone, Copy)]
enum Effect {
    /// Only looks.
    ReadOnly,
    /// Changes the project in memory, undoably.
    Edits,
}

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str], effect: Effect) -> Value {
    let read_only = matches!(effect, Effect::ReadOnly);
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": {"type": "object", "properties": props, "required": required, "additionalProperties": false},
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": false, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// Every tool, as `tools/list` gives them.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "list_commands",
            "List commands",
            "Every available command, with its summary and parameters. `filter` keeps those whose id or summary contains it.",
            json!({"filter": {"type": "string"}}),
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
            "Run a project or window command. Project commands answer with the project, window commands (view., window., arrangement., app., file.) with the editor; set inspect to false to return only the revision.",
            json!({"command": {"type": "string"}, "params": {"type": "object"}, "inspect": {"type": "boolean", "default": true}}),
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
            _ => false,
        };
        if !fits {
            return Err(format!("`{key}` must be of type {kind}"));
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

/// A failed command is a tool result the agent can read, not a protocol error.
fn failure(message: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": message}]})
}

/// Describe a command from the backend's own discovery, so a live editor's window commands are
/// described too. A window command a headless session lacks says what it needs.
fn describe_command(backend: &mut dyn Backend, id: &str) -> Result<Value, String> {
    let commands = backend.call("engine.commands", json!({}))?;
    match commands.as_array().into_iter().flatten().find(|command| command["id"] == id) {
        Some(command) => Ok(command.clone()),
        None if is_window_command(id) => Err(needs_live_editor(&format!("`{id}`"))),
        None => Err(format!("unknown command `{id}`")),
    }
}

/// Run a tool whose arguments have been checked.
pub fn call_tool(backend: &mut dyn Backend, name: &str, args: &Value) -> Value {
    let result = match name {
        "list_commands" => backend.call("engine.commands", json!({})).map(|all| {
            let filter = args["filter"].as_str().unwrap_or_default().to_lowercase();
            let keeps = |c: &&Value| {
                let field = |key: &str| c[key].as_str().unwrap_or_default().to_lowercase();
                field("id").contains(&filter) || field("summary").contains(&filter)
            };
            Value::Array(all.as_array().into_iter().flatten().filter(keeps).cloned().collect())
        }),
        "describe_command" => describe_command(backend, args["command"].as_str().unwrap_or_default()),
        "execute" => backend.call("engine.execute", json!({"command": args["command"], "params": args["params"], "inspect": args["inspect"]})),
        "cross_probe" => backend.call("project.cross_probe", args.clone()),
        "inspect_project" => backend.call("project.inspect", json!({})),
        "inspect_editor" => backend.call("editor.inspect", json!({})),
        _ => Err(format!("unknown tool `{name}`")),
    };
    match result {
        Ok(value) => text(&value),
        Err(message) => failure(&message),
    }
}
