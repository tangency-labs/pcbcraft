//! Headless/live parity for the authored persistent-identity commands and their revision policy.

use super::{Backend, Headless};
use crate::live::{Editor, call_editor};
use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, Session, nets, selection::Selection};
use serde_json::{Value, json};
use std::sync::Arc;

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
