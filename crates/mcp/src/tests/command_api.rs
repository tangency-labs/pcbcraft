use super::{Headless, PLACE_RESISTOR, Server, Session, Value, body, call, json, rpc, server};
use pcbcraft_engine::command_api::{COMMAND_API_VERSION, Deprecation};

const FIXTURE: &[Deprecation] = &[Deprecation { command: "project.new", parameter: Some("create_board"), since: "0.2.0", replacement: None }];

/// An implementation of only the legacy methods advertises no new flags and never sees a pin.
#[test]
fn legacy_backend_refuses_pins_without_dispatching_and_still_executes_unpinned() {
    use crate::{Backend, Failure};
    struct LegacyBackend;
    impl Backend for LegacyBackend {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
            match method {
                "engine.commands" => Ok(json!([{"id": "project.new"}])),
                "engine.execute" => {
                    assert!(params.get("version").is_none(), "a pinned call reached the legacy handler");
                    assert_eq!(params["command"], "project.new");
                    Ok(json!({"revision": 1}))
                }
                _ => panic!("unexpected backend request: {method}"),
            }
        }
        fn describe(&self) -> String {
            "a legacy backend".into()
        }
    }
    let mut s = Server::new(Box::new(LegacyBackend));
    let listed = call(&mut s, "list_commands", json!({}));
    assert_eq!(listed["structuredContent"]["capabilities"], json!(["command_api", "error_codes"]));
    let pinned = call(&mut s, "execute", json!({"command": "project.new", "version": COMMAND_API_VERSION}));
    assert_eq!(pinned["isError"], true);
    assert_eq!(pinned["structuredContent"]["error"]["code"], "invalid_request");
    let plain = call(&mut s, "execute", json!({"command": "project.new"}));
    assert_eq!(plain, json!({"content": [{"type": "text", "text": "{\"revision\":1}"}]}));
}

/// Exercise the changed API fixture, discovery, nonexecution on mismatch, and warning shape.
#[test]
fn a_version_two_api_refuses_mismatches_and_warns_on_deprecated_use() {
    let mut fixture = Server::new(Box::new(Headless::with_command_api(2, FIXTURE)));
    let list = call(&mut fixture, "list_commands", json!({"filter": "project.new"}));
    assert_eq!(list["structuredContent"]["command_api_version"], 2);
    let command = body(&call(&mut fixture, "describe_command", json!({"command": "project.new"})));
    assert_eq!(body(&list), json!([command]));
    let param = &command["params"]["properties"]["create_board"];
    assert_eq!(param["deprecated"], true);
    assert_eq!(param["x-deprecation"], json!({"since": "0.2.0", "replacement": null}));

    let execute = |fixture: &mut Server, version| {
        call(fixture, "execute", json!({"command": "project.new", "params": {"create_board": true}, "version": version}))
    };
    for stated in [1, 3, u64::MAX] {
        let refused = execute(&mut fixture, stated);
        let error = &refused["structuredContent"]["error"];
        assert_eq!(refused["isError"], true);
        assert_eq!(error["code"], "version_mismatch");
        assert_eq!(error["details"], json!({"command": "project.new", "stated_version": stated, "current_version": 2}));
        assert_eq!(refused["content"][0]["text"], error["message"]);
        assert!(refused["structuredContent"].get("warnings").is_none());
        assert!(body(&call(&mut fixture, "inspect_project", json!({})))["project"].is_null());
    }
    let warned = execute(&mut fixture, 2);
    assert_eq!(body(&warned)["project"]["boards"].as_array().unwrap().len(), 1);
    let warning = &warned["structuredContent"]["warnings"].as_array().unwrap();
    assert_eq!(warning.len(), 1);
    assert_eq!(warning[0]["code"], "deprecated");
    assert_eq!(warning[0]["parameter"], "create_board");
    let warning_part: Value = serde_json::from_str(warned["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(warning_part, warned["structuredContent"]);

    let unpinned = call(&mut fixture, "execute", json!({"command": "project.new", "params": {"create_board": false}}));
    assert_eq!(unpinned["structuredContent"]["warnings"].as_array().unwrap().len(), 1);
    let plain = call(&mut fixture, "execute", json!({"command": "project.new"}));
    assert!(body(&plain)["project"].is_object());
    assert_eq!(plain.as_object().unwrap().len(), 1);
    assert_eq!(plain["content"].as_array().unwrap().len(), 1);
}

/// Existing classifications and revision priority still apply; malformed pins never dispatch.
#[test]
fn a_version_pin_preserves_unknown_window_and_revision_refusals() {
    let mut s = server();
    let created = body(&call(&mut s, "execute", json!({"command": "project.new", "version": COMMAND_API_VERSION})));
    let revision = created["revision"].as_u64().unwrap();
    let code = |s: &mut Server, args| call(s, "execute", args)["structuredContent"]["error"]["code"].clone();
    let stale = json!({"expected_revision": revision + 1, "bad_key": true});
    assert_eq!(code(&mut s, json!({"command": PLACE_RESISTOR, "params": stale})), "stale_revision");
    assert_eq!(code(&mut s, json!({"command": PLACE_RESISTOR, "params": stale, "version": COMMAND_API_VERSION})), "stale_revision");
    assert_eq!(code(&mut s, json!({"command": PLACE_RESISTOR, "params": stale, "version": COMMAND_API_VERSION + 1})), "version_mismatch");
    assert_eq!(code(&mut s, json!({"command": "view.fit", "version": COMMAND_API_VERSION + 1})), "needs_live_editor");
    assert_eq!(code(&mut s, json!({"command": "project.nothing", "version": COMMAND_API_VERSION + 1})), "unknown_command");
    assert_eq!(body(&call(&mut s, "inspect_project", json!({})))["revision"], revision);
    for version in [json!(0), json!(-1), json!(1.0), json!("1"), json!(true), Value::Null] {
        let result = rpc(&mut s, "tools/call", json!({"name": "execute", "arguments": {"command": "project.new", "version": version}}));
        assert_eq!(result["error"]["code"], -32602, "{result}");
    }
    let mut session = Session::default();
    assert!(crate::backend::call_session(&mut session, "engine.execute", &json!({"command": "project.new", "version": null})).is_ok());
    let invalid = crate::backend::call_session(&mut session, "engine.execute", &json!({"command": "project.new", "version": "1"})).unwrap_err();
    assert_eq!(invalid.code(), "invalid_params");
}

/// Known live commands share one version, without bypassing the existing editor refusal.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_project_window_and_ui_commands_check_the_same_api_version() {
    use crate::{
        Failure,
        live::{Editor, call_editor},
    };
    #[derive(Default)]
    struct OpenEditor {
        session: Session,
        blocked: bool,
        ran: Vec<String>,
    }
    impl Editor for OpenEditor {
        fn session(&mut self) -> &mut Session {
            &mut self.session
        }
        fn refusal(&self, _command: &str) -> Option<String> {
            self.blocked.then(|| "finish the dialog".into())
        }
        fn window_commands(&self) -> Vec<Value> {
            vec![json!({"id": "view.fit"}), json!({"id": "ui.inspect"})]
        }
        fn inspect(&self) -> Value {
            json!({})
        }
        fn run_window_command(&mut self, id: &str, _params: &Value) -> Result<(), Failure> {
            if id != "view.fit" {
                return Err(Failure::new(pcbcraft_engine::error_codes::ErrorCode::UnknownCommand, "unknown window command"));
            }
            self.ran.push(id.into());
            Ok(())
        }
        fn ui_command(&mut self, id: &str, _params: &Value) -> Result<Value, Failure> {
            self.ran.push(id.into());
            Ok(json!({}))
        }
    }
    let mut editor = OpenEditor::default();
    let execute = |editor: &mut OpenEditor, id, version| {
        call_editor(editor, &json!({"method": "engine.execute", "params": {"command": id, "version": version}}))
    };
    for id in ["project.new", "view.fit", "ui.inspect"] {
        assert_eq!(execute(&mut editor, id, COMMAND_API_VERSION + 1).unwrap_err().code(), "version_mismatch");
    }
    assert_eq!(execute(&mut editor, "view.nothing", COMMAND_API_VERSION + 1).unwrap_err().code(), "unknown_command");
    assert!(editor.ran.is_empty());
    assert!(editor.session.project().is_none());
    editor.blocked = true;
    for id in ["project.new", "view.fit"] {
        assert_eq!(execute(&mut editor, id, COMMAND_API_VERSION + 1).unwrap_err().code(), "unavailable");
    }
    // UI input has a separate policy and does not consult the normal editor refusal.
    assert!(execute(&mut editor, "ui.inspect", COMMAND_API_VERSION).is_ok());
    editor.blocked = false;
    assert!(execute(&mut editor, "view.fit", COMMAND_API_VERSION).is_ok());
    assert!(execute(&mut editor, "project.new", COMMAND_API_VERSION).is_ok());
    assert_eq!(editor.ran, ["ui.inspect", "view.fit"]);
}

/// Remote pins stay guarded across old peers, and metadata and warnings belong to the peer.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn remote_preserves_legacy_execution_and_never_downgrades_a_pin() {
    use crate::live::Remote;
    use craft_control::loopback::{Listener, Token};
    use std::time::Duration;

    let listener = Listener::bind(0).unwrap();
    let port = listener.port().unwrap();
    let token = Token::generate().unwrap();
    let remote = Remote::new(port, token.secret()).unwrap();
    let peer = std::thread::spawn(move || {
        let warning = json!({"code": "deprecated", "command": "project.new", "parameter": null});
        let scripts = [
            ("engine.commands", json!({"result": []})),
            ("engine.execute_versioned", json!({"error": "Unknown live control method", "code": "invalid_request"})),
            ("engine.execute", json!({"result": {"revision": 1}})),
            (
                "engine.commands",
                json!({"result": [], "command_api": {"command_api_version": 2, "capabilities": ["version_check", "deprecation_warnings"]}}),
            ),
            ("engine.execute_versioned", json!({"result": {"revision": 2}, "warnings": [warning, 7]})),
        ];
        for (method, reply) in scripts {
            let mut peer = listener.accept(Duration::from_secs(5)).unwrap();
            let request = peer.authenticate(&token).unwrap();
            assert_eq!(request["method"], method);
            if method == "engine.execute_versioned" {
                assert_eq!(request["params"]["version"], 2);
            } else {
                assert!(request["params"].get("version").is_none());
            }
            peer.send(&reply).unwrap();
        }
    });
    let mut s = Server::new(Box::new(remote));
    let legacy = call(&mut s, "list_commands", json!({}));
    assert_eq!(legacy["structuredContent"]["capabilities"], json!(["command_api", "error_codes"]));
    let args = json!({"command": "project.new", "version": 2});
    assert_eq!(call(&mut s, "execute", args.clone())["structuredContent"]["error"]["code"], "invalid_request");
    let plain = call(&mut s, "execute", json!({"command": "project.new"}));
    assert_eq!(plain, json!({"content": [{"type": "text", "text": "{\"revision\":1}"}]}));
    let modern = call(&mut s, "list_commands", json!({}));
    assert_eq!(modern["structuredContent"]["command_api_version"], 2);
    assert_eq!(modern["structuredContent"]["capabilities"], json!(["version_check", "deprecation_warnings"]));
    let warned = call(&mut s, "execute", args);
    assert_eq!(body(&warned), json!({"revision": 2}));
    let warnings = warned["structuredContent"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["command"], "project.new");
    peer.join().unwrap();
}
