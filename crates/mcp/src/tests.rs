#[cfg(not(target_arch = "wasm32"))]
mod arcs;

use std::io::Cursor;

mod command_api;
#[cfg(not(target_arch = "wasm32"))]
mod library_updates;

use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, PLACE_SYMBOL, Session};
use serde_json::{Value, json};

use crate::{Headless, Server, tool_definitions};

/// Focused headless pin-remap queries retain the review payload even with inspect:false;
/// preview does not need an open project and never creates history or saves the candidate.
#[test]
fn pin_remap_mcp_preserves_the_preview_without_general_inspection() {
    let mut server = Server::new(Box::new(Headless::default()));
    let item = pcbcraft_engine::authoring::Item::new(pcbcraft_engine::authoring::Kind::Symbol).unwrap();
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let result = call(
        &mut server,
        "execute",
        json!({
            "command":pcbcraft_engine::library::sources::PREVIEW_PIN_REMAP,"inspect":false,
            "params":{"item":item,"pins":[{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]}
        }),
    );
    assert_ne!(result["isError"], true, "{result}");
    let report = body(&result);
    assert_eq!(report["changes"][0]["before"], "1");
    assert_eq!(report["changes"][0]["after"], "2");
    assert_eq!(report["changes"][0]["object"], "1");
    assert_eq!(report["source_revision"], Value::Null);
    assert!(report["item"]["revision"].is_number());
    assert_eq!(report["item"]["objects"]["1"]["number"], "2");
    assert_eq!(report["item"]["objects"]["3"]["number"], "1");
    assert_eq!(report["saved"], false);
    assert_eq!(report["instances"], json!([]));
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
}

/// Raw remap items must preserve duplicate-key validation for single requests and batches.
#[test]
fn pin_remap_wire_refuses_duplicate_primitive_ids() {
    let item = pcbcraft_engine::authoring::Item::new(pcbcraft_engine::authoring::Kind::Symbol).unwrap();
    let item = serde_json::to_value(item).unwrap();
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{
        "command":pcbcraft_engine::library::sources::PREVIEW_PIN_REMAP,"params":{"item":item,"pins":[{"unit":"main","object":"1","number":"7"}]}
    }}})
    .to_string();
    let duplicate = request.replacen("\"objects\":{", &format!("\"objects\":{{\"1\":{},", item["objects"]["1"]), 1);
    let mut server = Server::new(Box::new(Headless::default()));
    let reply = |server: &mut Server, line: &str| serde_json::from_str::<Value>(&server.handle_line(line).unwrap()).unwrap();
    let valid = reply(&mut server, &request);
    assert!(valid["result"]["content"][0]["text"].is_string(), "{valid}");
    assert_ne!(valid["result"]["isError"], true, "{valid}");
    let refused = reply(&mut server, &duplicate);
    assert_eq!(refused["id"], 1);
    assert!(refused.get("error").is_none(), "{refused}");
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(refused["result"]["content"][0]["text"], "Duplicate primitive ID in the pin remap item.");
    assert_eq!(refused["result"]["structuredContent"]["error"]["code"], "invalid_params");
    let batch = reply(&mut server, &format!(r#"[{duplicate},{{"jsonrpc":"2.0","id":2,"method":"ping"}}]"#));
    assert_eq!(batch, json!([refused,{"jsonrpc":"2.0","id":2,"result":{}}]));

    let padded = request.replacen("\"objects\":{", &format!("\"objects\":{{{}", " ".repeat(1 << 20)), 1);
    assert!(padded.len() > 1 << 20);
    assert_eq!(reply(&mut server, &padded), valid, "raw spacing must not count against the normalized item limit");

    let mut invalid: Value = serde_json::from_str(&request).unwrap();
    invalid["params"]["arguments"]["params"]["item"] = json!({});
    let ordinary = server.handle(invalid.clone()).unwrap();
    assert_eq!(ordinary["result"]["isError"], true);
    assert_eq!(reply(&mut server, &invalid.to_string()), ordinary, "ordinary invalid items must keep their existing tool error");
}

/// Refuse two different raw bodies for one inactive unit while accepting two unique units.
#[test]
fn pin_remap_wire_refuses_duplicate_unit_names() {
    use pcbcraft_engine::authoring::{Action, Item, Kind, Primitive, Session as Editor};

    let mut editor = Editor::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let template = editor.snapshot().objects[&1].clone();
    for (unit, pin) in [("aux", "3"), ("spare", "4")] {
        editor.execute(editor.revision(), Action::AddUnit { unit: unit.into() }).unwrap();
        let mut primitive = template.clone();
        let Primitive::Pin { number, .. } = &mut primitive else { panic!("fixture primitive must be a pin") };
        *number = pin.into();
        editor.execute(editor.revision(), Action::Insert { primitive }).unwrap();
    }
    editor.execute(editor.revision(), Action::SelectUnit { unit: "main".into() }).unwrap();
    let item = serde_json::to_value(editor.snapshot()).unwrap();
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{
        "command":pcbcraft_engine::library::sources::PREVIEW_PIN_REMAP,"params":{"item":item,"pins":[{"unit":"main","object":"1","number":"7"}]}
    }}});
    let wire = request.to_string();
    let units = &item["other_units"];
    assert_ne!(units["aux"], units["spare"]);
    let duplicate = wire.replacen("\"other_units\":{", &format!("\"other_units\":{{\"aux\":{},", units["spare"]), 1);
    assert_ne!(duplicate, wire);
    assert_eq!(serde_json::from_str::<Value>(&duplicate).unwrap(), request, "normalization would silently keep the last aux body");

    let mut server = Server::new(Box::new(Headless::default()));
    let reply = |server: &mut Server, line: &str| serde_json::from_str::<Value>(&server.handle_line(line).unwrap()).unwrap();
    let accepted = reply(&mut server, &wire);
    assert!(accepted["result"]["content"][0]["text"].is_string(), "{accepted}");
    assert_ne!(accepted["result"]["isError"], true, "{accepted}");
    let refused = reply(&mut server, &duplicate);
    assert_eq!(refused["id"], 1);
    assert!(refused.get("error").is_none(), "{refused}");
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(refused["result"]["content"][0]["text"], "Duplicate unit name in the pin remap item.");
    assert_eq!(refused["result"]["structuredContent"]["error"]["code"], "invalid_params");

    for field in ["objects", "other_units"] {
        let prefix = format!("\"{field}\":{{");
        let repeated = wire.replacen(&prefix, &format!("\"{field}\":{{}},{prefix}"), 1);
        assert_ne!(repeated, wire);
        assert_eq!(serde_json::from_str::<Value>(&repeated).unwrap(), request);
        let refused = reply(&mut server, &repeated);
        assert_eq!(refused["id"], 1);
        assert!(refused.get("error").is_none(), "{refused}");
        assert_eq!(refused["result"]["isError"], true);
        assert_eq!(refused["result"]["content"][0]["text"], "Duplicate member in the pin remap item.");
        assert_eq!(refused["result"]["structuredContent"]["error"]["code"], "invalid_params");
    }
}

/// Designator preview data survives inspect:false, while applying the same shared command
/// changes the instance lock and participates in ordinary Undo.
#[test]
fn designator_lock_preview_and_apply_share_mcp_dispatch() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let revision = session.revision();
    let before = session.snapshot().unwrap();
    let mut params = json!({"command":pcbcraft_engine::SET_SYMBOL_FIELDS,"params":{"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":true,"dry_run":true,"expected_revision":revision},"inspect":false});
    let preview = crate::backend::call_session(&mut session, "engine.execute", &params).unwrap();
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    assert_eq!(preview["provisional"], true);
    assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    params["params"]["dry_run"] = json!(false);
    crate::backend::call_session(&mut session, "engine.execute", &params).unwrap();
    assert!(session.project().unwrap().components().next().unwrap().designator_locked());
    params["params"]["dry_run"] = json!(true);
    let stale = crate::backend::call_session(&mut session, "engine.execute", &params).unwrap_err();
    assert_eq!(stale.code(), "stale_revision");
    assert_eq!(stale.details(), &json!({"current_revision":session.revision()}));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Shared MCP dispatch preserves focused query data with inspect:false and uses the same edit.
#[test]
fn no_connect_commands_and_focused_check_are_shared_with_mcp() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.snapshot().unwrap();
    let target = p.no_connect_target(p.symbols()[0].terminals()[0]).unwrap();
    let revision = s.revision();
    crate::backend::call_session(
        &mut s,
        "engine.execute",
        &json!({"command":pcbcraft_engine::no_connect::SET,"params":{"target":target,"marked":true,"expected_revision":revision},"inspect":false}),
    )
    .unwrap();
    let revision = s.revision();
    let before = s.snapshot().unwrap();
    let report =
        crate::backend::call_session(&mut s, "engine.execute", &json!({"command":pcbcraft_engine::no_connect::CHECK,"params":{},"inspect":false}))
            .unwrap();
    assert_eq!(report["revision"], revision);
    assert_eq!(report["no_connect_check"]["markers"].as_array().unwrap().len(), 1);
    assert!(std::sync::Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// The same public command accepts wire-end identity and returns termination findings even
/// when callers omit the general inspection payload. No private GUI-only edit is involved.
#[test]
fn no_connect_wire_end_commands_are_shared_with_mcp() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(
        pcbcraft_engine::DRAW_WIRE,
        &json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":25_400_000,"y_nm":0},"bends":[],"expected_revision":s.revision()}),
    )
    .unwrap();
    let target = s.project().unwrap().wire_end_targets().next().unwrap().0;
    let revision = s.revision();
    crate::backend::call_session(
        &mut s,
        "engine.execute",
        &json!({"command":pcbcraft_engine::no_connect::SET,"params":{"target":target,"marked":true,"expected_revision":revision},"inspect":false}),
    )
    .unwrap();
    let report =
        crate::backend::call_session(&mut s, "engine.execute", &json!({"command":pcbcraft_engine::no_connect::CHECK,"params":{},"inspect":false}))
            .unwrap();
    assert_eq!(report["no_connect_check"]["wire_markers"].as_array().unwrap().len(), 1);
    assert_eq!(report["no_connect_check"]["wire_findings"].as_array().unwrap().len(), 1, "the other open end remains unmarked");
    let inspected = s.inspect();
    assert!(
        inspected["project"]["schematic"]["junctions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|end| end["id"] == target.junction && end["no_connect"] == true)
    );
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let rejected = crate::backend::call_session(
        &mut s,
        "engine.execute",
        &json!({"command":pcbcraft_engine::DRAW_WIRE,
        "params":{"from":{"junction":target.junction},"to":{"x_nm":-25_400_000,"y_nm":0},"bends":[],"expected_revision":revision},"inspect":false}),
    );
    assert!(rejected.unwrap_err().message().contains(pcbcraft_engine::no_connect::WIRE_BLOCKED));
    assert!(std::sync::Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    crate::backend::call_session(
        &mut s,
        "engine.execute",
        &json!({"command":pcbcraft_engine::DELETE_ITEMS,
        "params":{"items":{"symbols":[],"wires":[],"no_connects":[target]},"expected_revision":revision},"inspect":false}),
    )
    .unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).total_markers, 0);
    assert_eq!(s.project().unwrap().wires(), before.wires());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).total_markers, 1);
}

fn server() -> Server {
    Server::new(Box::new(Headless::default()))
}

/// Native publication preparation returns transported documents even with inspection disabled;
/// it never accepts a filesystem destination or reports a successful durable save.
#[test]
fn publication_preparation_mcp_returns_documents_without_paths_or_project_changes() {
    let mut s = server();
    let item = pcbcraft_engine::authoring::Item::new(pcbcraft_engine::authoring::Kind::Symbol).unwrap();
    let manifest = pcbcraft_engine::authoring::publishing::new_library("workshop", "Workshop", &item.license).unwrap();
    let initial = body(&call(&mut s, "execute", json!({"command":"project.new","params":{}})));
    let params = json!({"item":item,"documents":{"library.json":manifest}});
    let result = body(&call(&mut s, "execute", json!({"command":"library.prepare_publication","params":params,"inspect":false})));
    assert_eq!(result["saved"], false);
    assert_eq!(result["address"]["library"], "workshop");
    assert_eq!(result["documents"].as_object().unwrap().len(), 1);
    let mut copy_params = params.clone();
    copy_params["copy"] = json!(true);
    let copied = body(&call(&mut s, "execute", json!({"command":"library.prepare_publication","params":copy_params,"inspect":false})));
    assert_eq!(copied["copy"], true);
    assert_ne!(copied["address"]["id"], result["address"]["id"]);
    let part = pcbcraft_engine::part_authoring::Record::new_part(pcbcraft_engine::library::parts::Reference {
        library: "basic-components".into(),
        id: "connector".into(),
        revision: 4,
    })
    .unwrap();
    let prepared = body(&call(
        &mut s,
        "execute",
        json!({"command":"library.prepare_part_publication","params":{"record":part,"documents":{"library.json":params["documents"]["library.json"]}},"inspect":false}),
    ));
    assert_eq!(prepared["saved"], false);
    assert!(prepared["documents"].as_object().unwrap().keys().all(|path| path.starts_with("parts/")));

    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), initial);
    let mut invalid = params;
    invalid["path"] = json!("/tmp/forbidden-destination");
    assert_eq!(call(&mut s, "execute", json!({"command":"library.prepare_publication","params":invalid}))["isError"], true);
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

/// The catalog exposes saved defaults and provenance through real MCP dispatch. Bad bounds
/// and attempts to weaken saved limits preserve the project and history.
#[test]
fn board_rules_mcp_discovery_authorship_and_diagnostics_agree() {
    let mut s = server();
    let created = body(&call(&mut s, "execute", json!({"command":"project.new","params":{"create_board":true}})));
    let board = created["project"]["boards"][0]["id"].clone();
    let values = pcbcraft_engine::board::rules::Values { clearance_nm: 400_000, ..Default::default() };
    let changed = call(
        &mut s,
        "execute",
        json!({"command":pcbcraft_engine::SET_RULES,"params":{"board":board,"expected_revision":created["revision"],"rules":values}}),
    );
    assert_ne!(changed["isError"], true, "{changed}");
    let changed = body(&changed);
    assert_eq!(changed["project"]["boards"][0]["rules"]["effective"]["values"]["clearance_nm"], 400_000);
    let report = body(&call(&mut s, "execute", json!({"command":pcbcraft_engine::CHECK_DESIGN,"params":{"board":board}})));
    assert_eq!(report["design_check"]["rules"]["clearance_nm"], 400_000);
    assert_eq!(report["design_check"]["sources"]["clearance_nm"]["kind"], "board");
    let invalid = call(&mut s, "execute", json!({"command":pcbcraft_engine::CHECK_DESIGN,"params":{"board":board,"clearance_nm":0}}));
    assert_eq!(invalid["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), changed);
    let description = call(&mut s, "describe_command", json!({"command":pcbcraft_engine::SET_RULES}));
    assert_ne!(description["isError"], true, "{description}");
}

/// Real JSON-RPC tool dispatch preserves class query data when full inspection is suppressed.
#[test]
fn net_class_mcp_tools_author_inspect_resolve_and_reject_stale_edits() {
    use pcbcraft_engine::net_classes as classes;
    let mut s = server();
    let initial = body(&call(&mut s, "execute", json!({"command":"project.new","params":{}})));
    let created = call(
        &mut s,
        "execute",
        json!({"command":classes::CREATE,"params":{"expected_revision":initial["revision"],"name":"Power","rules":{"clearance_nm":500_000}}}),
    );
    assert_ne!(created["isError"], true, "{created}");
    let created = body(&created);
    let class = created["project"]["net_classes"]["definitions"][0]["id"].clone();
    let inspected = body(&call(&mut s, "execute", json!({"command":classes::INSPECT,"params":{},"inspect":false})));
    assert_eq!(inspected["net_classes"], created["project"]["net_classes"]);
    let resolved = body(&call(&mut s, "execute", json!({"command":classes::RESOLVE,"params":{"classes":[class]},"inspect":false})));
    assert_eq!(resolved["resolution"]["values"]["clearance_nm"], 500_000);
    assert_eq!(resolved["revision"], created["revision"]);
    let stale = call(&mut s, "execute", json!({"command":classes::REMOVE,"params":{"class":class,"expected_revision":initial["revision"]}}));
    assert_eq!(stale["isError"], true);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), created);
    assert_ne!(call(&mut s, "describe_command", json!({"command":classes::RESOLVE}))["isError"], true);
}

/// MCP uses the native record resolver and exact quantity normalizer without an open project.
#[test]
fn native_part_resolution_and_units_are_available_through_mcp() {
    let mut server = server();
    let documents = json!({
        "library.json":include_str!("../../../examples/native-parts/library.json"),
        "symbols/resistor.json":include_str!("../../../examples/native-parts/symbols/resistor.json"),
        "footprints/resistor-0603.json":include_str!("../../../examples/native-parts/footprints/resistor-0603.json"),
        "categories/resistor.json":include_str!("../../../examples/native-parts/categories/resistor.json"),
        "parts/resistor.json":include_str!("../../../examples/native-parts/parts/resistor.json")
    });
    let loaded = call(&mut server, "execute", json!({"command":"library.load_source","params":{"source":"demo","documents":documents}}));
    assert_ne!(loaded["isError"], true);
    let result = body(&call(
        &mut server,
        "execute",
        json!({"command":"library.resolve_part","inspect":false,"params":{"library":"parts-demo","definition":"resistor","definition_revision":1}}),
    ));
    assert_eq!(result["part"]["format"], "pcbcraft-part/1");
    assert_eq!(result["symbol"]["format"], "pcbcraft-symbol/3");
    let normalized = body(&call(
        &mut server,
        "execute",
        json!({"command":"library.parse_quantity","inspect":false,"params":{"quantity":"resistance","value":"100 mΩ"}}),
    ));
    assert_eq!(normalized, json!({"type":"quantity","value":"0.1","unit":"Ohm"}));
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
        fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), crate::Failure> {
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
    assert!(call_editor(&mut editor, &own).unwrap_err().message().contains("dialog"));
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
/// Check each occurrence's sheet/use references before normalizing its independently allocated IDs.
fn without_sheet_identity(mut state: Value) -> Value {
    if let Some(project) = state["project"].as_object_mut() {
        assert_eq!(project["id"].as_str().unwrap().len(), 36);
        project.insert("id".into(), json!("<independent-project>"));
    }
    if let Some(registry) = state["project"].get_mut("net_registry") {
        let scope = registry["scope"].clone();
        let records = registry["records"].as_array_mut().unwrap();
        let mut numbers = std::collections::BTreeMap::new();
        for record in records.iter_mut() {
            let id = record["id"].as_str().unwrap().to_owned();
            assert_eq!(id.len(), 36);
            let number = record["generated_number"].as_u64().unwrap();
            assert!(number > 0 && numbers.insert(id, number).is_none());
            record["id"] = json!(number);
            assert_eq!(record["state"]["kind"], "active");
            assert!(record["lineage"].as_array().unwrap().is_empty());
            normalize_net_anchor(&mut record["primary"], &scope);
            for anchor in record["fallbacks"].as_array_mut().unwrap() {
                normalize_net_anchor(anchor, &scope);
            }
        }
        records.sort_by_key(|record| record["generated_number"].as_u64().unwrap());
        for net in registry["nets"].as_array_mut().unwrap() {
            net["id"] = json!(numbers[net["id"].as_str().unwrap()]);
            normalize_net_anchor(&mut net["anchor"], &scope);
        }
        registry["scope"]["id"] = Value::Null;
    }
    let circuit_use = state["project"]["sheet_instances"]["circuit_uses"][0]["id"].clone();
    let definitions: Vec<_> = state["project"]["sheets"].as_array().unwrap().iter().map(|sheet| sheet["id"].clone()).collect();
    for occurrence in state["project"]["sheet_instances"]["sheet_occurrences"].as_array_mut().unwrap() {
        assert!(definitions.contains(&occurrence["definition"]));
        assert_eq!(occurrence["circuit_use"], circuit_use);
        for field in ["id", "definition", "circuit_use"] {
            assert_eq!(occurrence[field].as_str().unwrap().len(), 36);
            occurrence[field] = Value::Null;
        }
    }
    state["project"]["sheet_instances"]["circuit_uses"][0]["id"] = Value::Null;
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

/// Check exact owner consistency before normalizing entropy-generated project identities in
/// independent executions. Numeric object/terminal keys and generated net numbers stay exact.
fn normalize_net_anchor(anchor: &mut Value, scope: &Value) {
    assert_eq!(anchor["scope"], *scope);
    assert_eq!(anchor["project"].as_str().unwrap().len(), 36);
    anchor["project"] = Value::Null;
    anchor["scope"]["id"] = Value::Null;
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

/// Discovery retains every native library command alongside pin-remap preview over MCP;
/// source loading/removal leave the project's electrical revision and history alone.
#[test]
fn native_source_search_and_placement_share_the_mcp_session() {
    let mut s = server();
    let definitions = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/symbols/resistor.json"));
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/library.json"));
    let documents = json!({"library.json":manifest,"symbols/resistor.json":definitions});
    let commands = body(&call(&mut s, "list_commands", json!({"filter":"library."})));
    let mut command_ids = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["id"].as_str().unwrap())
        .filter(|id| id.starts_with("library."))
        .collect::<Vec<_>>();
    command_ids.sort_unstable();
    assert_eq!(
        command_ids,
        [
            "library.available_revisions",
            "library.list_sources",
            "library.load_source",
            "library.parse_quantity",
            "library.prepare_part_publication",
            "library.prepare_publication",
            "library.preview_pin_remap",
            "library.remove_source",
            "library.resolve_part",
            "library.search",
        ]
    );
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

/// Setup reviews return the same data through the native live adapter and headless MCP,
/// including when inspection is suppressed, while capabilities stay session-local.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn board_setup_preview_is_shared_by_live_and_headless_mcp() {
    use crate::{
        Backend,
        live::{Editor, call_editor},
    };
    use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, PREVIEW_BOARD_SETUP, SET_BOARD_SETUP};
    struct OpenEditor(Session);
    impl Editor for OpenEditor {
        /// Expose the editor-owned session without a second document path.
        fn session(&mut self) -> &mut Session {
            &mut self.0
        }
        /// This test host has no modal interaction.
        fn refusal(&self, _: &str) -> Option<String> {
            None
        }
        /// No presentation commands are needed for a setup review.
        fn window_commands(&self) -> Vec<Value> {
            vec![]
        }
        /// Project inspection is provided by the shared session.
        fn inspect(&self) -> Value {
            json!({})
        }
        /// The host rejects unrelated window commands.
        fn run_window_command(&mut self, _: &str, _: &Value) -> Result<(), crate::Failure> {
            Err("not a window command".into())
        }
    }
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":s.revision()})).unwrap();
    let snapshot = s.snapshot().unwrap();
    let candidate = snapshot.board_setup().unwrap().with_copper_count(2).unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot);
    let mut editor = OpenEditor(live);
    let before = editor.0.inspect();
    let revision = editor.0.revision();
    let params = json!({"command":PREVIEW_BOARD_SETUP,"params":{"setup":candidate,"expected_revision":revision},"inspect":false});
    let a = headless.call("engine.execute", params.clone()).unwrap()["board_setup_preview"].clone();
    let b = call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap()["board_setup_preview"].clone();
    assert_eq!(a["changes"], b["changes"]);
    assert_eq!(a["can_apply"], true);
    assert_ne!(a["plan_token"], b["plan_token"]);
    assert_eq!(editor.0.inspect(), before);
    let foreign = json!({"method":"engine.execute","params":{"command":SET_BOARD_SETUP,"params":{"setup":candidate,"plan_token":a["plan_token"],"expected_revision":revision}}});
    let error = call_editor(&mut editor, &foreign).unwrap_err();
    assert_eq!(error.code(), "unavailable");
    assert!(error.message().contains("unknown, stale or replaced board setup preview"), "{error}");
    let own = json!({"method":"engine.execute","params":{"command":SET_BOARD_SETUP,"params":{"setup":candidate,"plan_token":b["plan_token"],"expected_revision":revision}}});
    let applied = call_editor(&mut editor, &own).unwrap();
    assert_eq!(applied["revision"], revision + 1);
    assert_eq!(applied["project"]["board"]["setup"]["stackup"]["strata"].as_array().unwrap().len(), 3);
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

/// `list_commands` reports the command API version, the revision of the whole catalog (the same
/// with or without a filter, and equal to `catalog_revision` of the unfiltered list) and the
/// capabilities. `describe_command` and the catalog resource agree with the list, and the listed
/// contracts match the committed snapshot.
#[test]
fn discovery_reports_the_api_version_and_catalog_revision() {
    use pcbcraft_engine::command_api::{CAPABILITIES, COMMAND_API_VERSION, catalog_revision, check_snapshot};
    let mut s = server();
    body(&call(&mut s, "execute", json!({"command": "project.new"})));
    let listed = call(&mut s, "list_commands", json!({}));
    let commands = body(&listed);
    let about: Value = serde_json::from_str(listed["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(about, listed["structuredContent"]);
    let revision = catalog_revision(commands.as_array().unwrap());
    assert_eq!(
        about,
        json!({"command_api_version": COMMAND_API_VERSION, "catalog_revision": revision, "capabilities": CAPABILITIES, "scope":"edit"})
    );
    assert!(about["capabilities"].as_array().unwrap().contains(&json!("command_api")));
    let filtered = call(&mut s, "list_commands", json!({"filter": "undo"}));
    assert!(body(&filtered).as_array().unwrap().len() < commands.as_array().unwrap().len());
    assert_eq!(filtered["structuredContent"]["catalog_revision"], json!(revision));
    for command in commands.as_array().unwrap() {
        assert!(command.get("version").is_none(), "{command}");
        let described = call(&mut s, "describe_command", json!({"command": command["id"]}));
        assert_eq!(&body(&described), command);
        assert_eq!(described["structuredContent"], json!({"scope":"edit"}));
    }
    let snapshot: Value = serde_json::from_str(include_str!("../../../docs/schemas/command-contracts.json")).unwrap();
    assert_eq!(check_snapshot(&snapshot, "commands", commands.as_array().unwrap(), COMMAND_API_VERSION), Ok(()));
    let reply = rpc(&mut s, "resources/read", json!({"uri": "pcbcraft://commands"}));
    let resource: Value = serde_json::from_str(reply["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(resource, commands);
    let failed = call(&mut s, "describe_command", json!({"command": "schematic.nothing"}));
    assert_eq!(failed["isError"], true);
    assert_eq!(failed["structuredContent"]["error"]["code"], "unknown_command");
}

/// Net appearance commands are backward-compatible catalog additions: discovery keeps API version
/// 1, publishes their full patch schema, and the catalog revision still changes when they are absent.
#[test]
fn net_appearance_commands_are_version_one_catalog_additions() {
    use pcbcraft_engine::command_api::{COMMAND_API_VERSION, catalog_revision};
    use pcbcraft_engine::nets::{INSPECT_APPEARANCE, SET_APPEARANCE};

    assert_eq!(COMMAND_API_VERSION, 1);
    let mut s = server();
    let listed = call(&mut s, "list_commands", json!({}));
    assert_eq!(listed["structuredContent"]["command_api_version"], json!(COMMAND_API_VERSION));
    let commands = body(&listed);
    let commands = commands.as_array().unwrap();
    let found = |id: &str| commands.iter().find(|command| command["id"] == id).cloned().unwrap();
    let inspect = found(INSPECT_APPEARANCE);
    let set = found(SET_APPEARANCE);
    assert_eq!(inspect["id"], json!(INSPECT_APPEARANCE));
    assert_eq!(set["id"], json!(SET_APPEARANCE));
    let clear = &set["params"]["properties"]["appearance"]["oneOf"][1]["properties"]["clear"];
    assert_eq!(clear["items"]["enum"], json!(["color", "width_cp"]));

    let without: Vec<Value> =
        commands.iter().filter(|command| command["id"] != INSPECT_APPEARANCE && command["id"] != SET_APPEARANCE).cloned().collect();
    assert_eq!(commands.len(), without.len() + 2);
    assert_ne!(catalog_revision(&without), catalog_revision(commands));
}

/// Discovery and inspection expose the same immutable scope. Both return modes refuse the
/// edit as a coded tool failure, and the same connection remains able to read afterwards.
#[test]
fn headless_read_only_reports_scope_and_refuses_edits_in_both_modes() {
    let mut editor = Session::default();
    editor.apply("project.new", &json!({"name":"Review fixture"})).unwrap();
    let mut s = Server::new(Box::new(Headless::with_scope(pcbcraft_engine::SessionScope::ReadOnly, editor.snapshot())));
    let listed = call(&mut s, "list_commands", json!({"filter":"project.rename"}));
    assert_eq!(listed["structuredContent"]["scope"], "read_only");
    let described = call(&mut s, "describe_command", json!({"command":"project.rename"}));
    assert_eq!(body(&described), body(&listed)[0]);
    let scope = json!({"scope":"read_only"});
    assert_eq!(described["structuredContent"], scope);
    assert_eq!(described["content"][1]["text"], scope.to_string());
    let before = body(&call(&mut s, "inspect_project", json!({})));
    assert_eq!(before["scope"], "read_only");
    let version = pcbcraft_engine::command_api::COMMAND_API_VERSION;
    for inspect in [true, false] {
        let mut args = json!({"command":"project.rename","params":{"name":"Refuse this"},"inspect":inspect,"version":version + 1});
        let mismatch = call(&mut s, "execute", args.clone());
        assert_eq!(mismatch["isError"], true);
        assert_eq!(mismatch["structuredContent"]["error"]["code"], "version_mismatch");
        args["version"] = json!(version);
        let result = call(&mut s, "execute", args);
        let error = &result["structuredContent"]["error"];
        assert_eq!(result["isError"], true);
        assert_eq!(error["code"], "out_of_scope");
        assert_eq!(error["details"], json!({"command":"project.rename","scope":"read_only"}));
    }
    let query = body(&call(&mut s, "execute", json!({"command":"project.export_netlist","inspect":false})));
    assert_eq!(query["revision"], before["revision"]);
    assert!(query["netlist"].is_object());
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), before);
}

/// A failed headless tool call keeps its message as the first text part and adds the published
/// code and details, as a second text part and as the structured result.
#[test]
fn failures_carry_published_codes_and_details() {
    use pcbcraft_engine::error_codes::ErrorCode;
    let mut s = server();
    let created = body(&call(&mut s, "execute", json!({"command": "project.new"})));
    let revision = created["revision"].as_u64().unwrap();
    let code = |result: &Value, reason: &str| {
        assert_eq!(result["isError"], true, "{result}");
        let error = &result["structuredContent"]["error"];
        let second: Value = serde_json::from_str(result["content"][1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(second, result["structuredContent"]);
        assert_eq!(result["content"][0]["text"], error["message"]);
        assert!(error["message"].as_str().unwrap().contains(reason), "{result}: expected {reason:?}");
        assert!(error["details"].is_object(), "{result}");
        let code = error["code"].as_str().unwrap().to_owned();
        assert!(ErrorCode::parse(&code).is_some(), "{code} is not published");
        code
    };
    let nothing = call(&mut s, "execute", json!({"command": "edit.redo"}));
    assert_eq!(code(&nothing, "nothing to redo"), "unavailable");
    let params = json!({"x_nm": 0, "y_nm": 0, "expected_revision": revision + 3});
    let stale = call(&mut s, "execute", json!({"command": PLACE_RESISTOR, "params": params}));
    assert_eq!(code(&stale, "project changed"), "stale_revision");
    assert_eq!(stale["structuredContent"]["error"]["details"], json!({"current_revision": revision}));
    assert_eq!(stale["content"][0]["text"], format!("project changed, inspect the current revision ({revision}) and retry"));
    let extra = call(&mut s, "execute", json!({"command": PLACE_RESISTOR, "params": {"pin": 1}}));
    assert_eq!(code(&extra, "has no parameter `pin`"), "invalid_params");
    assert_eq!(code(&call(&mut s, "execute", json!({"command": "schematic.nothing"})), "unknown command `schematic.nothing`"), "unknown_command");
    assert_eq!(
        code(&call(&mut s, "execute", json!({"command": "view.fit", "params": {"view": 0}})), "this MCP session has none"),
        "needs_live_editor"
    );
    assert_eq!(code(&call(&mut s, "inspect_editor", json!({})), "this MCP session has none"), "needs_live_editor");
    assert_eq!(code(&call(&mut s, "cross_probe", json!({})), "provide either a terminal or a pad"), "invalid_params");
    assert_eq!(body(&call(&mut s, "inspect_project", json!({})))["revision"], revision);
}

/// Constructed failures retain an object of details and engine failures retain their revision.
#[test]
fn failure_details_are_objects() {
    use crate::Failure;
    use pcbcraft_engine::{CommandError, error_codes::ErrorCode};
    let details = serde_json::Map::from_iter([("current_revision".into(), json!(u64::MAX))]);
    let failure = Failure::new(ErrorCode::StaleRevision, "project changed").with_details(details.clone());
    assert_eq!(failure.details(), &Value::Object(details));
    assert!(Failure::new(ErrorCode::Unavailable, "nothing to undo").details().is_object());
    let from_engine = Failure::from(CommandError::StaleRevision { current: u64::MAX });
    assert_eq!(from_engine.code(), "stale_revision");
    assert_eq!(from_engine.details(), &json!({"current_revision": u64::MAX}));
}

/// The transport accepts a complete JSON frame at EOF without its newline. A send failure
/// after the JSON bytes, before the newline, therefore cannot establish that nothing ran.
#[test]
fn a_complete_request_at_eof_can_execute_without_a_newline() {
    let request = json!({"method": "engine.execute", "params": {"command": "project.new", "params": {"name": "EOF request"}}});
    let bytes = serde_json::to_vec(&request).unwrap();
    let frame = craft_control::read_frame(&mut Cursor::new(&bytes), bytes.len()).unwrap().unwrap();
    let craft_control::Frame::Message(delivered) = frame else { panic!("request should fit") };
    assert_eq!(delivered, bytes);
    let delivered: Value = serde_json::from_slice(&delivered).unwrap();
    let mut session = Session::default();
    crate::backend::call_session(&mut session, delivered["method"].as_str().unwrap(), &delivered["params"]).unwrap();
    assert_eq!(session.project().unwrap().name, "EOF request");
}

/// Discovery distinguishes a typo in an editor's catalog from a headless-only limitation.
#[test]
fn describe_command_distinguishes_live_typos_from_headless_refusals() {
    struct LiveCatalog;
    impl crate::Backend for LiveCatalog {
        fn call(&mut self, method: &str, _: Value) -> Result<Value, crate::Failure> {
            assert_eq!(method, "engine.commands");
            Ok(json!([{"id":"view.fit"}]))
        }
        fn has_live_editor(&self) -> bool {
            true
        }
        fn describe(&self) -> String {
            "a live catalog fixture".into()
        }
    }
    let mut live = Server::new(Box::new(LiveCatalog));
    let mut headless = server();
    let ok = call(&mut live, "describe_command", json!({"command":"view.fit"}));
    assert_eq!(body(&ok)["id"], "view.fit");
    let expect_refusal = |server: &mut Server, id: &str, code: &str, reason: &str| {
        let refused = call(server, "describe_command", json!({"command":id}));
        assert_eq!(refused["isError"], true, "{refused}");
        assert_eq!(refused["structuredContent"]["error"]["code"], code, "{refused}");
        assert!(refused["content"][0]["text"].as_str().unwrap().contains(reason), "{refused}");
    };
    expect_refusal(&mut live, "view.no_such_command", "unknown_command", "unknown command `view.no_such_command`");
    for id in ["view.fit", "view.no_such_command"] {
        expect_refusal(&mut headless, id, "needs_live_editor", "acts on an open PCBCraft editor window");
    }
}

/// MCP omission reaches the direct empty-map convention; explicit null fails tool validation.
#[test]
fn execute_omission_and_null_differ_at_the_mcp_boundary() {
    let mut server = server();
    let omitted = call(&mut server, "execute", json!({"command":"project.new"}));
    assert!(body(&omitted)["project"].is_object(), "{omitted}");
    let explicit = rpc(&mut server, "tools/call", json!({"name":"execute","arguments":{"command":"project.new","params":null}}));
    assert_eq!(explicit["error"]["code"], -32602, "{explicit}");
    assert!(explicit.get("result").is_none(), "{explicit}");
}

/// Live error replies preserve valid codes and object details, with unavailable as the legacy
/// fallback. The closed-port fixture reports a connection failure with the selected port.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn live_replies_keep_their_codes() {
    use crate::{Backend, Failure};
    let coded = json!({"error": "project changed", "code": "stale_revision", "details": {"current_revision": 4}});
    let failure = Failure::from_reply(&coded, "project changed");
    assert_eq!((failure.code(), failure.message()), ("stale_revision", "project changed"));
    assert_eq!(failure.details(), &json!({"current_revision": 4}));
    let newer = Failure::from_reply(&json!({"error": "x", "code": "future_code"}), "x");
    assert_eq!(newer.code(), "future_code");
    for details in [Value::Null, json!([1]), json!(7), json!("revision"), json!(true)] {
        let failure = Failure::from_reply(&json!({"code": "stale_revision", "details": details}), "project changed");
        assert_eq!(failure.code(), "stale_revision");
        assert_eq!(failure.message(), "project changed");
        assert_eq!(failure.details(), &json!({}));
    }
    for reply in [
        json!({"error": "busy"}),
        json!({"error": "x", "code": 7}),
        json!({"error": "x", "code": "Not Found"}),
        json!({"error": "x", "code": ""}),
        json!({"error": "x", "code": "_found"}),
        json!({"error": "x", "details": [1]}),
    ] {
        let failure = Failure::from_reply(&reply, "x");
        assert_eq!(failure.code(), "unavailable", "{reply}");
        assert_eq!(failure.details(), &json!({}));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut remote = crate::live::Remote::new(port, &"a".repeat(64)).unwrap();
    let refused = remote.call("project.inspect", json!({})).unwrap_err();
    assert_eq!(refused.code(), "connection_failed", "{refused}");
    assert!(refused.message().starts_with(&format!("Could not send request to the PCBCraft editor on port {port}:")), "{refused}");
}

/// A lost, unreadable or unusable reply reports no_reply even when the shared command ran.
/// Acceptance and socket I/O are bounded; the synthetic peer never retries or runs a second edit.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn no_reply_can_follow_an_executed_edit() {
    use crate::Backend;
    use std::{
        io::{BufReader, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let cases: &[(&[u8], &str)] = &[
        (b"", "No reply from the PCBCraft editor. Check the connection and inspect the project before retrying an edit."),
        (b"not json\n", "No reply from the PCBCraft editor. Check the connection and inspect the project before retrying an edit."),
        (b"{}\n", "Invalid reply from the PCBCraft editor"),
        (b"[]\n", "Invalid reply from the PCBCraft editor"),
        (b"7\n", "Invalid reply from the PCBCraft editor"),
        (b"{\"error\":42}\n", "Invalid reply from the PCBCraft editor"),
        (b"{\"result\":null,\"error\":\"ambiguous\"}\n", "Invalid reply from the PCBCraft editor"),
    ];
    for &(reply, reason) in cases {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::scope(|scope| {
            let peer = scope.spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                let socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "no request reached the synthetic peer");
                            std::thread::yield_now();
                        }
                        Err(error) => panic!("could not accept the synthetic request: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                socket.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut reader = BufReader::new(socket);
                let frame = craft_control::read_frame(&mut reader, 64 * 1024).unwrap().unwrap();
                let craft_control::Frame::Message(bytes) = frame else { panic!("request should fit") };
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request["token"], "a".repeat(64));
                assert_eq!(request["method"], "engine.execute");
                assert_eq!(request["params"]["command"], "project.rename");
                let mut session = Session::default();
                session.apply("project.new", &json!({"name":"Before reply"})).unwrap();
                let before = session.revision();
                crate::backend::call_session(&mut session, request["method"].as_str().unwrap(), &request["params"]).unwrap();
                let after = session.revision();
                let name = session.project().unwrap().name.clone();
                reader.get_mut().write_all(reply).unwrap();
                (before, after, name)
            });
            let mut remote = crate::live::Remote::new(port, &"a".repeat(64)).unwrap();
            let failure = remote.call("engine.execute", json!({"command":"project.rename","params":{"name":"After reply"}})).unwrap_err();
            let (before, after, name) = peer.join().unwrap();
            assert_eq!(failure.code(), "no_reply", "{reply:?}: {failure}");
            assert!(failure.message().starts_with(reason), "{reply:?}: {failure}");
            assert_eq!(failure.details(), &json!({}));
            assert_eq!(after, before + 1);
            assert_eq!(name, "After reply");
        });
    }
}

/// Direct live requests report invalid_request before dispatch for missing methods or command
/// ids; headless dispatch shares command-id validation. Each refusal leaves the session intact.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn invalid_control_requests_leave_the_project_alone() {
    use crate::{Backend, Failure, live::Editor};
    struct OpenSession(Session);
    impl Editor for OpenSession {
        /// Borrow the real command session without another dispatch layer.
        fn session(&mut self) -> &mut Session {
            &mut self.0
        }
        /// Permit requests so host policy cannot mask request validation.
        fn refusal(&self, _command: &str) -> Option<String> {
            None
        }
        /// This fixture has no window command catalog.
        fn window_commands(&self) -> Vec<Value> {
            vec![]
        }
        /// Inspect the same session used by command dispatch.
        fn inspect(&self) -> Value {
            self.0.inspect()
        }
        /// Window dispatch is outside this fixture's supported methods.
        fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), Failure> {
            Err(Failure::new(pcbcraft_engine::error_codes::ErrorCode::Unavailable, "no window commands in this fixture"))
        }
    }
    let mut editor = OpenSession(Session::default());
    editor.0.apply("project.new", &json!({"name":"Request validation"})).unwrap();
    let before = editor.0.inspect();
    for (request, reason) in [
        (json!({}), "missing control method"),
        (json!({"method":null}), "missing control method"),
        (json!({"method":7}), "missing control method"),
        (json!({"method":"unknown.control.method"}), "Unknown live control method `unknown.control.method`"),
        (json!({"method":"engine.execute"}), "missing `command`"),
        (json!({"method":"engine.execute","params":{"command":null}}), "missing `command`"),
        (json!({"method":"engine.execute","params":{"command":7}}), "missing `command`"),
    ] {
        let failure = crate::live::call_editor(&mut editor, &request).unwrap_err();
        assert_eq!(failure.code(), "invalid_request", "{request}: {failure}");
        assert_eq!(failure.message(), reason, "{request}");
        assert_eq!(editor.0.inspect(), before);
    }
    let mut headless = Headless::default();
    headless.call("engine.execute", json!({"command":"project.new","params":{"name":"Headless validation"}})).unwrap();
    let before = headless.call("project.inspect", json!({})).unwrap();
    for (method, params, reason) in [
        ("unknown.control.method", json!({}), "unknown method `unknown.control.method`"),
        ("engine.execute", Value::Null, "missing `command`"),
        ("engine.execute", json!({}), "missing `command`"),
        ("engine.execute", json!({"command":null}), "missing `command`"),
        ("engine.execute", json!({"command":7}), "missing `command`"),
    ] {
        let failure = headless.call(method, params).unwrap_err();
        assert_eq!(failure.code(), "invalid_request");
        assert_eq!(failure.message(), reason);
        assert_eq!(headless.call("project.inspect", json!({})).unwrap(), before);
    }
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

/// Complete correspondence is additive: legacy singular payloads keep their published shape.
#[test]
fn resolve_counterparts_preserves_legacy_cross_probe_and_returns_complete_sets() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let project = setup.project().unwrap();
    let component = project.components().next().unwrap().id();
    let terminal = project.symbols()[0].terminals()[0].inspect();
    let sheet = project.sheet_target().id();
    let package = setup.project().unwrap().footprint_catalog().into_iter().find(|package| package.id == "sot23").unwrap();
    setup
        .apply(
            pcbcraft_engine::board::assignment::ASSIGN_FOOTPRINT,
            &json!({"component":component,"library":package.library,
        "definition":package.id,"definition_revision":package.revision,"mapping":{"1":["1","3"],"2":"2"},"expected_revision":setup.revision()}),
        )
        .unwrap();
    setup
        .apply(
            pcbcraft_engine::board::assignment::PLACE_ASSIGNED_FOOTPRINT,
            &json!({"component":component,"x_nm":0,"y_nm":0,"expected_revision":setup.revision()}),
        )
        .unwrap();
    let mut server = Server::new(Box::new(Headless::with_project(setup.snapshot().unwrap())));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let project_id = before["project"]["id"].clone();
    assert!(project_id.as_str().is_some(), "qualified query inputs must be discoverable through inspection");
    let source = json!({"kind":"terminal","project":project_id,"sheet":sheet,"symbol":terminal["symbol"],"terminal":terminal["terminal"]});
    let params = json!({"source":source,"expected_revision":before["revision"]});
    let response = call(&mut server, "resolve_counterparts", params.clone());
    assert!(response.is_object(), "expected a tool result, got {response}");
    assert_ne!(response["isError"], true, "{response}");
    let result = body(&response);
    assert_eq!(result["format"], "pcbcraft-counterparts/1");
    assert_eq!(result["status"], "resolved");
    assert_eq!(result["pads"].as_array().unwrap().len(), 2);
    assert_eq!(result["terminals"].as_array().unwrap().len(), 1);
    let old = body(&call(&mut server, "cross_probe", json!({"terminal":terminal})));
    assert_eq!(old.as_object().unwrap().len(), 1);
    assert_eq!(old["pad"]["pad"], "1");
    assert_eq!(body(&call(&mut server, "cross_probe", json!({"pad":old["pad"]}))), json!({"terminal":terminal}));
    for pad in result["pads"].as_array().unwrap() {
        let reverse = body(&call(&mut server, "resolve_counterparts", json!({"source":pad,"expected_revision":before["revision"]})));
        assert_eq!(reverse["terminals"], result["terminals"]);
        assert_eq!(reverse["pads"], result["pads"]);
    }
    let stale = body(&call(&mut server, "resolve_counterparts", json!({"source":source,"expected_revision":0})));
    assert_eq!(stale["status"], "stale");
    assert_eq!(stale["pads"], json!([]));
    assert_eq!(stale["terminals"], json!([]));
    for (invalid, reason) in [
        (json!({"source":{},"expected_revision":0}), "missing field `kind`"),
        (json!({"source":source,"expected_revision":-1}), "expected_revision must be an unsigned integer"),
    ] {
        let refused = call(&mut server, "resolve_counterparts", invalid);
        assert_eq!(refused["isError"], true, "{refused}");
        assert_eq!(refused["structuredContent"]["error"]["code"], "invalid_params", "{refused}");
        assert_eq!(refused["content"][0]["text"], reason, "{refused}");
    }
    for invalid in [
        json!({"source":source}),
        json!({"source":source,"expected_revision":1.5}),
        json!({"source":{"kind":"terminal","project":project_id,"sheet":sheet,"symbol":"symbol-01","terminal":"1"},"expected_revision":before["revision"]}),
        json!({"source":{"kind":"terminal","project":project_id,"sheet":sheet,"symbol":terminal["symbol"],"terminal":"1","extra":true},"expected_revision":before["revision"]}),
    ] {
        let refused = rpc(&mut server, "tools/call", json!({"name":"resolve_counterparts","arguments":invalid}));
        assert!(refused.get("error").is_some() || refused["result"]["isError"] == true, "{refused}");
    }
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
    let tool = tool_definitions().into_iter().find(|tool| tool["name"] == "resolve_counterparts").unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
}

/// Live and headless queries use the same resolver without invoking editor commands or edits.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn resolve_counterparts_agrees_through_headless_and_live_dispatch() {
    use crate::{
        Backend,
        live::{Editor, call_editor},
    };
    struct OpenEditor(Session);
    impl Editor for OpenEditor {
        /// The fixture shares the engine session with the live dispatcher.
        fn session(&mut self) -> &mut Session {
            &mut self.0
        }
        /// A query is not an editor command and must not reach this path.
        fn refusal(&self, _: &str) -> Option<String> {
            Some("unexpected editor command".into())
        }
        /// No presentation commands are needed for model correspondence.
        fn window_commands(&self) -> Vec<Value> {
            vec![]
        }
        /// Query dispatch must not replace its result with editor inspection.
        fn inspect(&self) -> Value {
            json!({"unexpected":true})
        }
        /// A correspondence query never invokes an editor mutation.
        fn run_window_command(&mut self, _: &str, _: &Value) -> Result<(), crate::Failure> {
            Err("unexpected editor command".into())
        }
    }
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let snapshot = setup.snapshot().unwrap();
    let terminal = snapshot.symbols()[0].terminals()[0].inspect();
    let source = json!({"kind":"terminal","project":snapshot.id(),"sheet":snapshot.sheet_target().id(),"symbol":terminal["symbol"],"terminal":terminal["terminal"]});
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot);
    let mut editor = OpenEditor(live);
    let before = headless.call("project.inspect", json!({})).unwrap();
    let params = json!({"source":source,"expected_revision":before["revision"]});
    let actual = headless.call("project.resolve_counterparts", params.clone()).unwrap();
    assert_eq!(actual["status"], "unassigned");
    assert_eq!(call_editor(&mut editor, &json!({"method":"project.resolve_counterparts","params":params})).unwrap(), actual);
    assert_eq!(headless.call("project.inspect", json!({})).unwrap(), before);
    assert_eq!(editor.0.inspect(), before);
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
        ("execute", json!({"command": "ui.click", "params": {"window":0,"target":{"handle":"toolbar:Connect:Wire"}}})),
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

/// The placement origin is set through the shared dispatcher, and the fabrication result names
/// the origin its coordinates are measured from.
#[test]
fn fabrication_result_names_the_placement_origin_it_used() {
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
    let description = call(&mut server, "describe_command", json!({"command":pcbcraft_engine::SET_PLACEMENT_ORIGIN}));
    assert_ne!(description["isError"], true, "{description}");
    let export = |server: &mut Server| {
        let response = call(server, "execute", json!({"command":pcbcraft_engine::EXPORT_FABRICATION,"params":{"board":board.board},"inspect":false}));
        assert_ne!(response["isError"], true, "{response}");
        body(&response)["fabrication"]["origin"].clone()
    };
    assert_eq!(export(&mut server)["source"], json!({"kind":"builtin"}));
    let revision = body(&call(&mut server, "inspect_project", json!({})))["revision"].clone();
    let response = call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::SET_PLACEMENT_ORIGIN,"params":{"board":board.board,"expected_revision":revision,"origin":{"kind":"outline_lower_left"}}}),
    );
    assert_ne!(response["isError"], true, "{response}");
    let authored = &body(&response)["project"]["boards"][0]["placement_origin"]["authored"];
    let origin = export(&mut server);
    assert_eq!(origin["anchor"], json!({"kind":"outline_lower_left"}));
    assert_eq!(origin["source"], json!({"kind":"board","origin":authored["id"]}));
    assert_eq!(origin["at"], json!({"x_nm":0,"y_nm":10_000_000}));
    assert_eq!(origin["axes"], "x_right_y_up");
}

/// Immutable profiles are pinned, previewed, adopted and retired through the shared dispatcher;
/// malformed content is refused without changing the catalog or board adoption.
#[test]
fn assembly_output_profiles_round_trip_through_execute() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    let board = setup.project().unwrap().board_target().unwrap().board;
    let mut server = Server::new(Box::new(Headless::with_project(setup.snapshot().unwrap())));
    let description = call(&mut server, "describe_command", json!({"command":pcbcraft_engine::SET_ASSEMBLY_PROFILE}));
    assert_ne!(description["isError"], true, "{description}");
    let set = |server: &mut Server, profile: Option<&Value>, content: Value| {
        let revision = body(&call(server, "inspect_project", json!({})))["revision"].clone();
        let mut params = json!({"expected_revision":revision,"content":content});
        if let Some(profile) = profile {
            params["profile"] = profile.clone();
        }
        call(server, "execute", json!({"command":pcbcraft_engine::SET_ASSEMBLY_PROFILE,"params":params}))
    };
    let profile = json!({"revision":1,"provenance":{"kind":"authored"},"name":"Reel","bottom_side":"viewed_from_bottom","corrections":[
        {"library":"basic-components","definition":"sot23","rotation":180},
        {"library":"basic-components","definition":"capacitor-0402","centroid":{"x_nm":100_000,"y_nm":0}}
    ]});
    let response = set(&mut server, None, profile);
    assert_ne!(response["isError"], true, "{response}");
    let profiles = body(&response)["project"]["assembly_profiles"]["revisions"].clone();
    assert_eq!(profiles.as_array().unwrap().len(), 1);
    assert_eq!(profiles[0]["format"], "pcbcraft-assembly-profile/1");
    assert_eq!(profiles[0]["content"]["bottom_side"], "viewed_from_bottom");
    assert_eq!(profiles[0]["content"]["corrections"][0]["definition"], "capacitor-0402", "corrections come back in canonical order");
    assert_eq!(profiles[0]["content"]["corrections"][1], json!({"library":"basic-components","definition":"sot23","rotation":180}));
    assert_eq!(body(&response)["project"]["boards"][0]["assembly_profiles"], json!([]));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let mut adoption = json!({"board":board,"expected_revision":before["revision"],"profile":profiles[0]["id"],"revision":1});
    let reviewed = call(&mut server, "execute", json!({"command":pcbcraft_engine::PREVIEW_ASSEMBLY_PROFILE,"params":adoption,"inspect":false}));
    assert_ne!(reviewed["isError"], true, "{reviewed}");
    let review = body(&reviewed);
    assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), before);
    adoption["acceptance"] = review["acceptance"].clone();
    let accepted = call(&mut server, "execute", json!({"command":pcbcraft_engine::ADOPT_ASSEMBLY_PROFILE,"params":adoption}));
    assert_ne!(accepted["isError"], true, "{accepted}");

    let refused = set(
        &mut server,
        None,
        json!({"revision":1,"provenance":{"kind":"authored"},"name":"Bad","corrections":[{"library":"basic-components","definition":"sot23","rotation":360}]}),
    );
    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["content"][0]["type"], "text");
    let reason = refused["content"][0]["text"].as_str().unwrap();
    assert!(reason.contains("`content`") && reason.contains("immutable profile content"), "{refused}");
    let inspected = body(&call(&mut server, "inspect_project", json!({})));
    assert_eq!(inspected["project"]["assembly_profiles"]["revisions"], profiles);
    assert_eq!(inspected["project"]["boards"][0]["assembly_profiles"], json!([{"profile":profiles[0]["id"],"revision":1}]));

    let response = set(&mut server, Some(&profiles[0]["id"]), Value::Null);
    assert_ne!(response["isError"], true, "{response}");
    assert_eq!(body(&response)["project"]["boards"][0]["assembly_profiles"], inspected["project"]["boards"][0]["assembly_profiles"]);
    assert_eq!(body(&response)["project"]["assembly_profiles"]["heads"][0]["removed"], true);
    assert_eq!(body(&response)["project"]["assembly_profiles"]["revisions"], profiles);
}

/// Suppressing inspection preserves identical read-only profile previews on headless and live dispatch.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn assembly_output_profiles_preview_through_headless_and_live() {
    use crate::{
        Backend,
        live::{Editor, call_editor},
    };
    struct OpenEditor(Session);
    impl Editor for OpenEditor {
        /// The live dispatcher uses this same command session.
        fn session(&mut self) -> &mut Session {
            &mut self.0
        }
        /// This fixture has no modal interaction.
        fn refusal(&self, _command: &str) -> Option<String> {
            None
        }
        /// The fixture exposes engine commands only.
        fn window_commands(&self) -> Vec<Value> {
            vec![]
        }
        /// No editor-local state participates in a profile review.
        fn inspect(&self) -> Value {
            json!({})
        }
        /// A mistaken window dispatch fails explicitly with an unavailable failure.
        fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), crate::Failure> {
            Err("not a window operation".into())
        }
    }
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    let id = "00000000-0000-0000-0000-000000000093";
    setup.apply(pcbcraft_engine::SET_ASSEMBLY_PROFILE,&json!({"expected_revision":setup.revision(),"profile":id,"content":{"revision":1,"name":"Portable","provenance":{"kind":"authored"},"bottom_side":"viewed_from_bottom"}})).unwrap();
    let board = setup.project().unwrap().board_target().unwrap().board;
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot);
    let mut editor = OpenEditor(live);
    let before = headless.call("project.inspect", json!({})).unwrap();
    let params = json!({"board":board,"expected_revision":before["revision"],"profile":id,"revision":1});
    let request = json!({"command":pcbcraft_engine::PREVIEW_ASSEMBLY_PROFILE,"params":params,"inspect":false});
    let headless_review = headless.call("engine.execute", request.clone()).unwrap();
    let live_review = call_editor(&mut editor, &json!({"method":"engine.execute","params":request})).unwrap();
    assert_eq!(serde_json::to_vec(&headless_review).unwrap(), serde_json::to_vec(&live_review).unwrap());
    assert!(headless_review["acceptance"].as_str().is_some_and(|digest| digest.len() == 64));
    assert_eq!(headless.call("project.inspect", json!({})).unwrap(), before);
    assert_eq!(editor.0.inspect(), before);
    let mut accept = params;
    accept["acceptance"] = headless_review["acceptance"].clone();
    let request = json!({"command":pcbcraft_engine::ADOPT_ASSEMBLY_PROFILE,"params":accept,"inspect":false});
    assert_eq!(
        headless.call("engine.execute", request.clone()).unwrap(),
        call_editor(&mut editor, &json!({"method":"engine.execute","params":request})).unwrap()
    );
    assert_eq!(headless.call("project.inspect", json!({})).unwrap(), editor.0.inspect());
}

/// A netlist export with inspect:false still returns its file and leaves the project unchanged.
#[test]
fn netlist_file_survives_suppressed_project_inspection() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({})).unwrap();
    setup.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let mut server = Server::new(Box::new(Headless::with_project(setup.snapshot().unwrap())));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    let response = call(&mut server, "execute", json!({"command":pcbcraft_engine::netlist::EXPORT_NETLIST,"params":{},"inspect":false}));
    assert_ne!(response["isError"], true, "{response}");
    let result = body(&response);
    let files = result["netlist"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0]["name"].as_str().unwrap().ends_with("-netlist.json"));
    assert!(files[0]["contents"].as_str().unwrap().contains("\"components\""));
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

/// MCP advertises explicit pinned unit placement and shares one part instance across drawings.
#[test]
fn multiunit_placement_uses_shared_commands_and_part_identity() {
    let mut s = server();
    let schema = body(&call(&mut s, "describe_command", json!({"command":pcbcraft_engine::PLACE_UNIT})));
    assert_eq!(schema["params"]["properties"]["unit"]["type"], "string");
    let opened = body(&call(&mut s, "execute", json!({"command":"project.new","params":{}})));
    let loaded = call(
        &mut s,
        "execute",
        json!({"command":"library.load_source","params":{"source":"units","documents":{
            "library.json":include_str!("../../../examples/native-parts/library.json"),
            "symbols/dual.json":include_str!("../../../examples/native-parts/symbols/dual-amplifier.json")
        }}}),
    );
    assert_ne!(loaded["isError"], true);
    let found = body(&call(&mut s, "execute", json!({"command":"library.search","params":{"query":"dual-amplifier","kind":"symbol"}})));
    assert_eq!(found[0]["units"], json!(["a", "b"]));
    let first = body(&call(
        &mut s,
        "execute",
        json!({"command":PLACE_SYMBOL,"params":{
            "library":"parts-demo","definition":"dual-amplifier","definition_revision":1,"definition_digest":found[0]["digest"],
            "unit":"b","x_nm":0,"y_nm":0,"expected_revision":opened["revision"]
        }}),
    ));
    let symbol = &first["project"]["schematic"]["symbols"][0];
    assert_eq!(symbol["display_reference"], "U1 B");
    assert_eq!(symbol["available_units"], json!(["a"]));
    let second = body(&call(
        &mut s,
        "execute",
        json!({"command":pcbcraft_engine::PLACE_UNIT,"params":{
            "component":symbol["component_id"],"unit":"a","x_nm":30_480_000,"y_nm":0,"expected_revision":first["revision"]
        }}),
    ));
    assert_eq!(second["project"]["components"].as_array().unwrap().len(), 1);
    let symbols = second["project"]["schematic"]["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0]["component_id"], symbols[1]["component_id"]);
}

/// MCP places a central native part/MPN atomically and edits requirements offline through the same engine authority.
#[test]
fn native_part_selection_and_compatibility_are_available_through_mcp() {
    let mut s = server();
    let created = body(&call(&mut s, "execute", json!({"command":"project.new","params":{}})));
    let loaded = call(
        &mut s,
        "execute",
        json!({"command":"library.load_source","params":{"source":"native-parts","documents":{
            "library.json":include_str!("../../../examples/native-parts/library.json"),
            "symbols/resistor.json":include_str!("../../../examples/native-parts/symbols/resistor.json"),
            "footprints/resistor.json":include_str!("../../../examples/native-parts/footprints/resistor-0603.json"),
            "categories/resistor.json":include_str!("../../../examples/native-parts/categories/resistor.json"),
            "parts/resistor.json":include_str!("../../../examples/native-parts/parts/resistor.json"),
            "manufacturer-parts/test.json":include_str!("../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")
        }}}),
    );
    assert_ne!(loaded["isError"], true);
    let resolved = body(&call(
        &mut s,
        "execute",
        json!({"command":"library.resolve_part","params":{"library":"parts-demo","definition":"resistor","definition_revision":1}}),
    ));
    let manufacturers =
        body(&call(&mut s, "execute", json!({"command":"library.search","params":{"kind":"manufacturer_part","query":"TEST-R100M"}})));
    let placed = body(&call(
        &mut s,
        "execute",
        json!({"command":"schematic.place_part","params":{
            "library":"parts-demo","definition":"resistor","definition_revision":1,"definition_digest":resolved["digest"],
            "manufacturer":{"library":"parts-demo","id":"test-resistor-100m","revision":1},"manufacturer_digest":manufacturers[0]["digest"],
            "x_nm":0,"y_nm":0,"expected_revision":created["revision"]
        }}),
    ));
    let instance = &placed["project"]["components"][0];
    assert_eq!(instance["part"]["manufacturer"]["mpn"], "TEST-R100M-0603");
    assert_eq!(instance["part"]["compatibility"]["status"], "meets_requirements");
    let removed = call(&mut s, "execute", json!({"command":"library.remove_source","params":{"source":"native-parts"}}));
    assert_ne!(removed["isError"], true, "{removed}");
    let params = json!({"component":instance["id"],"requirements":{
        "pcbcraft:tolerance":{"operator":"at_most","value":{"type":"quantity","value":"0.005","unit":"1"}}
    },"expected_revision":placed["revision"]});
    let edited = call(&mut s, "execute", json!({"command":"part.set_selection","params":params}));
    assert_ne!(edited["isError"], true);
    let edited = body(&edited);
    assert_eq!(edited["project"]["components"][0]["part"]["compatibility"]["status"], "does_not_meet_requirements");
    let stale = call(&mut s, "execute", json!({"command":"part.set_selection","params":params}));
    assert_eq!(stale["isError"], true);
}

/// MCP exposes explicit board lock intent and the same atomic protection as the editor.
#[test]
fn copper_lock_commands_and_protected_edits_share_mcp_inspection() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":setup.revision()})).unwrap();
    let board = setup.project().unwrap().board_target().unwrap();
    let layer = setup
        .project()
        .unwrap()
        .board_view(board)
        .unwrap()
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper)
        .unwrap()
        .id;
    setup
        .apply(
            pcbcraft_engine::ADD_TRACKS,
            &json!({"board":board.board,"layer":layer,"width_nm":250_000,
        "points":[{"x_nm":0,"y_nm":0},{"x_nm":5_000_000,"y_nm":0}],"expected_revision":setup.revision()}),
        )
        .unwrap();
    let track = setup.project().unwrap().board_view(board).unwrap().tracks()[0].id;
    let mut server = Server::new(Box::new(Headless::with_project(setup.snapshot().unwrap())));
    let before = body(&call(&mut server, "inspect_project", json!({})));
    assert_eq!(before["project"]["boards"][0]["tracks"][0]["locked"], false);
    let locked = call(
        &mut server,
        "execute",
        json!({"command":pcbcraft_engine::SET_COPPER_LOCKED,
        "params":{"board":board.board,"tracks":[track],"locked":true,"expected_revision":before["revision"]}}),
    );
    assert_ne!(locked["isError"], true, "{locked}");
    let locked = body(&locked);
    assert_eq!(locked["project"]["boards"][0]["tracks"][0]["locked"], true);
    for command in [pcbcraft_engine::DELETE_TRACKS, pcbcraft_engine::MOVE_TRACKS] {
        let mut params = json!({"board":board.board,"tracks":[track],"expected_revision":locked["revision"]});
        if command == pcbcraft_engine::MOVE_TRACKS {
            params["dx_nm"] = json!(1000);
            params["dy_nm"] = json!(0);
        }
        let result = call(&mut server, "execute", json!({"command":command,"params":params}));
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("locked"));
        assert_eq!(body(&call(&mut server, "inspect_project", json!({}))), locked);
    }
    let undone = body(&call(&mut server, "execute", json!({"command":"edit.undo","params":{}})));
    assert_eq!(undone["project"]["boards"][0]["tracks"][0]["locked"], false);
}
