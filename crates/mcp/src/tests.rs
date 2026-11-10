use std::io::Cursor;

use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, PLACE_SYMBOL, Session};
use serde_json::{Value, json};

use crate::{Headless, Server, tool_definitions};

fn server() -> Server {
    Server::new(Box::new(Headless::default()))
}

fn rpc(server: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let reply = server.handle_line(&line).unwrap();
    serde_json::from_str(&reply).unwrap()
}

fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
    rpc(server, "tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
}

fn body(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// Headless and live clients share preview/apply capability validation, including
/// dedicated read-query results when the caller suppresses project inspection.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn reviewed_board_updates_share_headless_and_live_authority_rules() {
    use crate::{
        Backend,
        live::{Editor, call_editor},
    };
    use pcbcraft_engine::{APPLY_UPDATE, ASSIGN_FOOTPRINT, PREVIEW_UPDATE};
    struct OpenEditor {
        session: Session,
        blocked: bool,
    }
    impl Editor for OpenEditor {
        fn session(&mut self) -> &mut Session {
            &mut self.session
        }
        fn refusal(&self, _command: &str) -> Option<String> {
            self.blocked.then(|| "finish the dialog".into())
        }
        fn window_commands(&self) -> Vec<Value> {
            vec![]
        }
        fn inspect(&self) -> Value {
            json!({})
        }
        fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), String> {
            Err("not a window operation".into())
        }
    }
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let component = setup.inspect()["project"]["components"][0]["id"].clone();
    let definition = setup.project().unwrap().footprint_catalog().into_iter().find(|definition| definition.id == "resistor-0402").unwrap();
    setup.apply(ASSIGN_FOOTPRINT, &json!({"component":component,"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},"expected_revision":setup.revision()})).unwrap();
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut session = Session::default();
    session.open_snapshot(snapshot);
    let mut editor = OpenEditor { session, blocked: false };
    let before = headless.call("project.inspect", json!({})).unwrap();
    let revision = before["revision"].clone();
    assert_eq!(editor.session.revision(), revision.as_u64().unwrap());
    let params = json!({"command":PREVIEW_UPDATE,"params":{"expected_revision":revision},"inspect":false});
    let headless_report = headless.call("engine.execute", params.clone()).unwrap()["board_update_preview"].clone();
    let live_report = call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap()["board_update_preview"].clone();
    assert_eq!(headless_report["changes"], live_report["changes"]);
    assert_eq!(headless_report["source"]["content_digest"], live_report["source"]["content_digest"]);
    assert_ne!(headless_report["plan_token"], live_report["plan_token"]);
    assert_eq!(headless.call("project.inspect", json!({})).unwrap()["project"], before["project"]);
    let foreign = json!({"method":"engine.execute","params":{"command":APPLY_UPDATE,"params":{"plan_token":headless_report["plan_token"],"expected_revision":revision}}});
    assert!(call_editor(&mut editor, &foreign).is_err());
    let own = json!({"method":"engine.execute","params":{"command":APPLY_UPDATE,"params":{"plan_token":live_report["plan_token"],"expected_revision":revision}}});
    editor.blocked = true;
    assert!(call_editor(&mut editor, &own).unwrap_err().contains("dialog"));
    editor.blocked = false;
    let adopted = call_editor(&mut editor, &own).unwrap();
    assert_eq!(adopted["project"]["board"]["accepted_baseline"]["component_count"], 1);
    assert!(adopted["board_update_preview"].is_null());
    let adopted = headless
        .call("engine.execute", json!({"command":APPLY_UPDATE,"params":{"plan_token":headless_report["plan_token"],"expected_revision":revision}}))
        .unwrap();
    assert_eq!(adopted["project"]["board"]["accepted_baseline"]["component_count"], 1);
    assert!(
        headless
            .call(
                "engine.execute",
                json!({"command":APPLY_UPDATE,"params":{"plan_token":headless_report["plan_token"],"expected_revision":adopted["revision"]}})
            )
            .is_err()
    );
}

/// Headless clients discover Board Outline layers and create/update/delete their graphics through
/// revision-checked transactions rather than editing a rectangle settings object.
#[test]
fn board_edge_tools_share_layer_geometry_and_undo() {
    use pcbcraft_engine::{ADD_DRAWING, DELETE_DRAWING, SET_DRAWING};
    let mut s = server();
    let opened = body(&call(&mut s, "execute", json!({"command":"project.new","params":{"create_board":true}})));
    let layer = opened["project"]["boards"][0]["layers"][0]["id"].clone();
    let schema = body(&call(&mut s, "describe_command", json!({"command":ADD_DRAWING})));
    assert_eq!(schema["params"]["properties"]["layer"]["format"], "uuid");
    let points = json!([{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0},{"x_nm":10_000_000,"y_nm":5_000_000}]);
    let updated = body(&call(
        &mut s,
        "execute",
        json!({"command":ADD_DRAWING,"params":{"layer":layer,"points":points,"closed":false,"expected_revision":opened["revision"]}}),
    ));
    let drawing = updated["project"]["boards"][0]["drawings"][0]["id"].clone();
    assert_eq!(updated["project"]["boards"][0]["drawings"][0]["layer"], layer);
    let stale = call(&mut s, "execute", json!({"command":DELETE_DRAWING,"params":{"drawing":drawing,"expected_revision":opened["revision"]}}));
    assert_eq!(stale["isError"], true);
    let same = body(&call(
        &mut s,
        "execute",
        json!({"command":SET_DRAWING,"params":{"drawing":drawing,"points":points,"closed":false,"expected_revision":updated["revision"]}}),
    ));
    assert_eq!(same, updated);
    let deleted =
        body(&call(&mut s, "execute", json!({"command":DELETE_DRAWING,"params":{"drawing":drawing,"expected_revision":updated["revision"]}})));
    assert_eq!(deleted["project"]["boards"][0]["drawings"], json!([]));
    let undone = body(&call(&mut s, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"], updated["project"]);
}

#[test]
fn initialize_offers_a_version_the_client_can_speak() {
    let mut s = server();
    let reply = rpc(&mut s, "initialize", json!({"protocolVersion": "2024-11-05"}));
    assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(reply["result"]["serverInfo"]["name"], "pcbcraft");
    let reply = rpc(&mut s, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(reply["result"]["protocolVersion"], crate::PROTOCOL_VERSION);
}

#[test]
fn tools_list_names_every_tool() {
    let reply = rpc(&mut server(), "tools/list", json!({}));
    assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), tool_definitions().len());
}

/// Headless MCP discovers hierarchy commands and observes synchronized parent pins and
/// explicit electrical links through the ordinary execute/inspect protocol.
#[test]
fn hierarchy_tools_share_engine_transactions_and_endpoint_schemas() {
    use pcbcraft_engine::hierarchy::{ADD_LABEL, CREATE_CHILD, EDIT_LABEL};
    let mut s = server();
    call(&mut s, "execute", json!({"command":"project.new","params":{}}));
    let schema = body(&call(&mut s, "describe_command", json!({"command":CREATE_CHILD})));
    assert_eq!(schema["params"]["properties"]["sheet"]["format"], "uuid");
    let state = body(&call(&mut s, "execute", json!({"command":CREATE_CHILD,"params":{"name":"Child","x_nm":0,"y_nm":0,"expected_revision":1}})));
    let child = state["project"]["sheets"][1]["id"].clone();
    let state = body(&call(
        &mut s,
        "execute",
        json!({"command":ADD_LABEL,"params":{"sheet":child,"name":"DATA","direction":"input","x_nm":0,"y_nm":0,"expected_revision":state["revision"]}}),
    ));
    let label = state["project"]["sheets"][1]["schematic"]["hierarchical_labels"][0]["id"].clone();
    let parent_pin = &state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["pins"][0];
    assert_eq!(parent_pin["name"], "DATA");
    assert_eq!(parent_pin["endpoint"]["sheet_pin"], label);
    assert_eq!(state["project"]["netlist"]["nets"][0]["interfaces"].as_array().unwrap().len(), 2);
    let state = body(&call(
        &mut s,
        "execute",
        json!({"command":EDIT_LABEL,"params":{"sheet":child,"id":label,"name":"RENAMED","direction":"output","x_nm":0,"y_nm":0,"expected_revision":state["revision"]}}),
    ));
    assert_eq!(state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["pins"][0]["name"], "RENAMED");
    assert_eq!(state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["pins"][0]["direction"], "output");
    let parent = state["project"]["sheets"][0]["id"].clone();
    let block = state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["id"].clone();
    let state = body(&call(
        &mut s,
        "execute",
        json!({"command":pcbcraft_engine::hierarchy::RESIZE_SHEET,"params":{"sheet":parent,"id":block,"x_nm":0,"y_nm":0,"width_nm":30_480_000,"height_nm":20_320_000,"expected_revision":state["revision"]}}),
    ));
    assert_eq!(state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["size"], json!({"x_nm":30_480_000,"y_nm":20_320_000}));
    assert_eq!(state["project"]["sheets"][0]["schematic"]["sheet_symbols"][0]["pins"][0]["at"]["x_nm"], 30_480_000);
    let stale = call(
        &mut s,
        "execute",
        json!({"command":ADD_LABEL,"params":{"sheet":child,"name":"STALE","direction":"input","x_nm":0,"y_nm":0,"expected_revision":1}}),
    );
    assert_eq!(stale["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), state);
}

/// MCP discovers explicit sheet targets and shares the same lifecycle, isolation, physical
/// correspondence and project-wide undo behavior as GUI/engine calls.
#[test]
fn multi_sheet_tools_preserve_explicit_ownership_and_cross_probe() {
    use pcbcraft_engine::sheets::{CREATE_SHEET, DELETE_SHEET, RENAME_SHEET};
    let mut server = server();
    for (command, params) in [("project.new", json!({"create_board":true})), (CREATE_SHEET, json!({"name":"Power","expected_revision":1}))] {
        assert_ne!(call(&mut server, "execute", json!({"command":command,"params":params}))["isError"], true);
    }
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let power = before["project"]["sheets"][1]["id"].clone();
    let main = before["project"]["sheets"][0]["id"].clone();
    let schema = body(&call(&mut server, "describe_command", json!({"command":PLACE_SYMBOL})));
    assert_eq!(schema["params"]["properties"]["sheet"]["format"], "uuid");
    let ambiguous = call(&mut server, "execute", json!({"command":PLACE_RESISTOR,"params":{"x_nm":0,"y_nm":0,"expected_revision":2}}));
    assert_eq!(ambiguous["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
    for (command, params) in [
        (PLACE_RESISTOR, json!({"sheet":power,"x_nm":0,"y_nm":0,"expected_revision":2})),
        (
            pcbcraft_engine::PLACE_FOOTPRINT,
            json!({"sheet":power,"symbol":"symbol-3","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":3}),
        ),
        (RENAME_SHEET, json!({"sheet":power,"name":"Supply","expected_revision":4})),
    ] {
        assert_ne!(call(&mut server, "execute", json!({"command":command,"params":params}))["isError"], true);
    }
    let before = body(&call(&mut server, "inspect_project", json!({})));
    assert!(before["project"]["schematic"].is_null());
    assert_eq!(before["project"]["sheets"][0]["schematic"]["symbols"], json!([]));
    assert_eq!(before["project"]["sheets"][1]["schematic"]["symbols"][0]["id"], "symbol-3");
    let terminal = json!({"symbol":"symbol-3","terminal":"1"});
    let mapping = body(&call(&mut server, "cross_probe", json!({"terminal":terminal})));
    assert_eq!(mapping["pad"]["footprint"], "footprint-4");
    assert_eq!(body(&call(&mut server, "cross_probe", json!({"pad":mapping["pad"]})))["terminal"], terminal);
    let foreign = call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::MOVE_SYMBOL,"params":{"sheet":main,"symbol":"symbol-3","x_nm":1,"y_nm":0,"expected_revision":5}}),
    );
    assert_eq!(foreign["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
    assert_ne!(call(&mut server, "execute", json!({"command":DELETE_SHEET,"params":{"sheet":power,"expected_revision":5}}))["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({})))["project"]["components"], json!([]));
    call(&mut server, "execute", json!({"command":"edit.undo","params":{}}));
    assert_eq!(body(&call(&mut server, "inspect_project", json!({})))["project"], before["project"]);
}

/// The promise of the whole project: an agent's commands do what a person's do.
#[test]
fn execute_gives_the_same_state_as_running_the_command_on_a_session() {
    let steps = [
        ("project.new", json!({"name": "blinky"})),
        ("project.rename", json!({"name": "blinky v2"})),
        (PLACE_SYMBOL, json!({"definition": "resistor", "x_nm": 1270000, "y_nm": 0, "expected_revision": 2})),
        (PLACE_SYMBOL, json!({"definition": "led", "x_nm": 25400000, "y_nm": 0, "expected_revision": 3})),
        (
            CONNECT_PINS,
            json!({"from": {"symbol": "symbol-3", "terminal": "2"}, "to": {"symbol": "symbol-4", "terminal": "1"}, "expected_revision": 4}),
        ),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
    ];
    let mut s = server();
    let mut session = Session::default();
    for (command, params) in steps {
        let result = call(&mut s, "execute", json!({"command": command, "params": params}));
        assert_eq!(without_sheet_identity(body(&result)), without_sheet_identity(session.execute(command, &params).unwrap()));
    }
    assert_eq!(without_sheet_identity(body(&call(&mut s, "inspect_project", json!({})))), without_sheet_identity(session.inspect()));
}

/// Independent project.new commands generate different UUIDs. Check each sheet's path and
/// retain its other metadata while comparing commands from these separate projects.
/// Empty boards also expose a compatibility layer derived from their distinct board IDs.
fn without_sheet_identity(mut state: Value) -> Value {
    for board in state["project"]["boards"].as_array_mut().unwrap() {
        assert_eq!(board["id"].as_str().unwrap().len(), 36);
        board["id"] = Value::Null;
        assert!(board["drawings"].as_array().unwrap().is_empty());
        for layer in board["layers"].as_array_mut().unwrap() {
            assert_eq!(layer["id"].as_str().unwrap().len(), 36);
            layer["id"] = Value::Null;
        }
    }
    for sheet in state["project"]["sheets"].as_array_mut().unwrap() {
        let id = sheet["id"].as_str().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(sheet["file"], format!("sheets/{id}.json"));
        sheet["id"] = Value::Null;
        sheet["file"] = Value::Null;
    }
    state
}

/// Publish extensible native symbol identity/content parameters without changing legacy inputs.
#[test]
fn placement_commands_expose_integer_coordinates_to_mcp() {
    for command in [PLACE_RESISTOR, PLACE_SYMBOL] {
        let result = call(&mut server(), "describe_command", json!({"command": command}));
        let schema = &body(&result)["params"];
        assert_eq!(schema["properties"]["x_nm"]["type"], "integer");
        assert_eq!(schema["properties"]["y_nm"]["type"], "integer");
        assert_eq!(schema["properties"]["expected_revision"]["minimum"], 0);
        if command == PLACE_SYMBOL {
            assert_eq!(
                schema["properties"]["definition"],
                json!({
                    "type": "string",
                    "minLength":1,"maxLength":64,
                    "description": "Native symbol item ID",
                })
            );
            assert_eq!(schema["required"], json!(["definition", "x_nm", "y_nm", "expected_revision"]));
        } else {
            assert_eq!(schema["required"], json!(["x_nm", "y_nm", "expected_revision"]));
        }
    }
}

/// Source discovery and placement use the same session catalog over real MCP tool dispatch,
/// while source loading/removal leave the project's electrical revision and history alone.
#[test]
fn native_source_search_and_placement_share_the_mcp_session() {
    let mut s = server();
    let definitions = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/symbols/resistor.json"));
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/library.json"));
    let documents = json!({"library.json":manifest,"symbols/resistor.json":definitions});
    let commands = body(&call(&mut s, "list_commands", json!({"filter":"library."})));
    assert_eq!(commands.as_array().unwrap().iter().filter(|c| c["id"].as_str().is_some_and(|id| id.starts_with("library."))).count(), 4);
    let initial = body(&call(&mut s, "execute", json!({"command":"project.new","params":{}})));
    let loaded = call(&mut s, "execute", json!({"command":"library.load_source","params":{"source":"workshop-source","documents":documents}}));
    assert_ne!(loaded["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), initial);
    let results = body(&call(&mut s, "execute", json!({"command":"library.search","params":{"kind":"symbol","query":"Workshop"}})));
    let found = &results[0];
    assert_eq!(found["key"]["library"], "workshop");
    let placed = call(
        &mut s,
        "execute",
        json!({"command":PLACE_SYMBOL,"params":{
            "library":found["key"]["library"],"definition":found["key"]["id"],"definition_revision":found["key"]["revision"],"definition_digest":found["digest"],
            "x_nm":0,"y_nm":0,"expected_revision":initial["revision"]
        }}),
    );
    assert_ne!(placed["isError"], true, "{placed}");
    let placed = body(&placed);
    assert_ne!(call(&mut s, "execute", json!({"command":"library.remove_source","params":{"source":"workshop-source"}}))["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), placed);
}

/// Headless clients discover the strict setup schema and edit the same revisioned model.
#[test]
fn board_setup_commands_are_discoverable_and_atomic() {
    use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP};
    let mut s = server();
    let description = body(&call(&mut s, "describe_command", json!({"command":SET_BOARD_SETUP})));
    let schema = &description["params"]["properties"]["setup"];
    assert_eq!(schema["additionalProperties"], false);
    let material_schema = &schema["properties"]["materials"]["items"]["properties"];
    assert_eq!(material_schema["tg"]["items"]["properties"]["temperature_milli_c"]["type"], "integer");
    assert_eq!(material_schema["dielectric"]["items"]["properties"]["frequency_hz"]["type"], "integer");
    assert_eq!(schema["properties"]["stackup"]["properties"]["strata"]["items"]["oneOf"][0]["properties"]["thickness_nm"]["type"], "integer");
    let created = body(&call(&mut s, "execute", json!({"command":"project.new","params":{"create_board":true}})));
    let initialized = body(&call(
        &mut s,
        "execute",
        json!({"command":INITIALIZE_BOARD_SETUP,"params":{"copper_layers":2,"expected_revision":created["revision"]}}),
    ));
    let mut setup = initialized["project"]["board"]["setup"].clone();
    setup["stackup"]["strata"][1]["thickness_nm"] = json!(2_540_000);
    let material = json!({"id":"00000000-0000-0000-0000-000000000101","revision":1});
    setup["materials"] = json!([{"id":material["id"],"revision":1,"name":"Custom dielectric","family":"User-defined",
        "dielectric":[{"dk_millionths":4200000,"frequency_hz":1000000000,"basis":"typical","source":{"document":"Synthetic source"}}]
    }]);
    setup["stackup"]["strata"][1]["construction"] = json!({"kind":"core","material":material});
    let params = json!({"setup":setup,"expected_revision":initialized["revision"]});
    let updated = body(&call(&mut s, "execute", json!({"command":SET_BOARD_SETUP,"params":params})));
    assert_eq!(updated["project"]["board"]["setup"]["stackup"]["strata"][1]["thickness_nm"], 2_540_000);
    assert_eq!(updated["project"]["board"]["setup"]["materials"], setup["materials"]);
    let stale = call(&mut s, "execute", json!({"command":SET_BOARD_SETUP,"params":params}));
    assert_eq!(stale["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), updated);
    setup["stackup"]["strata"][1]["construction"]["material"]["revision"] = json!(2);
    let invalid = call(&mut s, "execute", json!({"command":SET_BOARD_SETUP,"params":{"setup":setup,"expected_revision":updated["revision"]}}));
    assert_eq!(invalid["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), updated);
    let undone = body(&call(&mut s, "execute", json!({"command":"edit.undo"})));
    assert_eq!(undone["project"], initialized["project"]);
}

#[test]
fn a_refused_command_is_a_tool_error_the_agent_can_read() {
    let result = call(&mut server(), "execute", json!({"command": "edit.undo"}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "nothing to undo");
}

#[test]
fn bad_arguments_are_a_protocol_error() {
    let mut s = server();
    for arguments in [json!({}), json!({"command": 3}), json!({"command": "edit.undo", "extra": 1})] {
        let reply = rpc(&mut s, "tools/call", json!({"name": "execute", "arguments": arguments}));
        assert_eq!(reply["error"]["code"], -32602, "{arguments}");
    }
    let reply = rpc(&mut s, "tools/call", json!({"name": "nope"}));
    assert_eq!(reply["error"]["code"], -32602);
}

#[test]
fn list_commands_filters() {
    let result = call(&mut server(), "list_commands", json!({"filter": "EDIT.UNDO"}));
    let all = body(&result);
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["id"], "edit.undo");
}

#[test]
fn resources_read_the_project_and_catalog() {
    let mut s = server();
    let reply = rpc(&mut s, "resources/read", json!({"uri": "pcbcraft://project"}));
    assert!(reply["result"]["contents"][0]["text"].as_str().unwrap().contains("revision"));
    let reply = rpc(&mut s, "resources/read", json!({"uri": "nope://x"}));
    assert_eq!(reply["error"]["code"], -32002);
}

#[test]
fn serve_answers_each_line_keeps_going_after_junk_and_stays_quiet_for_notifications() {
    let input = concat!(
        "not json\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "\n",
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n",
    );
    let mut output = Vec::new();
    let mut s = server();
    s.serve(Cursor::new(input), &mut output).unwrap();
    assert!(s.is_initialized());
    let lines: Vec<Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["id"], 7);
}

#[test]
fn an_oversized_line_is_refused_in_bounded_space() {
    let mut input = vec![b'x'; (16 << 20) + 1];
    input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\n");
    let mut output = Vec::new();
    server().serve(Cursor::new(input), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("request too large"));
    assert!(lines[1].contains("\"id\":8"));
}

#[test]
fn the_wiring_command_describes_exact_terminal_references() {
    let result = call(&mut server(), "describe_command", json!({"command": CONNECT_PINS}));
    let schema = &body(&result)["params"];
    assert_eq!(schema["properties"]["from"]["type"], "object");
    assert_eq!(schema["properties"]["from"]["required"], json!(["symbol", "terminal"]));
    assert_eq!(schema["properties"]["to"]["properties"]["terminal"]["pattern"], json!("^[A-Za-z0-9+_-]{1,8}$"));
}

/// Bulk command callers may skip inspection and fetch the project separately.
#[test]
fn execute_can_return_only_the_new_revision() {
    let mut server = server();
    let result = call(&mut server, "execute", json!({"command":"project.new", "params":{"name":"Lean"}, "inspect":false}));
    assert_eq!(body(&result), json!({"revision":1}));
    assert_eq!(body(&call(&mut server, "inspect_project", json!({})))["project"]["name"], "Lean");
}

/// Cross-probing follows the saved pin mapping in either direction.
#[test]
fn cross_probe_follows_the_explicit_pin_to_pad_mapping() {
    let mut server = server();
    for (command, params) in [
        ("project.new", json!({"name":"Links","create_board":true})),
        (PLACE_SYMBOL, json!({"definition":"led", "x_nm":0, "y_nm":0, "expected_revision":1})),
        (
            pcbcraft_engine::PLACE_FOOTPRINT,
            json!({
                "symbol":"symbol-2", "definition":"generic-through-hole-2", "pin_1_pad":2, "x_nm":0, "y_nm":0, "expected_revision":2,
            }),
        ),
    ] {
        let result = call(&mut server, "execute", json!({"command":command, "params":params, "inspect":false}));
        assert_ne!(result["isError"], true);
    }
    let terminal = json!({"symbol":"symbol-2", "terminal":"1"});
    let inspected = body(&call(&mut server, "inspect_project", json!({})));
    let board = inspected["project"]["boards"][0]["id"].clone();
    let pad = json!({"board":board,"footprint":"footprint-3", "pad":"2"});
    assert_eq!(body(&call(&mut server, "cross_probe", json!({"terminal":terminal}))), json!({"pad":pad}));
    assert_eq!(body(&call(&mut server, "cross_probe", json!({"pad":pad}))), json!({"terminal":terminal}));
    assert_eq!(body(&call(&mut server, "cross_probe", json!({"pad":{"footprint":"footprint-3","pad":"2"}}))), json!({"terminal":terminal}));
    let bad = call(&mut server, "cross_probe", json!({"pad":{"board":"00000000-0000-0000-0000-000000000001","footprint":"footprint-3","pad":"2"}}));
    assert_eq!(bad["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), inspected);
}

/// MCP exposes rotated placement and authored routes to all library pins, including pin 4;
/// the resulting route can be deleted through the shared catalog.
#[test]
fn mcp_draws_free_routes_to_multi_pin_symbols() {
    let mut server = server();
    let command = body(&call(&mut server, "describe_command", json!({"command":pcbcraft_engine::DRAW_WIRE})));
    assert_eq!(command["params"]["properties"]["from"]["oneOf"][0]["properties"]["terminal"]["pattern"], json!("^[A-Za-z0-9+_-]{1,8}$"));
    body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Free to header"}})));
    let placed = body(&call(
        &mut server,
        "execute",
        json!({"command":PLACE_SYMBOL,"params":{
            "definition":"header-4","x_nm":0,"y_nm":0,"rotation":90,"expected_revision":1
        }}),
    ));
    assert_eq!(placed["project"]["schematic"]["symbols"][0]["rotation"], 90);
    let pin = &placed["project"]["schematic"]["symbols"][0]["pins"][3];
    let state = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,"params":{
            "from":{"x_nm":pin["x_nm"],"y_nm":12_700_000},
            "to":{"symbol":"symbol-2","terminal":"4"}, "bends":[],"expected_revision":placed["revision"]
        }}),
    ));
    assert_eq!(state["project"]["schematic"]["netlist"]["nets"][0]["terminals"], json!([{"symbol":"symbol-2","terminal":"4"}]));
    let wire = &state["project"]["schematic"]["wires"][0]["id"];
    let deleted = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DELETE_WIRE,"params":{
            "wire":wire,"expected_revision":state["revision"]
        }}),
    ));
    assert_eq!(deleted["project"]["schematic"]["wires"], json!([]));
    assert_eq!(deleted["project"]["schematic"]["junctions"], json!([]));
}

/// MCP discovers and executes a segment slide through the shared engine, with the same geometry,
/// history and revision rejection as the canvas. No UI-specific edit path is required.
#[test]
fn mcp_slides_wire_segments_and_rejects_stale_retries() {
    let mut server = server();
    let command = pcbcraft_engine::MOVE_WIRE_SEGMENT;
    let description = body(&call(&mut server, "describe_command", json!({"command":command})));
    assert_eq!(description["params"]["required"], json!(["wire", "at", "to", "expected_revision"]));
    body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Slide"}})));
    let point = |x: i64, y: i64| json!({"x_nm":x * 1_270_000,"y_nm":y * 1_270_000});
    let drawn = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,
        "params":{"from":point(0,0),"to":point(8,6),"bends":[point(8,0)],"expected_revision":1}}),
    ));
    let wire = &drawn["project"]["schematic"]["wires"][0]["id"];
    let request = json!({"command":command,"params":{"wire":wire,"at":point(8,3),"to":point(10,3),"expected_revision":drawn["revision"]}});
    let moved = body(&call(&mut server, "execute", request.clone()));
    let route = &moved["project"]["schematic"]["wires"][0];
    assert_eq!(route["bends"], json!([point(10, 0), point(10, 6)]));
    assert_eq!(route["from"], drawn["project"]["schematic"]["wires"][0]["from"]);
    assert_eq!(route["to"], drawn["project"]["schematic"]["wires"][0]["to"]);
    assert_eq!(moved["project"]["schematic"]["netlist"], drawn["project"]["schematic"]["netlist"]);
    let stale = call(&mut server, "execute", request);
    assert_eq!(stale["isError"], true);
    let inspected = body(&call(&mut server, "inspect_project", json!({})));
    assert_eq!(inspected, moved);
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"], drawn["project"]);
}

/// Headless MCP discovers the strict attachment schema and connects explicit endpoints through
/// the shared command; stale retries fail without affecting the one-step undo history.
#[test]
fn mcp_attaches_junctions_and_rejects_stale_retries() {
    let mut server = server();
    let command = pcbcraft_engine::ATTACH_JUNCTION;
    let description = body(&call(&mut server, "describe_command", json!({"command":command})));
    assert_eq!(description["params"]["required"], json!(["junction", "to", "expected_revision"]));
    assert_eq!(description["params"]["properties"]["to"]["oneOf"].as_array().unwrap().len(), 6);
    let mut state = body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Attach"}})));
    let point = |x: i64, y: i64| json!({"x_nm":x * 1_270_000,"y_nm":y * 1_270_000});
    for y in [0, 6] {
        state = body(&call(
            &mut server,
            "execute",
            json!({"command":pcbcraft_engine::DRAW_WIRE,
            "params":{"from":point(0,y),"to":point(8,y),"bends":[],"expected_revision":state["revision"]}}),
        ));
    }
    let wires = &state["project"]["schematic"]["wires"];
    let request =
        json!({"command":command,"params":{"junction":wires[0]["to"]["junction"],"to":wires[1]["to"],"expected_revision":state["revision"]}});
    let attached = body(&call(&mut server, "execute", request.clone()));
    let schematic = &attached["project"]["schematic"];
    assert_eq!(schematic["netlist"]["nets"].as_array().unwrap().len(), 1);
    let after = schematic["wires"].as_array().unwrap();
    assert!(after.iter().any(|wire| wire["from"] == wires[1]["to"] || wire["to"] == wires[1]["to"]));
    assert!(after.iter().all(|wire| wire["from"] != wires[0]["to"] && wire["to"] != wires[0]["to"]));
    assert_eq!(call(&mut server, "execute", request)["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), attached);
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"], state["project"]);
}

/// MCP discovers the default-on boolean, preserves a loop when it is explicitly false, and
/// can later heal that selected route through the same revision-checked command as the GUI.
#[test]
fn mcp_can_disable_automatic_healing_and_heal_later() {
    let mut server = server();
    let description = body(&call(&mut server, "describe_command", json!({"command":pcbcraft_engine::DRAW_WIRE})));
    assert_eq!(description["params"]["properties"]["auto_heal"]["type"], "boolean");
    assert_eq!(description["params"]["properties"]["auto_heal"]["default"], true);
    let start = body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Loop"}})));
    let p = |x: i64, y: i64| json!({"x_nm":x*1_270_000,"y_nm":y*1_270_000});
    let mut params = json!({"from":p(-10,0),"to":p(20,12),"bends":[p(6,0),p(6,12),p(-4,12),p(-4,6),p(6,6),p(6,12)],"auto_heal":"false","expected_revision":start["revision"]});
    assert_eq!(call(&mut server, "execute", json!({"command":pcbcraft_engine::DRAW_WIRE,"params":params}))["isError"], true);
    params["auto_heal"] = json!(false);
    let looped = body(&call(&mut server, "execute", json!({"command":pcbcraft_engine::DRAW_WIRE,"params":params})));
    let wire = &looped["project"]["schematic"]["wires"][0];
    assert_eq!(wire["bends"].as_array().unwrap().len(), 6);
    let healed = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::HEAL_WIRES,"params":{"items":{"symbols":[],"wires":[wire["id"]]},"expected_revision":looped["revision"]}}),
    ));
    assert_eq!(healed["project"]["schematic"]["wires"][0]["bends"], json!([p(6, 0), p(6, 12)]));
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"], looped["project"]);
}

/// MCP discovers the wire slice, break and segment deletion through the shared catalog and runs
/// them: a slice keeps the route connected through a new junction, and deleting a leg leaves
/// free ends.
#[test]
fn mcp_slices_wires_and_deletes_segments() {
    let mut server = server();
    for command in [pcbcraft_engine::SLICE_WIRE, pcbcraft_engine::BREAK_WIRE, pcbcraft_engine::DELETE_WIRE_SEGMENT] {
        let described = body(&call(&mut server, "describe_command", json!({"command":command})));
        assert_eq!(described["params"]["required"], json!(["wire", "at", "expected_revision"]));
    }
    body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Cut"}})));
    let point = |x: i64, y: i64| json!({"x_nm":x * 1_270_000,"y_nm":y * 1_270_000});
    let drawn = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,"params":{"from":point(0, 0),"to":point(8, 0),"bends":[],"expected_revision":1}}),
    ));
    let wire = drawn["project"]["schematic"]["wires"][0]["id"].clone();
    let broken = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::SLICE_WIRE,"params":{"wire":wire,"at":point(4, 0),"expected_revision":drawn["revision"]}}),
    ));
    let wires = &broken["project"]["schematic"]["wires"];
    assert_eq!(wires.as_array().unwrap().len(), 2);
    assert_eq!(wires[0]["to"], wires[1]["from"]);
    let nets = |state: &Value| state["project"]["schematic"]["netlist"]["nets"].as_array().unwrap().len();
    assert_eq!(nets(&broken), nets(&drawn), "a slice keeps one net");
    let cut = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DELETE_WIRE_SEGMENT,"params":{"wire":wires[1]["id"],"at":point(6, 0),"expected_revision":broken["revision"]}}),
    ));
    assert_eq!(cut["project"]["schematic"]["wires"].as_array().unwrap().len(), 1);
    let refused = call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DELETE_WIRE_SEGMENT,"params":{"wire":wire,"at":point(0, 0),"expected_revision":cut["revision"]}}),
    );
    assert_eq!(refused["isError"], true);
}

/// Agents reshape wires with the same three commands the canvas drags use: slide a segment,
/// move a corner and move a free end. Each keeps the wire's endpoints and its net.
#[test]
fn mcp_reshapes_wire_segments_corners_and_ends() {
    let mut server = server();
    for (command, required) in [
        (pcbcraft_engine::MOVE_WIRE_SEGMENT, json!(["wire", "at", "to", "expected_revision"])),
        (pcbcraft_engine::MOVE_WIRE_CORNER, json!(["wire", "at", "to", "expected_revision"])),
        (pcbcraft_engine::MOVE_JUNCTION, json!(["junction", "to", "expected_revision"])),
    ] {
        let described = body(&call(&mut server, "describe_command", json!({"command":command})));
        assert_eq!(described["params"]["required"], required, "{command}");
    }
    body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Shape"}})));
    let point = |x: i64, y: i64| json!({"x_nm":x * 1_270_000,"y_nm":y * 1_270_000});
    let drawn = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,"params":{"from":point(0, 0),"to":point(8, 6),"bends":[point(8, 0)],"expected_revision":1}}),
    ));
    let wire = drawn["project"]["schematic"]["wires"][0].clone();
    let slid = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::MOVE_WIRE_SEGMENT,"params":{"wire":wire["id"],"at":point(8, 3),"to":point(10, 3),"expected_revision":drawn["revision"]}}),
    ));
    let after = &slid["project"]["schematic"]["wires"][0];
    assert_eq!((&after["from"], &after["to"]), (&wire["from"], &wire["to"]));
    assert_eq!(after["bends"], json!([point(10, 0), point(10, 6)]));
    let cornered = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::MOVE_WIRE_CORNER,"params":{"wire":wire["id"],"at":point(10, 0),"to":point(9, 0),"expected_revision":slid["revision"]}}),
    ));
    assert_eq!(cornered["project"]["schematic"]["wires"][0]["bends"], json!([point(9, 0), point(9, 6)]));
    let moved = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::MOVE_JUNCTION,"params":{"junction":wire["to"]["junction"],"to":point(12, 6),"expected_revision":cornered["revision"]}}),
    ));
    assert_eq!(moved["project"]["schematic"]["wires"][0]["bends"], json!([point(9, 0), point(9, 6)]));
    assert_eq!(moved["project"]["schematic"]["netlist"], drawn["project"]["schematic"]["netlist"]);
    let refused = call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::MOVE_WIRE_SEGMENT,"params":{"wire":wire["id"],"at":point(9, 3),"to":point(11, 4),"expected_revision":moved["revision"]}}),
    );
    assert_eq!(refused["isError"], true);
}

/// MCP publishes segment targets and explicit healing, using the same graph changes and
/// undo history as the ribbon; a stale retry cannot reapply cleanup.
#[test]
fn mcp_branches_into_wire_segments_and_heals_selection() {
    let mut server = server();
    let command = pcbcraft_engine::HEAL_WIRES;
    let description = body(&call(&mut server, "describe_command", json!({"command":command})));
    assert_eq!(description["params"]["required"], json!(["items", "expected_revision"]));
    let mut state = body(&call(&mut server, "execute", json!({"command":"project.new","params":{"name":"Branch"}})));
    let point = |x: i64, y: i64| json!({"x_nm":x*1_270_000,"y_nm":y*1_270_000});
    state = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,
        "params":{"from":point(0,0),"to":point(20,0),"bends":[],"expected_revision":state["revision"]}}),
    ));
    let wire = state["project"]["schematic"]["wires"][0]["id"].clone();
    let before = state["project"].clone();
    state = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::DRAW_WIRE,
        "params":{"from":point(10,10),"to":{"wire":wire,"at":point(10,0)},"bends":[],"expected_revision":state["revision"]}}),
    ));
    assert_eq!(state["project"]["schematic"]["wires"].as_array().unwrap().len(), 3);
    let request = json!({"command":command,"params":{"items":{"symbols":[],"wires":[wire]},"expected_revision":state["revision"]}});
    let healed = body(&call(&mut server, "execute", request.clone()));
    assert_eq!(healed, state, "this branch is already normalized");
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"], before);
    assert_eq!(call(&mut server, "execute", request)["isError"], true);
}

/// A session without a window refuses window commands and editor inspection with a reason,
/// instead of silently succeeding, and leaves the project untouched.
#[test]
fn headless_sessions_say_window_commands_need_a_live_editor() {
    let mut server = server();
    body(&call(&mut server, "execute", json!({"command": "project.new"})));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    for (tool, arguments) in [
        ("execute", json!({"command": "view.split", "params": {"view": 0, "split": true}})),
        ("execute", json!({"command": "app.set_preferences", "params": {"theme": "light"}})),
        ("describe_command", json!({"command": "window.set_panel"})),
        ("inspect_editor", json!({})),
    ] {
        let refused = call(&mut server, tool, arguments);
        assert_eq!(refused["isError"], true, "{tool}");
        assert!(refused["content"][0]["text"].as_str().unwrap().contains("Connect AI assistants"), "{refused}");
    }
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
    let unknown = call(&mut server, "describe_command", json!({"command": "schematic.nothing"}));
    assert_eq!(unknown["content"][0]["text"], "unknown command `schematic.nothing`");
    let listed = body(&call(&mut server, "list_commands", json!({})));
    assert!(listed.as_array().unwrap().iter().all(|command| !crate::is_window_command(command["id"].as_str().unwrap())));
}

/// A token file is read trimmed and bounded.
#[test]
fn a_token_file_is_read_trimmed_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("pcbcraft-mcp-token-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let good = dir.join("good.token");
    std::fs::write(&good, "0123456789abcdef0123456789abcdef\n")?;
    assert!(crate::live::Remote::from_token_file(41234, &good).is_ok());
    let huge = dir.join("huge.token");
    std::fs::write(&huge, "a".repeat(5000))?;
    let error = crate::live::Remote::from_token_file(41234, &huge).err().ok_or("an oversized file was accepted")?;
    assert!(error.contains("too large") && !error.contains("aaaa"), "{error}");
    let junk = dir.join("junk.token");
    std::fs::write(&junk, "not a token")?;
    let error = crate::live::Remote::from_token_file(41234, &junk).err().ok_or("junk was accepted")?;
    assert!(!error.contains("not a token"), "the file's text stays out of errors: {error}");
    assert!(crate::live::Remote::from_token_file(41234, &dir.join("missing.token")).is_err());
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// Headless clients discover exact package metadata, assign without copper and place later
/// through the same revision-checked transactions, preserving explicit swapped mappings.
#[test]
fn footprint_assignment_is_discoverable_and_separate_from_placement() {
    use pcbcraft_engine::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT};
    let mut s = server();
    call(&mut s, "execute", json!({"command":"project.new","params":{"create_board":true}}));
    call(&mut s, "execute", json!({"command":PLACE_RESISTOR,"params":{"x_nm":0,"y_nm":0,"expected_revision":1}}));
    let before = body(&call(&mut s, "inspect_project", json!({})));
    let component = before["project"]["components"][0]["id"].clone();
    let package = before["footprint_catalog"].as_array().unwrap().iter().find(|p| p["id"] == "resistor-0402").unwrap();
    let assigned = call(
        &mut s,
        "execute",
        json!({"command":ASSIGN_FOOTPRINT,"params":{
        "component":component,"library":package["library"],"definition":package["id"],"definition_revision":package["revision"],
        "mapping":{"1":"2","2":"1"},"expected_revision":2}}),
    );
    assert_ne!(assigned["isError"], true);
    assert_eq!(body(&assigned)["project"]["board"]["footprints"], json!([]));
    let placed = call(
        &mut s,
        "execute",
        json!({"command":PLACE_ASSIGNED_FOOTPRINT,"params":{
        "component":component,"x_nm":0,"y_nm":0,"expected_revision":3}}),
    );
    assert_ne!(placed["isError"], true);
    assert_eq!(body(&placed)["project"]["board"]["footprints"][0]["mapping"], json!({"1":"2","2":"1"}));
    let stale = call(
        &mut s,
        "execute",
        json!({"command":PLACE_ASSIGNED_FOOTPRINT,"params":{
        "component":component,"x_nm":1,"y_nm":0,"expected_revision":3}}),
    );
    assert_eq!(stale["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), body(&placed));
}

/// Headless clients start schematic-only and create the same addressed board as the GUI.
#[test]
fn optional_board_creation_is_discoverable_revision_checked_and_undoable() {
    let mut server = server();
    let initial = body(&call(&mut server, "execute", json!({"command":"project.new","params":{}})));
    assert_eq!(initial["project"]["boards"], json!([]));
    let schema = body(&call(&mut server, "describe_command", json!({"command":"pcb.create_board"})));
    assert_eq!(schema["params"]["properties"]["expected_revision"]["type"], "integer");
    let stale = call(&mut server, "execute", json!({"command":"pcb.create_board","params":{"expected_revision":0}}));
    assert_eq!(stale["isError"], true);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), initial);
    let created = body(&call(
        &mut server,
        "execute",
        json!({
            "command":"pcb.create_board", "params":{"name":"Controller","expected_revision":initial["revision"]}
        }),
    ));
    assert_eq!(created["project"]["boards"][0]["name"], "Controller");
    let board = created["project"]["boards"][0]["id"].clone();
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"]["boards"], json!([]));
    let redone = body(&call(&mut server, "execute", json!({"command":"edit.redo","params":{}})));
    assert_eq!(redone["project"]["boards"][0]["id"], board);
}

/// Opt-in accepted history retains the physical object through the same headless commands.
#[test]
fn accepted_baseline_retains_deleted_source_footprints_through_mcp() {
    let mut server = server();
    for command in [pcbcraft_engine::DELETE_SYMBOL, pcbcraft_engine::DELETE_ITEMS, pcbcraft_engine::sheets::DELETE_SHEET] {
        let described = body(&call(&mut server, "describe_command", json!({"command":command})));
        let summary = described["summary"].as_str().unwrap();
        assert!(summary.contains("accepted baseline") && summary.contains("pending review") && summary.contains("otherwise"), "{described}");
    }
    let execute = |server: &mut Server, command: &str, params: Value| {
        let result = call(server, "execute", json!({"command":command,"params":params}));
        assert_ne!(result["isError"], true, "{result}");
        body(&result)
    };
    execute(&mut server, "project.new", json!({"create_board":true}));
    let state = execute(&mut server, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    let component = state["project"]["components"][0]["id"].clone();
    let symbol = state["project"]["schematic"]["symbols"][0]["id"].clone();
    let package = state["footprint_catalog"].as_array().unwrap().iter().find(|package| package["id"] == "resistor-0402").unwrap();
    let assigned = execute(
        &mut server,
        pcbcraft_engine::ASSIGN_FOOTPRINT,
        json!({
            "component":component,"library":package["library"],"definition":package["id"],
            "definition_revision":package["revision"],"mapping":{"1":"1","2":"2"},"expected_revision":state["revision"]
        }),
    );
    let placed = execute(
        &mut server,
        pcbcraft_engine::PLACE_ASSIGNED_FOOTPRINT,
        json!({
            "component":component,"x_nm":0,"y_nm":0,"expected_revision":assigned["revision"]
        }),
    );
    let initialized = execute(&mut server, pcbcraft_engine::INITIALIZE_UPDATE_BASELINE, json!({"expected_revision":placed["revision"]}));
    let removed = execute(&mut server, pcbcraft_engine::DELETE_SYMBOL, json!({"symbol":symbol,"expected_revision":initialized["revision"]}));
    assert_eq!(removed["project"]["components"], json!([]));
    assert_eq!(removed["project"]["board"]["footprints"][0]["id"], placed["project"]["board"]["footprints"][0]["id"]);
    assert_eq!(removed["project"]["board"]["footprints"][0]["pending_removal"], true);
    assert_eq!(removed["project"]["board"]["pending_removals"], json!([component]));
    let restored = execute(&mut server, "edit.undo", json!({}));
    assert_eq!(restored["project"]["components"], placed["project"]["components"]);
    assert_eq!(restored["project"]["board"]["footprints"][0]["pending_removal"], false);
}

/// Headless clients discover explicit populated-board binding adoption and placement-only
/// deletion, using the same revision gates and undo history as the graphical controls.
#[test]
fn populated_setup_and_footprint_deletion_are_discoverable_shared_commands() {
    use pcbcraft_engine::{DELETE_FOOTPRINTS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT};
    let mut s = server();
    let execute = |s: &mut Server, command: &str, params: Value| body(&call(s, "execute", json!({"command":command,"params":params})));
    execute(&mut s, "project.new", json!({"create_board":true}));
    let placed_symbol = execute(&mut s, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    let symbol = placed_symbol["project"]["schematic"]["symbols"][0]["id"].clone();
    let original = execute(
        &mut s,
        PLACE_FOOTPRINT,
        json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":0,"y_nm":0,"expected_revision":placed_symbol["revision"]}),
    );
    let schema = body(&call(&mut s, "describe_command", json!({"command":INITIALIZE_BOARD_SETUP})));
    assert_eq!(schema["params"]["properties"]["resolve_existing_footprints"]["type"], "boolean");
    let board = original["project"]["board"]["id"].clone();
    let configured = execute(
        &mut s,
        INITIALIZE_BOARD_SETUP,
        json!({"board":board,"copper_layers":4,"resolve_existing_footprints":true,"expected_revision":original["revision"]}),
    );
    assert!(configured["project"]["board"]["setup"].is_object());
    let schema = body(&call(&mut s, "describe_command", json!({"command":DELETE_FOOTPRINTS})));
    assert_eq!(schema["params"]["properties"]["footprints"]["uniqueItems"], true);
    let footprint = configured["project"]["board"]["footprints"][0]["id"].clone();
    let removed = execute(&mut s, DELETE_FOOTPRINTS, json!({"board":board,"footprints":[footprint],"expected_revision":configured["revision"]}));
    assert_eq!(removed["project"]["board"]["footprints"], json!([]));
    assert_eq!(removed["project"]["board"]["assignments"][0]["placed"], false);
    assert_eq!(removed["project"]["schematic"], original["project"]["schematic"]);
    assert_eq!(execute(&mut s, "edit.undo", json!({}))["project"], configured["project"]);
}

/// The shared dispatcher retains clipboard payloads with inspect:false, while paste remains
/// a single ordinary revision-checked edit.
#[test]
fn clipboard_payloads_survive_suppressed_project_inspection() {
    use pcbcraft_engine::clipboard::{COPY, CUT, PASTE};
    let mut server = server();
    let created = body(&call(&mut server, "execute", json!({"command":"project.new","params":{}})));
    let placed =
        body(&call(&mut server, "execute", json!({"command":PLACE_RESISTOR,"params":{"x_nm":0,"y_nm":0,"expected_revision":created["revision"]}})));
    let symbol = placed["project"]["schematic"]["symbols"][0]["id"].clone();
    let selection = json!({"symbols":[symbol],"wires":[]});
    for command in [COPY, CUT] {
        let result = body(&call(
            &mut server,
            "execute",
            json!({"command":command,"inspect":false,"params":{"items":selection,"expected_revision":placed["revision"]}}),
        ));
        assert!(result["clipboard"].as_str().is_some_and(|s| pcbcraft_engine::clipboard::Fragment::from_text(s).is_ok()));
        if command == COPY {
            assert_eq!(result["revision"], placed["revision"]);
        } else {
            let pasted = body(&call(
                &mut server,
                "execute",
                json!({"command":PASTE,"inspect":false,"params":{"text":result["clipboard"],"x_nm":12_700_000,"y_nm":0,"expected_revision":result["revision"]}}),
            ));
            assert!(pasted.get("project").is_none());
        }
    }
    let state = body(&call(&mut server, "inspect_project", json!({})));
    assert_eq!(state["project"]["schematic"]["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(state["project"]["schematic"]["symbols"][0]["x_nm"], 12_700_000);
}

/// Suppressing project inspection still returns the DRC report without changing the project,
/// alongside the clipboard payload exceptions in the shared dispatcher.
#[test]
fn design_check_report_survives_suppressed_project_inspection() {
    let mut server = server();
    let created = body(&call(&mut server, "execute", json!({"command":"project.new","params":{"create_board":true}})));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let response = call(&mut server, "execute", json!({"command":pcbcraft_engine::CHECK_DESIGN,"params":{"clearance_nm":250_000},"inspect":false}));
    assert_ne!(response["isError"], true, "{response}");
    let result = body(&response);
    assert_eq!(result["revision"], created["revision"]);
    assert_eq!(result["design_check"]["rules"]["clearance_nm"], 250_000);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
}

/// Fabrication files survive inspect:false through the shared dispatcher without changing the
/// project, revision or history, alongside clipboard preparation and DRC report results.
#[test]
fn fabrication_files_survive_suppressed_project_inspection() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":setup.revision()})).unwrap();
    let project = setup.project().unwrap();
    let board = project.board_target().unwrap();
    let layer = project
        .board_view(board)
        .unwrap()
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline)
        .unwrap()
        .id;
    setup.apply(pcbcraft_engine::ADD_DRAWING, &json!({"board":board.board,"layer":layer,"closed":true,"points":[{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0},{"x_nm":10_000_000,"y_nm":10_000_000},{"x_nm":0,"y_nm":10_000_000}],"expected_revision":setup.revision()})).unwrap();
    let mut server = Server::new(Box::new(Headless::with_project(setup.snapshot().unwrap())));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let response =
        call(&mut server, "execute", json!({"command":pcbcraft_engine::EXPORT_FABRICATION,"params":{"board":board.board},"inspect":false}));
    assert_ne!(response["isError"], true, "{response}");
    let result = body(&response);
    let files = result["fabrication"]["files"].as_array().unwrap();
    assert!(files.iter().any(|file| file["name"].as_str().unwrap().ends_with("-Outline.gbr")));
    assert!(files.iter().any(|file| file["name"].as_str().unwrap().ends_with("-PTH.drl")));
    assert_eq!(result["revision"], before["revision"]);
    assert!(result.get("project").is_none());
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
}

/// Board Copy/Cut return native payloads with inspect:false, and a same-project cut can relocate
/// its existing logical component through the ordinary shared paste transaction.
#[test]
fn pcb_clipboard_payloads_and_component_reuse_share_the_dispatcher() {
    use pcbcraft_engine::board::clipboard::{BoardFragment, COPY, CUT, PASTE};
    let mut server = server();
    let initial = body(&call(&mut server, "execute", json!({"command":"project.new","params":{"create_board":true}})));
    let symbol =
        body(&call(&mut server, "execute", json!({"command":PLACE_RESISTOR,"params":{"x_nm":0,"y_nm":0,"expected_revision":initial["revision"]}})));
    let symbol_id = symbol["project"]["schematic"]["symbols"][0]["id"].clone();
    let placed = body(&call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::PLACE_FOOTPRINT,"params":{"symbol":symbol_id,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":symbol["revision"]}}),
    ));
    let footprint = placed["project"]["board"]["footprints"][0]["id"].clone();
    let items = json!({"footprints":[footprint]});
    for command in [COPY, CUT] {
        let copied = body(&call(
            &mut server,
            "execute",
            json!({"command":command,"inspect":false,"params":{"items":items,"expected_revision":placed["revision"]}}),
        ));
        assert!(BoardFragment::from_text(copied["clipboard"].as_str().unwrap()).is_ok());
        if command == COPY {
            assert_eq!(copied["revision"], placed["revision"]);
        } else {
            let pasted = call(
                &mut server,
                "execute",
                json!({"command":PASTE,"inspect":false,"params":{"text":copied["clipboard"],"reuse_components":true,"x_nm":7_000_000,"y_nm":3_000_000,"expected_revision":copied["revision"]}}),
            );
            assert_ne!(pasted["isError"], true, "{pasted}");
        }
    }
    let state = body(&call(&mut server, "inspect_project", json!({})));
    assert_eq!(state["project"]["schematic"]["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(state["project"]["board"]["footprints"].as_array().unwrap().len(), 1);
    assert_eq!(state["project"]["board"]["footprints"][0]["x_nm"], 7_000_000);
}

/// Text preparation returns the portable payload when inspection is suppressed; a client can
/// review/relay it without editing a project or invoking the OS clipboard.
#[test]
fn external_clipboard_preparation_preserves_payload_without_inspection() {
    let mut session = pcbcraft_engine::Session::default();
    for (id, params) in [
        (pcbcraft_engine::clipboard_text::LABELS, json!({"text":"A\nB","style":"local","spacing_nm":2_540_000,"expected_revision":0})),
        (pcbcraft_engine::clipboard_text::OUTLINE, json!({"text":"0,0\n10,0\n10,10","units":"mm","closed":true,"expected_revision":0})),
    ] {
        let result = crate::backend::call_session(&mut session, "engine.execute", &json!({"command":id,"params":params,"inspect":false})).unwrap();
        assert!(result["clipboard"].as_str().is_some());
        assert_eq!(session.revision(), 0);
        assert!(session.project().is_none());
    }
}

/// Extended hierarchy payloads and Cut relocation use the same public dispatcher and retain
/// clipboard results when callers suppress unrelated project inspection.
#[test]
fn hierarchical_clipboard_commands_preserve_payload_and_child_identity() {
    let mut s = pcbcraft_engine::Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(pcbcraft_engine::hierarchy::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    let root = p.sheet_target();
    let child = p.sheets().find(|p| p.name == "Child").unwrap().target;
    let block = p.sheet_symbols()[0].id;
    let cut = json!({"command":pcbcraft_engine::clipboard::CUT,"inspect":false,"params":{"sheet":root.id(),"items":{"symbols":[],"wires":[]},"objects":[format!("sheet-symbol-{block}")],"expected_revision":s.revision()}});
    let data = crate::backend::call_session(&mut s, "engine.execute", &cut).unwrap();
    assert!(data["clipboard"].as_str().unwrap().contains("pcbcraft-clipboard/2"));
    assert_eq!(s.project().unwrap().parent_sheet(child), None);
    let paste = json!({"command":pcbcraft_engine::clipboard::PASTE,"inspect":false,"params":{"sheet":root.id(),"text":data["clipboard"],"reuse_sheets":true,"x_nm":1_270_000,"y_nm":1_270_000,"expected_revision":s.revision()}});
    crate::backend::call_session(&mut s, "engine.execute", &paste).unwrap();
    assert_eq!(s.project().unwrap().parent_sheet(child), Some(root));
    assert_eq!(s.project().unwrap().sheets().len(), 2);
}
