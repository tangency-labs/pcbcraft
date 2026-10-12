//! MCP stdio server plus a loopback bridge into the desktop's actual engine session.
use base64::Engine as _;
use pcbcraft_engine::{Command, Session};
use serde_json::{Value, json};
#[cfg(any(not(target_arch = "wasm32"), test))]
use std::io::BufReader;
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type SharedSession = Arc<Mutex<Session>>;
pub const MAX_MESSAGE: u64 = 1024 * 1024;
pub fn shared(session: Session) -> SharedSession {
    Arc::new(Mutex::new(session))
}
pub fn tool_list() -> Value {
    let object = |properties: Value, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let command = json!({"type":"object","properties":{"command":{"type":"string"},"params":{"type":"object"}},"required":["command","params"],"additionalProperties":false});
    let mut tools = vec![
        json!({"name":"list_commands","description":"Discover every engine command, input schema and unit convention","inputSchema":object(json!({}),&[])}),
        json!({"name":"inspect_design","description":"Inspect the canonical project, stable IDs, revision, history and truthful capability limits","inputSchema":object(json!({}),&[])}),
        json!({"name":"checks","description":"Run initial ERC/DRC and inspect unrouted airwires; returns scope and affected object IDs","inputSchema":object(json!({}),&[])}),
        json!({"name":"execute","description":"Execute one engine command atomically, optionally rejecting a stale revision","inputSchema":object(json!({"command":{"type":"string"},"params":{"type":"object"},"expected_revision":{"type":"integer","minimum":0}}),&["command","params"])}),
        json!({"name":"batch","description":"Execute 1–128 commands as one transaction and undo step; any failure rolls back everything","inputSchema":object(json!({"commands":{"type":"array","items":command,"minItems":1,"maxItems":128},"expected_revision":{"type":"integer","minimum":0}}),&["commands"])}),
        json!({"name":"undo","description":"Undo one transaction","inputSchema":object(json!({}),&[])}),
        json!({"name":"redo","description":"Redo one transaction","inputSchema":object(json!({}),&[])}),
        json!({"name":"new_design","description":"Replace the project with a new empty one; replacement is undoable","inputSchema":object(json!({}),&[])}),
        json!({"name":"sample_design","description":"Load the original LED indicator example; replacement is undoable","inputSchema":object(json!({}),&[])}),
        json!({"name":"open","description":"Open and validate a .pcbcraft project from the server host; replacement is undoable","inputSchema":object(json!({"path":{"type":"string"}}),&["path"])}),
        json!({"name":"save","description":"Atomically save the current project on the server host","inputSchema":object(json!({"path":{"type":"string"}}),&["path"])}),
        json!({"name":"export","description":"Export the supported PCB subset to KiCad, or export a schematic/PCB SVG preview","inputSchema":object(json!({"path":{"type":"string"},"format":{"type":"string","enum":["kicad_pcb","svg"]},"view":{"type":"string","enum":["schematic","pcb"]}}),&["path","format"])}),
        json!({"name":"screenshot","description":"Return a PNG design preview from current session state (not a desktop UI screenshot)","inputSchema":object(json!({"view":{"type":"string","enum":["schematic","pcb"]}}),&["view"])}),
    ];
    for spec in pcbcraft_engine::commands() {
        tools.push(
            json!({"name":spec.id,"description":spec.description,"inputSchema":spec.input_schema}),
        );
    }
    json!({"tools":tools})
}
fn text_result(value: Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
}
fn error_result(error: String) -> Value {
    json!({"content":[{"type":"text","text":error}],"isError":true})
}
fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string parameter: {key}"))
}
fn revision(args: &Value) -> Result<Option<u64>, String> {
    args.get("expected_revision")
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| "expected_revision must be a nonnegative integer".into())
        })
        .transpose()
}

pub fn call_tool(session: &SharedSession, name: &str, args: &Value) -> Value {
    match invoke(session, name, args) {
        Ok(v) => v,
        Err(e) => error_result(e),
    }
}
fn invoke(session: &SharedSession, name: &str, args: &Value) -> Result<Value, String> {
    if !args.is_object() {
        return Err("arguments must be an object".into());
    }
    let tools = tool_list();
    let schema = tools
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
        })
        .and_then(|tool| tool.get("inputSchema"))
        .ok_or("unknown tool")?;
    validate_schema(args, schema, "arguments")?;
    let mut s = session
        .lock()
        .map_err(|_| "session unavailable after internal failure".to_string())?;
    let result = match name {
        "list_commands" => {
            json!({"commands":pcbcraft_engine::commands(),"units":"millimetres and degrees"})
        }
        "inspect_design" => s.inspect(),
        "checks" => s.checks(),
        "execute" => s
            .execute(
                string(args, "command")?,
                args.get("params").cloned().ok_or("missing params")?,
                revision(args)?,
            )
            .map_err(|e| e.to_string())?,
        "batch" => {
            let commands: Vec<Command> =
                serde_json::from_value(args.get("commands").cloned().ok_or("missing commands")?)
                    .map_err(|e| e.to_string())?;
            s.batch(&commands, revision(args)?)
                .map_err(|e| e.to_string())?
        }
        "undo" => s.undo().map_err(|e| e.to_string())?,
        "redo" => s.redo().map_err(|e| e.to_string())?,
        "new_design" => s.replace(Default::default()).map_err(|e| e.to_string())?,
        "sample_design" => s
            .replace(Session::sample().map_err(|e| e.to_string())?.project)
            .map_err(|e| e.to_string())?,
        "open" => {
            let p = pcbcraft_io::load(Path::new(string(args, "path")?))?;
            s.replace(p).map_err(|e| e.to_string())?
        }
        "save" => {
            let path = string(args, "path")?;
            pcbcraft_io::save(&s.project, Path::new(path))?;
            json!({"path":path,"revision":s.revision})
        }
        "export" => {
            let path = string(args, "path")?;
            let text = match string(args, "format")? {
                "kicad_pcb" => pcbcraft_io::kicad_board(&s.project)?,
                "svg" => pcbcraft_io::svg(
                    &s.project,
                    args.get("view").and_then(Value::as_str).unwrap_or("pcb"),
                )?,
                _ => return Err("unsupported export format".into()),
            };
            pcbcraft_io::atomic_write(Path::new(path), text.as_bytes())?;
            json!({"path":path,"revision":s.revision})
        }
        "screenshot" => {
            let view = string(args, "view")?;
            let bytes = pcbcraft_io::preview_png(&s.project, view)?;
            return Ok(
                json!({"content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(bytes)},{"type":"text","text":format!("{view} design preview; revision {}",s.revision)}],"isError":false}),
            );
        }
        command => s
            .execute(command, args.clone(), None)
            .map_err(|e| e.to_string())?,
    };
    Ok(text_result(result))
}

fn validate_schema(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let kind = schema.get("type").and_then(Value::as_str);
    let valid = match kind {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => true,
    };
    if !valid {
        return Err(format!("{path}: expected {}", kind.unwrap_or("value")));
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        return Err(format!("{path}: unsupported value"));
    }
    if let Some(number) = value.as_f64()
        && (schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || schema
                .get("exclusiveMinimum")
                .and_then(Value::as_f64)
                .is_some_and(|min| number <= min))
    {
        return Err(format!("{path}: below minimum"));
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("{path}: missing {key}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            if let Some(child) = properties.and_then(|p| p.get(key)) {
                validate_schema(value, child, &format!("{path}.{key}"))?;
            } else if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
                return Err(format!("{path}: unexpected field {key}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|min| (items.len() as u64) < min)
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|max| (items.len() as u64) > max)
        {
            return Err(format!("{path}: invalid item count"));
        }
        if let Some(child) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_schema(item, child, &format!("{path}[{index}]"))?;
            }
        }
    }
    Ok(())
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub fn handle_rpc(session: &SharedSession, request: Value, connect: Option<&str>) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || method.is_none()
        || id
            .as_ref()
            .is_some_and(|v| !v.is_string() && !v.is_number() && !v.is_null())
    {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid JSON-RPC request",
        ));
    }
    let method = method?;
    let id = id?; // Notifications have no response.
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            if params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_none()
                || !params.get("capabilities").is_some_and(Value::is_object)
                || params
                    .pointer("/clientInfo/name")
                    .and_then(Value::as_str)
                    .is_none()
                || params
                    .pointer("/clientInfo/version")
                    .and_then(Value::as_str)
                    .is_none()
            {
                return Some(rpc_error(
                    id,
                    -32602,
                    "initialize requires protocolVersion, capabilities and clientInfo",
                ));
            }
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-11-25");
            let version = if ["2025-11-25", "2025-06-18", "2024-11-05"].contains(&requested) {
                requested
            } else {
                "2025-11-25"
            };
            json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"pcbcraft","version":env!("CARGO_PKG_VERSION")},"instructions":"100% Rust EDA engine. Use inspect_design and list_commands before edits; batches are atomic. Check capability limits and checks.scope."})
        }
        "ping" => json!({}),
        "tools/list" => tool_list(),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "missing tool name"));
            };
            if !tool_list()
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|t| {
                    t.iter()
                        .any(|t| t.get("name").and_then(Value::as_str) == Some(name))
                })
            {
                return Some(rpc_error(id, -32602, "unknown tool"));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if let Some(address) = connect {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    match forward(address, &request) {
                        Ok(response) => return Some(response),
                        Err(e) => error_result(e),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = address;
                    error_result("live bridge is unavailable in browser builds".into())
                }
            } else {
                call_tool(session, name, &args)
            }
        }
        _ => return Some(rpc_error(id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = vec![];
    let count = reader
        .take(MAX_MESSAGE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count as u64 > MAX_MESSAGE {
        return Err("message exceeds 1 MiB".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}
pub fn serve(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    session: &SharedSession,
    connect: Option<&str>,
) -> Result<(), String> {
    let mut initialized = false;
    while let Some(line) = read_message(reader)? {
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let method = request.get("method").and_then(Value::as_str);
                if !initialized && method.is_some_and(|m| m.starts_with("tools/")) {
                    request
                        .get("id")
                        .map(|id| rpc_error(id.clone(), -32002, "initialize the MCP session first"))
                } else {
                    let initialize = method == Some("initialize");
                    let response = handle_rpc(session, request, connect);
                    if initialize && response.as_ref().is_some_and(|r| r.get("result").is_some()) {
                        initialized = true;
                    }
                    response
                }
            }
            Err(_) => Some(rpc_error(Value::Null, -32700, "parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut *writer, &response).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
fn loopback(address: &str) -> Result<std::net::SocketAddr, String> {
    let address: std::net::SocketAddr = address
        .parse()
        .map_err(|_| "use a literal loopback IP and port, e.g. 127.0.0.1:7991".to_string())?;
    if !address.ip().is_loopback() {
        return Err("control bridge must use a loopback address".into());
    }
    Ok(address)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn forward(address: &str, request: &Value) -> Result<Value, String> {
    let mut stream = std::net::TcpStream::connect_timeout(
        &loopback(address)?,
        std::time::Duration::from_secs(3),
    )
    .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut stream, request).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let text =
        read_message(&mut BufReader::new(stream))?.ok_or("bridge closed without a response")?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn start_bridge(session: SharedSession, address: &str) -> Result<(), String> {
    let listener = std::net::TcpListener::bind(loopback(address)?).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("pcbcraft-control".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                if stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .is_err()
                {
                    continue;
                }
                if let Ok(reader) = stream.try_clone() {
                    let mut reader = BufReader::new(reader);
                    if let Ok(Some(line)) = read_message(&mut reader) {
                        let response = match serde_json::from_str(&line) {
                            Ok(request) => handle_rpc(&session, request, None),
                            Err(_) => Some(rpc_error(Value::Null, -32700, "parse error")),
                        };
                        if let Some(response) = response {
                            let _ = serde_json::to_writer(&mut stream, &response);
                            let _ = stream.write_all(b"\n");
                        }
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schemas_reject_bad_arguments_before_mutation() {
        let session = shared(Session::default());
        for args in [
            json!({"command":"net.create","params":{"name":"A"},"expected_revision":-1}),
            json!({"command":"net.create","params":{"name":"A"},"extra":true}),
        ] {
            assert_eq!(
                call_tool(&session, "execute", &args).get("isError"),
                Some(&json!(true))
            );
        }
        assert_eq!(
            call_tool(&session, "inspect_design", &json!({}))
                .pointer("/structuredContent/revision"),
            Some(&json!(0))
        );
    }
    #[test]
    fn protocol_negotiates_and_notifications_do_not_reply() {
        let s = shared(Session::default());
        let response = handle_rpc(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
            None,
        );
        assert_eq!(
            response
                .and_then(|r| r.get("result").cloned())
                .and_then(|r| r.get("protocolVersion").cloned()),
            Some(json!("2025-06-18"))
        );
        assert!(
            handle_rpc(
                &s,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                None
            )
            .is_none()
        );
    }
    #[test]
    fn tool_errors_are_recoverable_and_stdout_is_json_lines() {
        let s = shared(Session::default());
        let mut output = vec![];
        let initialize = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
        let edit = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"component.move","arguments":{"id":999,"position":{"x":0,"y":0}}}});
        let inspect = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_design"}});
        let messages = format!("{initialize}\nbroken\n{edit}\n{inspect}\n");
        assert!(
            serve(
                &mut BufReader::new(messages.as_bytes()),
                &mut output,
                &s,
                None
            )
            .is_ok()
        );
        let text = String::from_utf8(output).unwrap_or_default();
        let values: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert_eq!(values.len(), 4);
        assert_eq!(
            values.get(2).and_then(|v| v.pointer("/result/isError")),
            Some(&json!(true))
        );
        assert_eq!(
            values.get(3).and_then(|v| v.pointer("/result/isError")),
            Some(&json!(false))
        );
    }
}
