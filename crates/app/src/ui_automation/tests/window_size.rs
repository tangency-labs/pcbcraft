//! Measured window sizes and native size requests for recordings.
use super::*;

/// Paint until the window's latest size request is applied or constrained, as a client polls.
fn settle(h: &mut Harness<'_, PcbApp>, window: u64) -> Value {
    for _ in 0..20 {
        h.step();
        let state = command(h, "ui.window", json!({"window":window})).unwrap();
        if state["resize"]["state"] != "pending" {
            return state;
        }
    }
    panic!("Window resize did not settle");
}

/// Recording sizes reach the existing host, are reported from the measured paint, retire the
/// old layout token, and leave the project, views and history as they were.
#[test]
fn recording_sizes_are_applied_and_measured_without_changing_the_project() {
    let mut h = ready();
    let before = (h.state().session.inspect(), h.state().session.revision(), h.state().session.history_root_revision());
    let views = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let state = command(&mut h, "ui.window", json!({"window":0})).unwrap();
    assert_eq!(state["client"], json!({"width":1600.0,"height":1000.0}), "{state}");
    assert!(state["resize"].is_null(), "{state}");
    for (width, height) in [(1280.0, 800.0), (1920.0, 1080.0)] {
        let old = command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap();
        let requested = command(&mut h, "ui.resize", json!({"window":0,"width":width,"height":height})).unwrap();
        assert_eq!(requested["state"], "pending");
        assert!(requested.get("client").is_none(), "a request is not a measurement: {requested}");
        let pending = command(&mut h, "ui.window", json!({"window":0})).unwrap();
        assert_ne!(pending["client"], json!({"width":width,"height":height}), "{pending}");
        assert_eq!(pending["resize"]["requested"], json!({"width":width,"height":height}));
        assert!(command(&mut h, "ui.key", json!({"window":0,"key":"W","layout":old["layout"]})).unwrap_err().contains("stale"));
        assert!(command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap_err().contains("stale"));
        let state = settle(&mut h, 0);
        assert_eq!(state["resize"]["state"], "applied", "{state}");
        assert_eq!(state["client"], json!({"width":width,"height":height}), "{state}");
        assert_ne!(state["layout"], old["layout"]);
        let at = command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap();
        assert_eq!(at["layout"], state["layout"]);
        assert!(at["x"].as_f64().unwrap() < width && at["y"].as_f64().unwrap() < height, "{at}");
        assert!(command(&mut h, "ui.key", json!({"window":0,"key":"W","layout":old["layout"]})).unwrap_err().contains("stale"));
    }
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"}})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { .. })));
    assert_eq!((h.state().session.inspect(), h.state().session.revision(), h.state().session.history_root_revision()), before);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), views);
}

/// A compositor that keeps another size reports `constrained` with the measured size.
/// Requests the window cannot honor are refused without sending anything.
#[test]
fn constrained_and_refused_resizes_are_reported_separately() {
    let mut h = ready();
    command(&mut h, "ui.window", json!({"window":0})).unwrap();
    command(&mut h, "ui.resize", json!({"window":0,"width":1280.0,"height":800.0})).unwrap();
    h.step();
    h.set_size(egui::vec2(1200.0, 800.0));
    h.state_mut().automation.resizes.get_mut(&HostId::ROOT).unwrap().since = Instant::now() - super::super::window::RESIZE_SETTLE;
    let state = settle(&mut h, 0);
    assert_eq!(state["resize"]["state"], "constrained", "{state}");
    assert_eq!(state["client"], json!({"width":1200.0,"height":800.0}));
    assert_eq!(state["resize"]["requested"], json!({"width":1280.0,"height":800.0}));
    for (params, reason) in [
        (json!({"window":9,"width":1280.0,"height":800.0}), "Unknown editor window"),
        (json!({"window":0,"width":320.0,"height":800.0}), "at least 640×400"),
        (json!({"window":0,"width":1280.0,"height":20000.0}), "exceeds"),
        (json!({"window":0,"width":1280.0}), "height"),
    ] {
        assert!(command(&mut h, "ui.resize", params.clone()).unwrap_err().contains(reason), "{params}");
    }
    assert!(command(&mut h, "ui.window", json!({"window":9})).unwrap_err().contains("Unknown editor window"));
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().maximized = Some(true);
    h.step();
    // macOS reports this flag only when the window opens, so a later value is unknown.
    let maximized = if cfg!(target_os = "macos") { Value::Null } else { json!(true) };
    assert_eq!(command(&mut h, "ui.window", json!({"window":0})).unwrap()["maximized"], maximized);
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().fullscreen = Some(true);
    h.step();
    assert!(command(&mut h, "ui.resize", json!({"window":0,"width":1280.0,"height":800.0})).unwrap_err().contains("fullscreen"));
    assert_eq!(command(&mut h, "ui.window", json!({"window":0})).unwrap()["resize"]["state"], "constrained");
}

/// A resize takes the window away from queued input: it is cancelled, not replayed at new
/// geometry, and the old layout token is retired even when the size does not change.
#[test]
fn resizing_cancels_queued_input_and_retires_the_layout() {
    let mut h = ready();
    let old = command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap();
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"}})).unwrap();
    command(&mut h, "ui.resize", json!({"window":0,"width":1600.0,"height":1000.0})).unwrap();
    let status = command(&mut h, "ui.inspect", json!({})).unwrap();
    assert_eq!(status["operation"]["state"], "cancelled", "{status}");
    assert_eq!(status["operation"]["reason"], "Window resize requested");
    assert_eq!(settle(&mut h, 0)["resize"]["state"], "applied");
    assert!(h.state().views.active.schematic.tool.is_none());
    let at = command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap();
    assert_eq!((&at["x"], &at["y"]), (&old["x"], &old["y"]));
    assert_ne!(at["layout"], old["layout"]);
}
