//! JSON-RPC 2.0 framing and the MCP lifecycle / tools / resources methods.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{BufRead, Write},
};

use craft_control::{Frame, read_frame, write_frame};

use crate::backend::Backend;
use pcbcraft_engine::error_codes::ErrorCode;

use crate::{
    Failure,
    tools::{call_tool, check_arguments, failure, tool_definitions},
};
use serde::{
    Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::{Value, json, value::RawValue};

/// The newest protocol revision we speak; offered when the client asks for one we don't know.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// Most messages in one JSON-RPC batch.
const MAX_BATCH: usize = 1000;
/// Longest accepted request line.
const MAX_LINE: usize = 16 << 20;

/// What every MCP client is told at `initialize`.
const INSTRUCTIONS: &str = "PCBCraft is an electronics design app, and these tools drive it through the same commands its menus run. \
`list_commands` shows what exists, `execute` runs one, `inspect_project` shows the result. Changes are undoable with `edit.undo`. \
Project files are never edited directly. Inspect the project to read its current revision before making an edit. \
With a live editor, view., window., arrangement., app. and file. commands change its views, windows, arrangements and settings without editing the project; \
`inspect_editor` gives the view and window IDs they take. With a live editor, unless the user requests otherwise, keep the area you are editing visible \
in the appropriate editor pane. Inspect views with `inspect_editor` before starting, frame planned work with `view.center` and `view.zoom`, and reframe \
after meaningful batches as needed. Use `view.fit` for a whole-document overview. Respect requests to keep the camera fixed or work without changing \
the view, and leave unrelated panes alone.";

pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Only the remap item needs its original JSON; ordinary transport arguments keep their existing codecs.
fn has_raw_remap_item(message: &Value) -> bool {
    let params = &message["params"];
    message.get("id").is_some()
        && message["method"] == "tools/call"
        && params["name"] == "execute"
        && params["arguments"]["command"] == pcbcraft_engine::library::sources::PREVIEW_PIN_REMAP
        && params["arguments"]["params"]["item"].is_object()
}

/// Borrow the selected member's exact bytes, including any repeated keys inside it.
fn raw_member<'a>(text: &'a str, key: &str) -> Result<&'a str, String> {
    let object: BTreeMap<String, &RawValue> = serde_json::from_str(text).map_err(|error| error.to_string())?;
    object.get(key).map(|value| value.get()).ok_or_else(|| format!("Missing raw `{key}` member."))
}

/// Inspect raw drawing or unit map keys without decoding primitive values.
fn duplicate_drawing_keys(text: &str, units: bool) -> Option<&'static str> {
    struct DuplicateKeys(bool);
    impl<'de> Visitor<'de> for DuplicateKeys {
        type Value = Option<&'static str>;

        /// Describe the JSON shape accepted by the duplicate-key check.
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("a drawing or unit map")
        }

        /// Compare decoded keys and inspect every original inactive drawing before any normalization.
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = BTreeSet::new();
            let mut duplicate = false;
            let mut drawing_error = None;
            while let Some((key, body)) = map.next_entry::<String, &RawValue>()? {
                duplicate |= !keys.insert(key);
                if self.0 && drawing_error.is_none() {
                    drawing_error = duplicate_drawing_keys(body.get(), false);
                }
            }
            Ok(if duplicate {
                Some(if self.0 { "Duplicate unit name in the pin remap item." } else { "Duplicate primitive ID in the pin remap item." })
            } else {
                drawing_error
            })
        }
    }
    serde_json::Deserializer::from_str(text).deserialize_map(DuplicateKeys(units)).ok().flatten()
}

/// Detect repeated drawing and unit keys; leave size, shape and semantics to the normalized decoder.
fn raw_remap_duplicate(mut text: &str) -> Option<&'static str> {
    for key in ["params", "arguments", "params", "item"] {
        text = raw_member(text, key).ok()?;
    }
    raw_member(text, "objects")
        .ok()
        .and_then(|objects| duplicate_drawing_keys(objects, false))
        .or_else(|| raw_member(text, "other_units").ok().and_then(|units| duplicate_drawing_keys(units, true)))
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, initialized: false }
    }
    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. Only protocol messages go to
    /// `output`; diagnostics belong on stderr. Consume oversized lines in bounded space so an
    /// agent cannot exhaust memory by sending an arbitrarily long request without a newline.
    pub fn serve(&mut self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        while let Some(frame) = read_frame(&mut input, MAX_LINE)? {
            let reply = match frame {
                Frame::Oversized => Some(error(Value::Null, INVALID_REQUEST, "request too large").to_string()),
                Frame::Message(line) => match String::from_utf8(line) {
                    Ok(line) => self.handle_line(&line),
                    Err(_) => Some(error(Value::Null, PARSE_ERROR, "parse error: input is not UTF-8").to_string()),
                },
            };
            if let Some(reply) = reply {
                write_frame(&mut output, reply.as_bytes())?;
            }
        }
        Ok(())
    }

    /// One line in, at most one line out (notifications get no reply).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        // Check the raw input before trimming: a huge whitespace prefix is still a huge line.
        if line.len() > MAX_LINE {
            return Some(error(Value::Null, INVALID_REQUEST, "request too large").to_string());
        }
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if batch.is_empty() => Some(error(Value::Null, INVALID_REQUEST, "empty batch")),
            Ok(Value::Array(batch)) if batch.len() > MAX_BATCH => Some(error(Value::Null, INVALID_REQUEST, "batch too large")),
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = if batch.iter().any(has_raw_remap_item) {
                    match serde_json::from_str::<Vec<&RawValue>>(line) {
                        Ok(raw) => batch.into_iter().zip(raw).filter_map(|(msg, raw)| self.handle_raw(msg, raw.get())).collect(),
                        Err(e) => return Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}")).to_string()),
                    }
                } else {
                    batch.into_iter().filter_map(|msg| self.handle(msg)).collect()
                };
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => self.handle_raw(msg, line),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Detect duplicate remap keys on the wire without changing normal request validation.
    fn handle_raw(&mut self, msg: Value, raw: &str) -> Option<Value> {
        let remap_error = has_raw_remap_item(&msg).then(|| raw_remap_duplicate(raw)).flatten();
        self.handle_message(msg, remap_error)
    }

    /// Handle an already-decoded message, which has no raw wire keys to inspect.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        self.handle_message(msg, None)
    }

    /// Share JSON-RPC handling for programmatic messages and requests parsed from the wire.
    fn handle_message(&mut self, msg: Value, remap_error: Option<&'static str>) -> Option<Value> {
        let Value::Object(o) = &msg else { return Some(error(Value::Null, INVALID_REQUEST, "message must be an object")) };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a request of ours (we send none) or junk.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "missing `method`"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        Some(match self.request(method, &params, remap_error) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    /// Answer one JSON-RPC method. A command failure, including duplicate raw remap keys
    /// inside a checked tool call, carries `isError` and a stable PCBCraft code. An unknown
    /// method or resource, invalid tool envelope, or resource read failure is a JSON-RPC error.
    fn request(&mut self, method: &str, params: &Value, remap_error: Option<&'static str>) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}, "resources": {"listChanged": false}},
                    "serverInfo": {"name": "pcbcraft", "title": "PCBCraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{INSTRUCTIONS} Backend: {}.", self.backend.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a) => a.clone(),
                };
                check_arguments(name, &args).map_err(|e| (INVALID_PARAMS, e))?;
                if let Some(message) = remap_error {
                    return Ok(failure(&Failure::new(ErrorCode::InvalidParams, message)));
                }
                Ok(call_tool(self.backend.as_mut(), name, &args))
            }
            "resources/list" => Ok(json!({"resources": [
                {"uri": "pcbcraft://project", "name": "Open project", "description": "The project's state", "mimeType": "application/json"},
                {"uri": "pcbcraft://commands", "name": "Command catalog", "description": "Every command with its parameters", "mimeType": "application/json"},
            ]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                let v = match uri {
                    "pcbcraft://project" => self.backend.call("project.inspect", json!({})),
                    "pcbcraft://commands" => self.backend.call("engine.commands", json!({})),
                    _ => return Err((RESOURCE_NOT_FOUND, format!("unknown resource `{uri}`"))),
                }
                .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": v.to_string()}]}))
            }
            _ => Err((METHOD_NOT_FOUND, format!("method `{method}` not found"))),
        }
    }
}
