//! Headless/live parity for the authored persistent-identity commands and their revision policy.

use super::{Backend, Headless};
use crate::live::{Editor, call_editor};
use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, Session, nets, selection::Selection};
use serde_json::{Value, json};
use std::sync::Arc;

/// Catalog edits and explicit-class reads share live/headless authority, including modal refusal,
/// query results with inspection disabled, stale revisions and the editor's ordinary Undo history.
#[test]
fn net_class_commands_share_live_and_headless_revision_and_query_behavior() {
    use pcbcraft_engine::net_classes as classes;
    let mut setup = Session::default();
    apply(&mut setup, "project.new", json!({}));
    apply(&mut setup, classes::CREATE, json!({"name":"Power","rules":{"clearance_nm":500_000}}));
    let id = setup.project().unwrap().net_classes().definitions[0].id;
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live_session = Session::default();
    live_session.open_snapshot(snapshot.clone());
    let mut editor = OpenEditor { session: live_session, blocked: false };
    let catalog = headless.call("engine.commands", json!({})).unwrap();
    for command in [classes::CREATE, classes::UPDATE, classes::REMOVE, classes::INSPECT, classes::RESOLVE] {
        assert!(catalog.as_array().unwrap().iter().any(|entry| entry["id"] == command));
    }
    let revision = editor.session.revision();
    let edit = json!({"command":classes::UPDATE,"inspect":false,"params":{"class":id,"name":"Supply","rules":{"clearance_nm":700_000},"expected_revision":revision}});
    editor.blocked = true;
    assert!(call_editor(&mut editor, &json!({"method":"engine.execute","params":edit})).unwrap_err().contains("dialog"));
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    editor.blocked = false;
    let live = call_editor(&mut editor, &json!({"method":"engine.execute","params":edit})).unwrap();
    assert_eq!(live, headless.call("engine.execute", edit.clone()).unwrap());
    assert!(call_editor(&mut editor, &json!({"method":"engine.execute","params":edit})).is_err());
    assert!(headless.call("engine.execute", edit).is_err());
    let saved = editor.session.snapshot().unwrap();
    for params in [
        json!({"command":classes::INSPECT,"inspect":false,"params":{}}),
        json!({"command":classes::RESOLVE,"inspect":false,"params":{"classes":[id],"other_classes":[]}}),
    ] {
        let live = call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap();
        assert_eq!(live, headless.call("engine.execute", params).unwrap());
        assert!(live.get("net_classes").is_some() || live.get("resolution").is_some());
    }
    assert!(Arc::ptr_eq(&saved, &editor.session.snapshot().unwrap()));
    call_editor(&mut editor, &json!({"method":"engine.execute","params":{"command":"edit.undo","params":{}}})).unwrap();
    assert_eq!(editor.session.project().unwrap().net_classes(), snapshot.net_classes());
    assert!(editor.session.project().unwrap().board_target().is_none());
}

/// Part-group edits and membership reads behave the same in the live editor and headless,
/// including queries with inspection disabled and the editor's ordinary Undo history.
#[test]
fn part_group_commands_share_live_and_headless_behavior() {
    use pcbcraft_engine::part_groups as groups;
    let mut setup = circuit(2);
    let part_instance = setup.project().unwrap().part_instance_addresses().next().unwrap();
    apply(&mut setup, groups::CREATE, json!({"name":"Power"}));
    let group = setup.project().unwrap().part_groups().groups[0].id;
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut session = Session::default();
    session.open_snapshot(snapshot.clone());
    let mut editor = OpenEditor { session, blocked: false };
    let catalog = headless.call("engine.commands", json!({})).unwrap();
    for command in [groups::CREATE, groups::UPDATE, groups::REMOVE, groups::INSPECT, groups::SET_GROUPS, groups::INSPECT_GROUPS] {
        assert!(catalog.as_array().unwrap().iter().any(|entry| entry["id"] == command));
    }
    let run = |editor: &mut OpenEditor, headless: &mut Headless, request: Value| {
        let live = call_editor(editor, &json!({"method":"engine.execute","params":request})).unwrap();
        assert_eq!(live, headless.call("engine.execute", request).unwrap());
        live
    };
    let report = run(&mut editor, &mut headless, json!({"command":groups::INSPECT,"inspect":false,"params":{}}));
    assert!(report.get("project").is_none());
    assert_eq!(report["part_groups"]["groups"][0]["id"], json!(group));
    let revision = editor.session.revision();
    let edit =
        json!({"command":groups::SET_GROUPS,"inspect":false,"params":{"part_instance":part_instance,"groups":[group],"expected_revision":revision}});
    run(&mut editor, &mut headless, edit);
    let query = json!({"command":groups::INSPECT_GROUPS,"inspect":false,"params":{}});
    let report = run(&mut editor, &mut headless, query);
    assert_eq!(report["part_instance_memberships"], json!([{"part_instance":part_instance,"groups":[group]}]));
    call_editor(&mut editor, &json!({"method":"engine.execute","params":{"command":"edit.undo","params":{}}})).unwrap();
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(editor.session.snapshot().unwrap()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(snapshot).unwrap()
    );
}

/// Model the real live adapter's ownership/refusal boundary without opening an OS window.
struct OpenEditor {
    session: Session,
    blocked: bool,
}
impl Editor for OpenEditor {
    /// Live commands share this exact session and undo history.
    fn session(&mut self) -> &mut Session {
        &mut self.session
    }
    /// A modal owner prevents authored commands before dispatch.
    fn refusal(&self, _: &str) -> Option<String> {
        self.blocked.then(|| "finish the dialog".into())
    }
    /// This fixture exposes document commands only.
    fn window_commands(&self) -> Vec<Value> {
        vec![]
    }
    /// Window presentation is independent from document inspection.
    fn inspect(&self) -> Value {
        json!({})
    }
    /// No document command may fall through to a window handler.
    fn run_window_command(&mut self, _: &str, _: &Value) -> Result<(), String> {
        Err("not a window command".into())
    }
}

/// Keep original fixture authoring on the ordinary shared command path.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    if command != "project.new" {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// Build a meaningful unnamed network through the ordinary numbered authoring path.
fn circuit(count: usize) -> Session {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({}));
    for index in 0..count {
        apply(&mut session, PLACE_RESISTOR, json!({"x_nm":index as i64*25_400_000,"y_nm":0}));
    }
    apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}}));
    session
}

/// Load a production historical fixture without persistent net IDs. This keeps the legacy
/// promotion paths exercised after newly authored networks began owning numbered identities.
fn legacy_circuit() -> Arc<pcbcraft_engine::Project> {
    let project = pcbcraft_engine::persistence::decode(include_bytes!("../../app/tests/fixtures/two-resistors.pc")).unwrap();
    Arc::new(project.with_board_folders(".").unwrap().with_sheet_occurrences().unwrap())
}

/// Live and headless clients keep the same saved number and UUID when a representative
/// component is deleted from a surviving net; querying cannot substitute another pad name.
#[test]
fn numbered_net_deletion_keeps_live_and_headless_identity_and_name() {
    let mut setup = circuit(3);
    apply(&mut setup, CONNECT_PINS, json!({"from":{"symbol":"symbol-3","terminal":"1"},"to":{"symbol":"symbol-4","terminal":"1"}}));
    let project = setup.snapshot().unwrap();
    let original = setup.inspect()["project"]["net_registry"]["records"][0].clone();
    let mut headless = Headless::with_project(project.clone());
    let mut session = Session::default();
    session.open_snapshot(project);
    let mut editor = OpenEditor { session, blocked: false };
    let edit = json!({"command":pcbcraft_engine::DELETE_SYMBOL,"params":{"symbol":"symbol-2","expected_revision":editor.session.revision()}});
    let live = call_editor(&mut editor, &json!({"method":"engine.execute","params":edit})).unwrap();
    let remote = headless.call("engine.execute", edit).unwrap();
    assert_eq!(live, remote);
    assert_eq!(live["project"]["net_registry"]["records"][0]["id"], original["id"]);
    assert_eq!(live["project"]["net_registry"]["records"][0]["generated_number"], original["generated_number"]);
    assert_eq!(live["project"]["schematic"]["netlist"]["nets"][0]["name"], "Net-0001");
}

/// Retain only active identity records when comparing independently captured resolution history.
fn active(report: &Value, id: &Value) -> Value {
    report["project"]["net_registry"]["records"].as_array().unwrap().iter().find(|record| &record["id"] == id).unwrap().clone()
}

/// Both adapters discover and execute promotion, reject stale authority/modal ownership and
/// inspect without allocation. Undo uses the same authored history rather than a separate API.
#[test]
fn promotion_is_discoverable_and_shared_by_headless_and_live_clients() {
    let snapshot = legacy_circuit();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut session = Session::default();
    session.open_snapshot(snapshot);
    let mut editor = OpenEditor { session, blocked: false };
    let catalog = headless.call("engine.commands", json!({})).unwrap();
    for command in [nets::PROMOTE, nets::RESOLVE_AMBIGUITY] {
        let entry = catalog.as_array().unwrap().iter().find(|entry| entry["id"] == command).unwrap();
        assert!(entry["params"]["required"].as_array().unwrap().contains(&json!("expected_revision")));
    }
    let before = editor.session.snapshot().unwrap();
    let inspect = headless.call("project.inspect", json!({})).unwrap();
    assert!(inspect["project"].get("net_registry").is_none());
    let revision = inspect["revision"].clone();
    let params = json!({"command":nets::PROMOTE,"params":{"anchor":{"symbol":"symbol-2","terminal":"2"},"expected_revision":revision}});
    editor.blocked = true;
    assert!(call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap_err().contains("dialog"));
    assert!(Arc::ptr_eq(&before, &editor.session.snapshot().unwrap()));
    editor.blocked = false;
    let live = call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap();
    let headless = headless.call("engine.execute", params).unwrap();
    let live_record = &live["project"]["net_registry"]["records"][0];
    let headless_record = &headless["project"]["net_registry"]["records"][0];
    assert_ne!(live_record["id"], headless_record["id"]);
    assert_eq!(live_record["primary"], headless_record["primary"]);
    assert_eq!(live_record["fallbacks"], headless_record["fallbacks"]);
    let saved = editor.session.snapshot().unwrap();
    assert!(call_editor(&mut editor,&json!({"method":"engine.execute","params":{"command":nets::PROMOTE,"params":{"anchor":live_record["primary"],"expected_revision":revision}}})).is_err());
    assert!(Arc::ptr_eq(&saved, &editor.session.snapshot().unwrap()));
    call_editor(&mut editor, &json!({"method":"engine.execute","params":{"command":"edit.undo","params":{}}})).unwrap();
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(editor.session.snapshot().unwrap()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(before).unwrap()
    );
}

/// Both adapters resolve the same recorded ambiguity, preserving its UUID and validating an
/// inspected typed branch anchor. Separate captures may differ while live authority agrees.
#[test]
fn ambiguity_resolution_is_available_in_both_mcp_modes() {
    let mut setup = circuit(6);
    apply(&mut setup, nets::PROMOTE, json!({"anchor":{"symbol":"symbol-2","terminal":"1"}}));
    let source = setup.inspect()["project"]["net_registry"]["records"][0]["id"].clone();
    for (a, b) in [(2, 4), (4, 5), (2, 6), (6, 7)] {
        apply(
            &mut setup,
            CONNECT_PINS,
            json!({"from":{"symbol":format!("symbol-{a}"),"terminal":"1"},"to":{"symbol":format!("symbol-{b}"),"terminal":"1"}}),
        );
    }
    let selection = Selection { symbols: ["symbol-2".into(), "symbol-3".into()].into(), ..Default::default() };
    apply(&mut setup, pcbcraft_engine::DELETE_ITEMS, json!({"items":selection}));
    assert_eq!(active(&setup.inspect(), &source)["state"]["kind"], "ambiguous");
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut session = Session::default();
    session.open_snapshot(snapshot);
    let mut editor = OpenEditor { session, blocked: false };
    let before = headless.call("project.inspect", json!({})).unwrap();
    let branch =
        before["project"]["net_registry"]["nets"].as_array().unwrap().iter().find(|net| net["anchor"]["object"] == 4).unwrap()["anchor"].clone();
    let params = json!({"command":nets::RESOLVE_AMBIGUITY,"params":{"net":source,"anchor":branch,"expected_revision":before["revision"]}});
    let live = call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap();
    let headless = headless.call("engine.execute", params).unwrap();
    assert_eq!(active(&live, &source), active(&headless, &source));
    assert_eq!(active(&live, &source)["state"]["kind"], "active");
}

/// Assignment requests share scoped authority and origin-preserving history in both adapters;
/// query payloads survive inspect:false and rejected stale/foreign requests publish nothing.
#[test]
fn class_assignments_share_live_headless_targets_queries_and_rejection() {
    use pcbcraft_engine::net_classes;
    let mut setup = circuit(2);
    apply(&mut setup, net_classes::CREATE, json!({"name":"Power","rules":{"clearance_nm":500000}}));
    apply(&mut setup, nets::PROMOTE, json!({"anchor":{"symbol":"symbol-2","terminal":"1"}}));
    let project = setup.snapshot().unwrap();
    let class = project.net_classes().definitions[0].id;
    let target = json!({"kind":"derived","anchor":project.inspect_net_registry().unwrap()["nets"][0]["anchor"]});
    let mut headless = Headless::with_project(project.clone());
    let mut session = Session::default();
    session.open_snapshot(project.clone());
    let mut live = OpenEditor { session, blocked: false };
    let catalog = headless.call("engine.commands", json!({})).unwrap();
    for id in [nets::SET_CLASSES, nets::INSPECT_CLASSES] {
        assert!(catalog.as_array().unwrap().iter().any(|command| command["id"] == id));
    }
    let request =
        json!({"command":nets::SET_CLASSES,"inspect":false,"params":{"target":target,"classes":[class],"expected_revision":live.session.revision()}});
    live.blocked = true;
    assert!(call_editor(&mut live, &json!({"method":"engine.execute","params":request})).is_err());
    assert!(Arc::ptr_eq(&project, &live.session.snapshot().unwrap()));
    live.blocked = false;
    let a = call_editor(&mut live, &json!({"method":"engine.execute","params":request})).unwrap();
    assert_eq!(a, headless.call("engine.execute", request.clone()).unwrap());
    let assigned = live.session.snapshot().unwrap();
    assert!(headless.call("engine.execute", request.clone()).is_err());
    assert!(call_editor(&mut live, &json!({"method":"engine.execute","params":request})).is_err());
    assert!(Arc::ptr_eq(&assigned, &live.session.snapshot().unwrap()));
    let query = json!({"command":nets::INSPECT_CLASSES,"inspect":false,"params":{"target":target}});
    let a = call_editor(&mut live, &json!({"method":"engine.execute","params":query})).unwrap();
    assert_eq!(a, headless.call("engine.execute", query).unwrap());
    assert_eq!(a["net_assignments"]["status"], "ready");
    assert!(a.get("project").is_none());
    call_editor(&mut live, &json!({"method":"engine.execute","params":{"command":"edit.undo","params":{}}})).unwrap();
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(live.session.snapshot().unwrap()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(project).unwrap()
    );
}

/// Read-only discovery supplies typed derived anchors before any persistent ID exists. Both
/// hosts then promote and bind in one transaction, without a separate promotion history step.
#[test]
fn assignment_discovery_promotes_derived_targets_in_one_shared_transaction() {
    let mut setup = Session::default();
    setup.open_snapshot(legacy_circuit());
    apply(&mut setup, pcbcraft_engine::net_classes::CREATE, json!({"name":"Power","rules":{}}));
    let p = setup.snapshot().unwrap();
    let class = p.net_classes().definitions[0].id;
    let mut headless = Headless::with_project(p.clone());
    let mut session = Session::default();
    session.open_snapshot(p.clone());
    let mut live = OpenEditor { session, blocked: false };
    let query = json!({"command":nets::INSPECT_CLASSES,"inspect":false,"params":{}});
    let before = live.session.snapshot().unwrap();
    let revision = live.session.revision();
    let report = call_editor(&mut live, &json!({"method":"engine.execute","params":query})).unwrap();
    assert_eq!(report, headless.call("engine.execute", query).unwrap());
    assert_eq!(report["net_assignments"]["records"], json!([]));
    assert!(Arc::ptr_eq(&before, &live.session.snapshot().unwrap()));
    let target = json!({"kind":"derived","anchor":report["net_assignments"]["nets"][0]["anchor"]});
    let edit = json!({"command":nets::SET_CLASSES,"inspect":false,"params":{"target":target,"classes":[class],"expected_revision":revision}});
    call_editor(&mut live, &json!({"method":"engine.execute","params":edit})).unwrap();
    headless.call("engine.execute", edit).unwrap();
    assert_eq!(live.session.revision(), revision + 1);
    let query = json!({"command":nets::INSPECT_CLASSES,"inspect":false,"params":{}});
    for report in
        [call_editor(&mut live, &json!({"method":"engine.execute","params":query})).unwrap(), headless.call("engine.execute", query).unwrap()]
    {
        let records = report["net_assignments"]["records"].as_array().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["class_bindings"][0]["class"], json!(class));
        assert_eq!(records[0]["class_bindings"][0]["origin"], records[0]["id"]);
    }
    call_editor(&mut live, &json!({"method":"engine.execute","params":{"command":"edit.undo","params":{}}})).unwrap();
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(live.session.snapshot().unwrap()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(p).unwrap()
    );
}

/// Scoped routing inspection and class-aware edits use the same live/headless transaction path,
/// retain read results with inspection disabled and reject stale or weakened dimensions atomically.
#[test]
fn class_routing_queries_and_commits_have_live_headless_parity() {
    use pcbcraft_engine::{
        board::{
            class_rules::ROUTE_RULES,
            setup::{LayerPurpose, Side},
        },
        net_classes,
    };
    let mut setup = Session::default();
    apply(&mut setup, "project.new", json!({"create_board":true}));
    apply(&mut setup, pcbcraft_engine::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    for index in 0..2 {
        apply(&mut setup, PLACE_RESISTOR, json!({"x_nm":index*10000000,"y_nm":0}));
        let symbol = setup.project().unwrap().symbols().last().unwrap().id();
        apply(
            &mut setup,
            pcbcraft_engine::PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":index*10000000,"y_nm":0}),
        );
    }
    let symbols: Vec<_> = setup.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    apply(&mut setup, CONNECT_PINS, json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"}}));
    apply(&mut setup, pcbcraft_engine::INITIALIZE_UPDATE_BASELINE, json!({}));
    apply(
        &mut setup,
        net_classes::CREATE,
        json!({"name":"Power","rules":{"clearance_nm":500000,"min_track_width_nm":400000,"preferred_track_width_nm":500000}}),
    );
    let class = setup.project().unwrap().net_classes().definitions[0].id;
    let anchor = setup.execute(nets::INSPECT_CLASSES, &json!({})).unwrap()["net_assignments"]["nets"][0]["anchor"].clone();
    apply(&mut setup, nets::SET_CLASSES, json!({"target":{"kind":"derived","anchor":anchor},"classes":[class]}));
    let project = setup.snapshot().unwrap();
    let board = project.board_target().unwrap();
    let view = project.board_view(board).unwrap();
    let at = view.footprints().next().unwrap().pads.iter().find(|pad| pad.number == "1").unwrap().center;
    let layer = view.layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Front)).unwrap().id;
    let mut headless = Headless::with_project(project.clone());
    let mut live_session = Session::default();
    live_session.open_snapshot(project.clone());
    let mut live = OpenEditor { session: live_session, blocked: false };
    let revision = live.session.revision();
    let query = json!({"command":ROUTE_RULES,"inspect":false,"params":{"board":board.board,"at":at,"layer":layer}});
    let report = call_editor(&mut live, &json!({"method":"engine.execute","params":query})).unwrap();
    assert_eq!(report, headless.call("engine.execute", query.clone()).unwrap());
    assert_eq!(report["route_rules"]["applied"]["resolution"]["values"]["preferred_track_width_nm"], 500000);
    assert!(Arc::ptr_eq(&project, &live.session.snapshot().unwrap()));
    assert_eq!(live.session.revision(), revision);
    let edit = json!({"command":pcbcraft_engine::ADD_TRACKS,"inspect":false,"params":{"board":board.board,"layer":layer,"points":[at,{"x_nm":at.x_nm,"y_nm":at.y_nm+2000000}],"expected_revision":revision}});
    live.blocked = true;
    assert!(call_editor(&mut live, &json!({"method":"engine.execute","params":edit})).is_err());
    assert!(Arc::ptr_eq(&project, &live.session.snapshot().unwrap()));
    live.blocked = false;
    assert_eq!(
        call_editor(&mut live, &json!({"method":"engine.execute","params":edit})).unwrap(),
        headless.call("engine.execute", edit.clone()).unwrap()
    );
    assert!(call_editor(&mut live, &json!({"method":"engine.execute","params":edit})).is_err());
    assert!(headless.call("engine.execute", edit).is_err());
    assert!(live.session.project().unwrap().board_view(board).unwrap().tracks().iter().all(|track| track.width_nm == 500000));
    let remote = headless.call("project.inspect", json!({})).unwrap();
    assert_eq!(remote["project"]["boards"][0]["tracks"][0]["width_nm"], 500000);
    let before = live.session.snapshot().unwrap();
    let weak = json!({"command":pcbcraft_engine::ADD_TRACKS,"inspect":false,"params":{"board":board.board,"layer":layer,"points":[at,{"x_nm":at.x_nm,"y_nm":at.y_nm-2000000}],"width_nm":200000,"expected_revision":live.session.revision()}});
    assert!(call_editor(&mut live, &json!({"method":"engine.execute","params":weak})).is_err());
    assert!(headless.call("engine.execute", weak).is_err());
    assert!(Arc::ptr_eq(&before, &live.session.snapshot().unwrap()));
    let undo = json!({"command":"edit.undo","inspect":false,"params":{}});
    assert_eq!(call_editor(&mut live, &json!({"method":"engine.execute","params":undo})).unwrap(), headless.call("engine.execute", undo).unwrap());
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(live.session.snapshot().unwrap()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(project).unwrap()
    );
}
