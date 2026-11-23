//! The live editor's MCP adapter. Document commands run on the editor's own session, sharing
//! its undo history; window commands run through the editor's own handlers.

use std::{io, time::Duration};

use craft_control::loopback::{Connection, Token};
use pcbcraft_engine::{
    Session,
    command_api::{self, COMMAND_API_VERSION, DEPRECATIONS},
    error_codes::ErrorCode,
};
use serde_json::{Value, json};

use crate::{
    Failure,
    backend::{Backend, api_metadata, call_session, command_id, describe_engine_commands, execute_warnings, is_window_command, legacy_api_metadata},
};

/// Environment variable used by the desktop configuration and the MCP CLI.
pub const TOKEN_ENV: &str = "PCBCRAFT_CONTROL_TOKEN";
/// The longest reply wait [`Remote::with_wait`] allows.
pub const MAX_WAIT: Duration = Duration::from_secs(900);
/// Files bigger than this are not token files.
const MAX_TOKEN_FILE: u64 = 4096;

const TIMEOUT: Duration = Duration::from_secs(5);
const VERSIONED_EXECUTE: &str = "engine.execute_versioned";

/// Existing live JSON success-frame bound, including the result wrapper and excluding LF.
/// Hosts may preflight the same transport contract before publishing transient review authority.
pub const MAX_REPLY_BYTES: usize = 16 << 20;

/// Check the complete success frame against the receiving client's existing byte limit.
/// Serialization stops at the limit without retaining a second encoded copy. This bounds
/// transport bytes, not the memory or work used to prepare the supplied inspection value.
pub fn check_reply_size(result: &Value) -> Result<(), String> {
    check_reply_size_within(result, MAX_REPLY_BYTES, 0)
}

/// Check the complete live reply frame for a prepared package completion against `MAX_REPLY_BYTES`,
/// returning `cap_exceeded` before publication or before sending an oversized dry run.
pub fn check_package_completion_reply_size(request: &Value, report: &Value, effectful: bool) -> Result<(), Failure> {
    check_package_completion_reply_size_limit(request, report, effectful, MAX_REPLY_BYTES)
}

/// Same as [`check_package_completion_reply_size`] with an explicit byte budget, counting both
/// the `{"result":...}` envelope and any deprecation `warnings` emitted by [`success_reply`].
pub fn check_package_completion_reply_size_limit(request: &Value, report: &Value, effectful: bool, max_bytes: usize) -> Result<(), Failure> {
    let warnings_overhead = if request["method"] == "engine.execute" {
        let warnings = execute_warnings(&request["params"], DEPRECATIONS);
        if warnings.is_empty() { 0 } else { b",\"warnings\":".len() + serde_json::to_vec(&warnings).map_or(0, |encoded| encoded.len()) }
    } else {
        0
    };
    check_reply_size_within(report, max_bytes, warnings_overhead).map_err(|_| {
        let message = if effectful {
            "Package completion reply exceeds the 16 MiB live reply limit; no edit was applied. Run with sheet scope or in a headless session."
        } else {
            "Package completion preview exceeds the 16 MiB live reply limit; no edit was applied. Run with sheet scope or in a headless session."
        };
        Failure::new(ErrorCode::CapExceeded, message)
    })
}

fn check_reply_size_within(result: &Value, max_bytes: usize, extra_overhead: usize) -> Result<(), String> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        /// Count each encoded chunk, refusing before it crosses the remaining byte budget.
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("live reply byte limit exceeded"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        /// Counting has no buffered output to flush.
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // Request::answer wraps the result; craft_control::read_frame excludes the framing newline.
    let overhead = b"{\"result\":}".len().saturating_add(extra_overhead);
    let Some(limit) = max_bytes.checked_sub(overhead) else {
        return Err("Live editor reply exceeds the reply byte limit".to_owned());
    };
    let mut budget = Budget(limit);
    serde_json::to_writer(&mut budget, result).map_err(|error| format!("Live editor reply exceeds the reply byte limit: {error}"))
}

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
        let metadata = reply["command_api"].take();
        let metadata = if metadata["command_api_version"].as_u64().is_some_and(|version| version > 0) && metadata["capabilities"].is_array() {
            metadata
        } else {
            legacy_api_metadata()
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
    let mut reply = serde_json::Map::with_capacity(2);
    reply.insert("result".into(), result);
    if request["method"] == "engine.execute" {
        let warnings = execute_warnings(&request["params"], DEPRECATIONS);
        if !warnings.is_empty() {
            reply.insert("warnings".into(), Value::Array(warnings));
        }
    } else if request["method"] == "engine.commands" {
        reply.insert("command_api".into(), api_metadata(COMMAND_API_VERSION));
    }
    Value::Object(reply)
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
/// as in a headless session. A symbol-field preview that exceeds the live reply limit
/// returns a tool error without applying the edit.
pub fn call_editor(editor: &mut dyn Editor, request: &Value) -> Result<Value, Failure> {
    call_editor_with_package_commit(editor, request, || Ok(()))
}

/// Dispatch through the same live policy, letting the host arbitrate an effectful package
/// completion after preparation and reply-size validation, but before publication. Other
/// commands, previews and no-ops do not invoke the supplied authorization.
pub fn call_editor_with_package_commit(
    editor: &mut dyn Editor,
    request: &Value,
    authorize: impl FnOnce() -> Result<(), Failure>,
) -> Result<Value, Failure> {
    call_editor_with_package_commit_limit(editor, request, MAX_REPLY_BYTES, authorize)
}

fn call_editor_with_package_commit_limit(
    editor: &mut dyn Editor,
    request: &Value,
    package_reply_limit: usize,
    authorize: impl FnOnce() -> Result<(), Failure>,
) -> Result<Value, Failure> {
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
            if id == pcbcraft_engine::COMPLETE_PACKAGES {
                check_deferred_version(id, params)?;
                let prepared = editor.session().prepare_package_completion(&params["params"])?.finish()?;
                check_package_completion_reply_size_limit(request, prepared.report(), prepared.is_effectful(), package_reply_limit)?;
                if prepared.is_effectful() {
                    authorize()?;
                }
                return editor.session().commit_package_completion(prepared).map_err(Failure::from);
            }
            if !is_window_command(id) {
                let result = call_session(editor.session(), method, params)?;
                // This dry run cannot publish an edit. Refuse an oversized reply before the
                // host writes its frame, rather than truncating the proposed documents.
                if id == pcbcraft_engine::SET_SYMBOL_FIELDS
                    && params["params"]["dry_run"].as_bool() == Some(true)
                    && check_reply_size(&result).is_err()
                {
                    return Err(Failure::new(
                        ErrorCode::CapExceeded,
                        "The symbol-field preview exceeds the 16 MiB live reply limit; no edit was applied.",
                    ));
                }
                return match id {
                    pcbcraft_engine::netlist::EXPORT_BOARD_NETLIST => board_netlist_reply(result),
                    pcbcraft_engine::netlist::COMPARE_NETLIST => comparison_netlist_reply(result),
                    _ => Ok(result),
                };
            }
            check_window_version(editor, id, &params["version"])?;
            editor.run_window_command(id, &params["params"])?;
            if params["inspect"].as_bool() == Some(false) { Ok(json!({"revision": editor.session().revision()})) } else { Ok(editor.inspect()) }
        }
        _ => Err(Failure::new(ErrorCode::InvalidRequest, format!("Unknown live control method `{method}`"))),
    }
}

/// Cap the live board export and retain its headless export and dry-run guidance.
fn board_netlist_reply(result: Value) -> Result<Value, Failure> {
    bounded_netlist_reply(
        result,
        "Board netlist reply exceeds the live editor's 16 MiB limit. Save the project and run pcb.export_netlist in a headless session for the full export, or use dry_run for findings only.",
    )
}

/// Cap the live comparison reply using the same budget as a board export.
fn comparison_netlist_reply(result: Value) -> Result<Value, Failure> {
    bounded_netlist_reply(result, "Netlist comparison reply exceeds the live editor's 16 MiB limit.")
}

/// Measure the escaped payload including its `result` envelope, without allocating another
/// buffer. The framing newline is not counted by `Connection::receive`. An oversized reply
/// fails with `unavailable` and the caller's message.
fn bounded_netlist_reply(result: Value, too_large: &str) -> Result<Value, Failure> {
    struct ReplyBudget(usize);

    impl io::Write for ReplyBudget {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| io::Error::other("reply exceeds the live limit"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    serde_json::to_writer(ReplyBudget(MAX_REPLY_BYTES - b"{\"result\":}".len()), &result)
        .map_err(|_| Failure::new(ErrorCode::Unavailable, too_large))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live exports fit at the escaped reply limit and refuse one byte beyond it.
    #[test]
    fn live_board_netlist_reply_counts_escaping_and_envelope() {
        let report = |contents: String| json!({"revision":7,"netlist":{"files":[{"name":"netlist.json","contents":contents}],"findings":[]}});
        let overhead = serde_json::to_vec(&json!({"result":report(String::new())})).unwrap().len();
        let length = MAX_REPLY_BYTES - overhead;
        // CR, newline, quote and backslash each take two bytes on the wire.
        let text = ["\r\n\"\\".repeat(length / 8), "x".repeat(length % 8)].concat();
        assert!(text.len() < MAX_REPLY_BYTES);
        let fits = board_netlist_reply(report(text.clone())).unwrap();
        assert_eq!(serde_json::to_vec(&json!({"result":fits})).unwrap().len(), MAX_REPLY_BYTES);
        let refused = board_netlist_reply(report(text + "x")).unwrap_err();
        assert_eq!(refused.code(), ErrorCode::Unavailable.as_str());
        assert!(refused.message().contains("16 MiB") && refused.message().contains("headless"), "{refused}");
        let dry = json!({"revision":7,"netlist":{"files":[{"name":"netlist.json"}],"findings":[]}});
        assert_eq!(board_netlist_reply(dry.clone()).unwrap(), dry);
    }

    /// Compare replies use the same escaped frame budget but have their own diagnostic.
    #[test]
    fn live_netlist_comparison_reply_has_its_own_size_error() {
        let small = json!({"revision":7,"comparison":{"matches":true}});
        assert_eq!(comparison_netlist_reply(small.clone()).unwrap(), small);
        let large = json!({"comparison":{"notes":[{"message":"\"".repeat(MAX_REPLY_BYTES / 2)}]}});
        let error = comparison_netlist_reply(large).unwrap_err();
        assert_eq!(
            error.to_json(),
            json!({"code":"unavailable","message":"Netlist comparison reply exceeds the live editor's 16 MiB limit.","details":{}})
        );
    }
}

#[cfg(test)]
mod reply_size_tests {
    use super::*;

    /// Match the actual frame reader's success-wrapper budget, accepting the last JSON byte
    /// and refusing the next. UTF-8 characters and JSON escapes count as their encoded bytes.
    #[test]
    fn reply_budget_matches_the_complete_transport_frame() {
        let overhead = serde_json::to_vec(&json!({"result":""})).unwrap().len();
        for (extra, accepted) in [(0, true), (1, false)] {
            let result = json!("x".repeat(MAX_REPLY_BYTES - overhead + extra));
            let mut frame = serde_json::to_vec(&json!({"result":result})).unwrap();
            assert_eq!(frame.len() <= MAX_REPLY_BYTES, accepted);
            if accepted {
                check_reply_size(&result).unwrap();
            } else {
                assert!(check_reply_size(&result).unwrap_err().contains("reply byte limit"));
            }
            frame.push(b'\n');
            let parsed = craft_control::read_frame(&mut std::io::Cursor::new(frame), MAX_REPLY_BYTES).unwrap();
            if accepted {
                assert!(matches!(parsed, Some(craft_control::Frame::Message(_))));
            } else {
                assert!(matches!(parsed, Some(craft_control::Frame::Oversized)));
            }
        }
        let result = json!("\"界\n".repeat(MAX_REPLY_BYTES / 4));
        assert!(serde_json::to_vec(&json!({"result":result})).unwrap().len() > MAX_REPLY_BYTES);
        assert!(check_reply_size(&result).unwrap_err().contains("reply byte limit"));
    }

    /// Package completion enforces the exact live frame cap before publication and on dry runs,
    /// leaving revision, snapshot and Undo history untouched when an Apply is refused.
    #[test]
    fn package_completion_reply_limit_matches_frame_cap_and_refuses_before_commit() {
        use pcbcraft_engine::{COMPLETE_PACKAGES, PLACE_SYMBOL, library::sources};
        use std::sync::Arc;

        struct TestEditor(Session);
        impl Editor for TestEditor {
            fn session(&mut self) -> &mut Session {
                &mut self.0
            }
            fn refusal(&self, _command: &str) -> Option<String> {
                None
            }
            fn window_commands(&self) -> Vec<Value> {
                Vec::new()
            }
            fn inspect(&self) -> Value {
                json!({})
            }
            fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), Failure> {
                Ok(())
            }
        }

        let request = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"params":{"scope":"project"}}});
        let overhead = serde_json::to_vec(&json!({"result":{"revision":1,"changes":[""],"skipped":[]}})).unwrap().len();
        for (extra, accepted) in [(0, true), (1, false)] {
            let report = json!({"revision":1,"changes":["x".repeat(MAX_REPLY_BYTES - overhead + extra)],"skipped":[]});
            let mut frame = serde_json::to_vec(&success_reply(&request, report.clone())).unwrap();
            assert_eq!(frame.len() <= MAX_REPLY_BYTES, accepted);
            for effectful in [true, false] {
                let checked = check_package_completion_reply_size(&request, &report, effectful);
                if accepted {
                    checked.unwrap();
                } else {
                    let error = checked.unwrap_err();
                    assert_eq!(error.code(), ErrorCode::CapExceeded.as_str());
                    assert!(error.message().contains("16 MiB"), "{error}");
                }
            }
            frame.push(b'\n');
            let parsed = craft_control::read_frame(&mut std::io::Cursor::new(frame), MAX_REPLY_BYTES).unwrap();
            assert_eq!(matches!(parsed, Some(craft_control::Frame::Message(_))), accepted);
        }

        let mut editor = TestEditor(Session::default());
        editor.0.apply("project.new", &json!({})).unwrap();
        editor
            .0
            .execute(
                sources::LOAD,
                &json!({"source":"completion-test","documents":{
                    "library.json":include_str!("../../../examples/native-parts/library.json"),
                    "symbols/dual.json":include_str!("../../../examples/native-parts/symbols/dual-amplifier.json"),
                }}),
            )
            .unwrap();
        let definition = editor.0.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
        let digest = sources::symbol_digest(&definition).unwrap();
        for (unit, x) in [("a", 0), ("b", 25_400_000)] {
            editor
                .0
                .apply(
                    PLACE_SYMBOL,
                    &json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,
                        "definition_digest":digest,"unit":unit,"x_nm":x,"y_nm":0,"expected_revision":editor.0.revision()}),
                )
                .unwrap();
        }
        let before_rename = editor.0.snapshot().unwrap();
        editor.0.apply("project.rename", &json!({"name":"Prior edit"})).unwrap();
        let before = editor.0.snapshot().unwrap();
        let revision = editor.0.revision();
        let mut authorized = false;

        let dry_run = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"params":{
            "scope":"project","dry_run":true,"expected_revision":revision}}});
        let dry_err = call_editor_with_package_commit_limit(&mut editor, &dry_run, 64, || {
            authorized = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(dry_err.code(), ErrorCode::CapExceeded.as_str());
        assert!(dry_err.message().contains("preview") && dry_err.message().contains("16 MiB"), "{dry_err}");
        assert!(!authorized);
        assert_eq!(editor.0.revision(), revision);

        let apply = json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"params":{
            "scope":"project","dry_run":false,"expected_revision":revision}}});
        let apply_err = call_editor_with_package_commit_limit(&mut editor, &apply, 64, || {
            authorized = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(apply_err.code(), ErrorCode::CapExceeded.as_str());
        assert!(apply_err.message().contains("no edit was applied"), "{apply_err}");
        assert!(!authorized);
        assert_eq!(editor.0.revision(), revision);
        assert!(Arc::ptr_eq(&before, &editor.0.snapshot().unwrap()));

        // Undo reverts the prior rename rather than a phantom package completion step.
        editor.0.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before_rename, &editor.0.snapshot().unwrap()));
        editor.0.apply("edit.redo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &editor.0.snapshot().unwrap()));
        let redo_revision = editor.0.revision();

        let ok = call_editor_with_package_commit_limit(
            &mut editor,
            &json!({"method":"engine.execute","params":{"command":COMPLETE_PACKAGES,"params":{
                "scope":"project","dry_run":false,"expected_revision":redo_revision}}}),
            MAX_REPLY_BYTES,
            || {
                authorized = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(authorized);
        assert_eq!(ok["changes"].as_array().unwrap().len(), 1);
        assert_eq!(ok["revision"], redo_revision + 1);
        assert_eq!(editor.0.revision(), redo_revision + 1);
    }
}
