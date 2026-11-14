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

/// Build a meaningful unnamed network without persistent records.
fn circuit(count: usize) -> Session {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({}));
    for index in 0..count {
        apply(&mut session, PLACE_RESISTOR, json!({"x_nm":index as i64*25_400_000,"y_nm":0}));
    }
    apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}}));
    session
}

/// Retain only active identity records when comparing independently captured resolution history.
fn active(report: &Value, id: &Value) -> Value {
    report["project"]["net_registry"]["records"].as_array().unwrap().iter().find(|record| &record["id"] == id).unwrap().clone()
}

/// Both adapters discover and execute promotion, reject stale authority/modal ownership and
/// inspect without allocation. Undo uses the same authored history rather than a separate API.
#[test]
fn promotion_is_discoverable_and_shared_by_headless_and_live_clients() {
    let setup = circuit(2);
    let snapshot = setup.snapshot().unwrap();
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
    let params = json!({"command":nets::PROMOTE,"params":{"anchor":{"symbol":"symbol-2","terminal":"1"},"expected_revision":revision}});
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
    let mut setup = circuit(2);
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
