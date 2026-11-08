use super::*;
use pcbcraft_engine::{DRAW_WIRE, PLACE_SYMBOL, Session};
use serde_json::{Value, json};

/// Execute a fixture edit at the current revision without hand-editing native documents.
fn run(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Cached targets must follow edits, undo and a reopened snapshot; camera and viewport
/// changes move the target immediately. Changing only the grid still uses current snapping.
#[test]
fn cached_targets_follow_history_reopening_navigation_and_grid_changes() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Targets"})).unwrap();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let free = s.snapshot().unwrap();
    let pin = WireEndpoint::Terminal(free.symbols()[0].terminals()[1]);
    let point = free.endpoint_position(pin).unwrap();
    let mut camera = Camera::default();
    let mut cache = Cache::default();
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let pos = camera.screen(point, rect);
    assert_eq!(cache.get(s.project().unwrap(), s.revision(), &camera, rect).automatic_node(pos), Some(Anchor::Existing(pin)));
    run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":{"x_nm":25400000,"y_nm":12700000},"bends":[{"x_nm":25400000,"y_nm":0}]}));
    let targets = cache.get(s.project().unwrap(), s.revision(), &camera, rect);
    assert_eq!(targets.automatic_node(pos), None);
    assert_eq!(targets.anchor(&camera, rect, pos, None, true), Some(Anchor::Existing(pin)));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(cache.get(s.project().unwrap(), s.revision(), &camera, rect).automatic_node(pos), Some(Anchor::Existing(pin)));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(cache.get(s.project().unwrap(), s.revision(), &camera, rect).automatic_node(pos), None);
    s.open_snapshot(free);
    assert_eq!(cache.get(s.project().unwrap(), s.revision(), &camera, rect).automatic_node(pos), Some(Anchor::Existing(pin)));
    camera.center_on(Point { x_nm: 12700000, y_nm: 0 });
    assert_eq!(cache.get(s.project().unwrap(), s.revision(), &camera, rect).automatic_node(camera.screen(point, rect)), Some(Anchor::Existing(pin)));
    let resized = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0));
    assert_eq!(
        cache.get(s.project().unwrap(), s.revision(), &camera, resized).automatic_node(camera.screen(point, resized)),
        Some(Anchor::Existing(pin))
    );
    let mut state = serde_json::to_value(camera.saved_state()).unwrap();
    state["zoom"] = json!(1.5);
    camera = Camera::from_state(&serde_json::from_value(state).unwrap());
    assert_eq!(
        cache.get(s.project().unwrap(), s.revision(), &camera, resized).automatic_node(camera.screen(point, resized)),
        Some(Anchor::Existing(pin))
    );
    assert!(camera.set_snap_nm(500_000));
    let pointer = Pos2::new(900.0, 620.0);
    assert_eq!(
        cache.get(s.project().unwrap(), s.revision(), &camera, resized).anchor(&camera, resized, pointer, None, true),
        Some(Anchor::Free(camera.snap(pointer, resized)))
    );
}

/// Coincident pins and named markers form one deliberate target. A label contacting an
/// occupied route requires W, while the explicit target remains available for wiring.
#[test]
fn named_contact_is_unambiguous_and_occupied_routes_keep_selection_priority() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    let point = s.project().unwrap().terminal_position(pin).unwrap();
    run(&mut s, pcbcraft_engine::net_labels::ADD, json!({"name":"GND","style":"ground","x_nm":point.x_nm,"y_nm":point.y_nm}));
    let end = WireEndpoint::NetLabel(s.project().unwrap().net_labels()[0].id);
    let camera = Camera::default();
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000., 700.));
    let pos = camera.screen(point, rect);
    let targets = Targets::new(s.project().unwrap(), &camera, rect);
    assert_eq!(targets.automatic_node(pos), Some(Anchor::Existing(end)));
    assert_eq!(targets.anchor(&camera, rect, pos, None, true), Some(Anchor::Existing(end)));
    run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":{"x_nm":-25_400_000,"y_nm":0},"bends":[]}));
    let targets = Targets::new(s.project().unwrap(), &camera, rect);
    assert_eq!(targets.automatic_node(pos), None);
    assert_eq!(targets.anchor(&camera, rect, pos, None, true), Some(Anchor::Existing(end)));
}
