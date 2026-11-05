use std::io::Cursor;

use pcbcraft_engine::{PLACE_RESISTOR, Session};
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

/// The promise of the whole project: an agent's commands do what a person's do.
#[test]
fn execute_gives_the_same_state_as_running_the_command_on_a_session() {
    let steps = [
        ("project.new", json!({"name": "blinky"})),
        ("project.rename", json!({"name": "blinky v2"})),
        (PLACE_RESISTOR, json!({"x_nm": 1270000, "y_nm": 0, "expected_revision": 2})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
    ];
    let mut s = server();
    let mut session = Session::default();
    for (command, params) in steps {
        let result = call(&mut s, "execute", json!({"command": command, "params": params}));
        assert_eq!(body(&result), session.execute(command, &params).unwrap());
    }
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), session.inspect());
}

#[test]
fn the_placement_command_exposes_integer_coordinates_to_mcp() {
    let result = call(&mut server(), "describe_command", json!({"command": PLACE_RESISTOR}));
    let schema = &body(&result)["params"];
    assert_eq!(schema["properties"]["x_nm"]["type"], "integer");
    assert_eq!(schema["properties"]["y_nm"]["type"], "integer");
    assert_eq!(schema["properties"]["expected_revision"]["minimum"], 0);
    assert_eq!(schema["required"], json!(["x_nm", "y_nm", "expected_revision"]));
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
