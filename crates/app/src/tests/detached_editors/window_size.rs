//! Native size requests for detached editor windows.
use super::*;

/// A detached window's size request goes to its own viewport. While its compositor keeps
/// the old size the request stays pending beside the measured size; a closed window refuses both.
#[test]
fn detached_window_resize_targets_its_own_viewport_and_closed_windows_refuse() {
    let (mut h, peer) = multi_window();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.open_window", &json!({"view":root.0})).unwrap();
    h.state_mut().automation.observing = true;
    peer.borrow_mut().focused = true;
    h.run_steps(4);
    let resize = json!({"window":1,"width":1280.0,"height":800.0});
    assert!(h.state_mut().automation_command(&ctx, "ui.resize", &json!({"window":1,"width":400.0,"height":800.0})).unwrap_err().contains("480×400"));
    assert_eq!(h.state_mut().automation_command(&ctx, "ui.resize", &resize).unwrap()["state"], "pending");
    h.step();
    assert!(h.output().viewport_output[&editor_windows::viewport(HostId(1))].commands.contains(&ViewportCommand::InnerSize(vec2(1280.0, 800.0))));
    assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.iter().any(|command| matches!(command, ViewportCommand::InnerSize(_))));
    h.run_steps(2);
    let state = h.state_mut().automation_command(&ctx, "ui.window", &json!({"window":1})).unwrap();
    assert_eq!(state["resize"]["state"], "pending", "{state}");
    assert_eq!(state["client"], json!({"width":1100.0,"height":760.0}));
    peer.borrow_mut().close = true;
    h.run_steps(3);
    assert_eq!(h.state().views.hosts.len(), 1);
    assert!(h.state_mut().automation_command(&ctx, "ui.window", &json!({"window":1})).unwrap_err().contains("Unknown editor window"));
    assert!(h.state_mut().automation_command(&ctx, "ui.resize", &resize).unwrap_err().contains("Unknown editor window"));
}
