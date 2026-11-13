//! Real editor interactions, queue authority and authenticated MCP discovery.
use super::*;
use egui_kittest::Harness;

#[cfg(target_os = "linux")]
mod native;

/// Use the ordinary resistor fixture and an explicitly enabled local endpoint.
fn ready() -> Harness<'static, PcbApp> {
    let mut h = crate::tests::harness();
    h.set_size(egui::vec2(1600.0, 1000.0));
    crate::tests::resistor_at_origin(&mut h);
    let ctx = h.ctx.clone();
    h.state_mut().live_control = Some(crate::live_control::LiveControl::start(&ctx).unwrap());
    h.state_mut().automation.observing = true;
    h.run();
    h
}

/// Exercise the production dispatcher; production callers attach the same endpoint authority.
fn command(h: &mut Harness<'_, PcbApp>, id: &str, params: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    let result = pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx },
        &json!({"method":"engine.execute","params":{"command":id,"params":params}}),
    );
    if let Some(live) = &h.state().live_control {
        let owner = live.automation_owner(false, live.current());
        h.state_mut().automation.authorize(owner);
    }
    result
}

/// The headless harness does not call eframe's raw hook; use that identical hook before a pass.
fn tick(h: &mut Harness<'_, PcbApp>) {
    let ctx = h.ctx.clone();
    let mut input = std::mem::take(h.input_mut());
    h.state_mut().automation_input(&ctx, &mut input);
    *h.input_mut() = input;
    h.step();
}

/// Stop after the bounded queue retires; an unfinished operation is a failed test.
fn finish(h: &mut Harness<'_, PcbApp>) -> Value {
    for _ in 0..130 {
        tick(h);
        if h.state().automation.job.is_none() && h.state().automation.release.is_none() {
            return h.state().automation.status();
        }
    }
    panic!("UI input did not finish");
}

/// Find an original symbol through its camera, not a hardcoded pixel fixture.
fn symbol(h: &Harness<'_, PcbApp>) -> Pos2 {
    let view = &h.state().views.active;
    view.camera.screen(h.state().session.project().unwrap().symbols()[0].position, view.camera.schematic_rect().unwrap())
}

#[test]
fn named_ribbon_click_and_keyboard_follow_real_editor_tools() {
    let mut h = ready();
    let before = h.state().session.inspect();
    let handles = command(&mut h, "ui.handles", json!({"window":0,"prefix":"toolbar:Connect:"})).unwrap();
    assert!(handles["handles"].as_array().unwrap().iter().any(|t| t["name"] == "toolbar:Connect:Wire" && t["enabled"] == true), "{handles}");
    let at = command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap();
    assert!(at["x"].as_f64().unwrap() > 0.0);
    assert!(command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Bus"})).unwrap_err().contains("disabled"));
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"}})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { .. })));
    command(&mut h, "ui.key", json!({"window":0,"key":"Escape"})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(h.state().session.inspect(), before);
}

#[test]
fn pointer_drag_commits_one_normal_edit_and_undo_restores_the_symbol() {
    let mut h = ready();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let from = symbol(&h);
    command(&mut h, "ui.drag", json!({"window":0,"from":{"x":from.x,"y":from.y},"to":{"x":from.x+40.64,"y":from.y},"steps":4})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert_ne!(h.state().session.project().unwrap().symbols()[0].position, before.symbols()[0].position);
    assert_eq!(h.state().session.revision(), revision + 1);
    h.state_mut().session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(h.state().session.project().unwrap().symbols(), before.symbols());
}

#[test]
fn text_and_command_modifier_edit_the_real_modal_field() {
    let mut h = ready();
    h.state_mut().dialog = crate::Dialog::Name { command: "project.rename", text: "Old".into(), error: None, focused: false };
    h.run();
    let revision = h.state().session.revision();
    command(&mut h, "ui.key", json!({"window":0,"key":"A","modifiers":{"command":true}})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    command(&mut h, "ui.text", json!({"window":0,"text":"Recorded circuit"})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"button:OK"}})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert_eq!(h.state().session.project().unwrap().name, "Recorded circuit");
    assert_eq!(h.state().session.revision(), revision + 1);
}

#[test]
fn stale_resize_focus_disable_timeout_and_physical_input_cancel_pending_events() {
    for reason in ["stale", "resize", "focus", "disable", "replacement", "generation", "timeout", "physical", "leave"] {
        let mut h = ready();
        let from = symbol(&h);
        command(&mut h, "ui.drag", json!({"window":0,"from":{"x":from.x,"y":from.y},"to":{"x":from.x+40.64,"y":from.y},"steps":8})).unwrap();
        tick(&mut h);
        tick(&mut h);
        tick(&mut h);
        let before = h.state().session.inspect();
        match reason {
            "stale" => {
                h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
            }
            "resize" => {
                h.set_size(egui::vec2(1500.0, 900.0));
            }
            "focus" => h.input_mut().focused = false,
            "disable" => h.state_mut().live_control = None,
            "replacement" => {
                let ctx = h.ctx.clone();
                h.state_mut().live_control = Some(crate::live_control::LiveControl::start(&ctx).unwrap());
            }
            "generation" => {
                h.state().live_control.as_ref().unwrap().next_generation();
            }
            "timeout" => h.state_mut().automation.job.as_mut().unwrap().deadline = Instant::now() - Duration::from_secs(1),
            "physical" => h.input_mut().events.push(Event::PointerMoved(egui::pos2(100.0, 100.0))),
            "leave" => h.input_mut().events.push(Event::PointerGone),
            _ => panic!("Unknown interruption fixture: {reason}"),
        }
        let expected = if reason == "stale" { h.state().session.inspect() } else { before };
        assert_eq!(finish(&mut h)["state"], "cancelled", "{reason}");
        assert_eq!(h.state().session.inspect(), expected, "{reason}");
        assert!(!h.ctx.input(|i| i.pointer.any_down()), "{reason}");
    }
}

#[test]
fn queue_bounds_duplicate_handles_stale_layout_and_busy_commands_are_refused_atomically() {
    let mut h = ready();
    let before = h.state().session.inspect();
    for (id, params) in [
        ("ui.click", json!({"window":0,"target":{"x":-1,"y":0}})),
        ("ui.click", json!({"window":999,"target":{"x":10,"y":10}})),
        ("ui.key", json!({"window":0,"key":"not a key"})),
        ("ui.key", json!({"window":0,"key":"W","unknown":true})),
        ("ui.text", json!({"window":0,"text":"x".repeat(4097)})),
        ("ui.drag", json!({"window":0,"from":{"x":10,"y":10},"to":{"x":20,"y":20},"steps":121})),
        ("ui.scroll", json!({"window":0,"target":{"x":10,"y":10},"delta_x":0,"delta_y":10001})),
        ("ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"},"layout":0})),
    ] {
        assert!(command(&mut h, id, params).is_err(), "{id}");
        assert!(h.state().automation.job.is_none());
    }
    let targets = &mut h.state_mut().automation.snapshots.get_mut(&HostId(0)).unwrap().targets;
    let duplicate = targets.iter().find(|t| t.name == "toolbar:Connect:Wire").unwrap().clone();
    targets.push(duplicate);
    assert!(command(&mut h, "ui.at", json!({"window":0,"handle":"toolbar:Connect:Wire"})).unwrap_err().contains("ambiguous"));
    h.step();
    command(&mut h, "ui.key", json!({"window":0,"key":"W"})).unwrap();
    assert!(command(&mut h, "ui.key", json!({"window":0,"key":"Escape"})).unwrap_err().contains("busy"));
    assert!(command(&mut h, "view.fit", json!({"view":0})).unwrap_err().contains("queued UI input"));
    command(&mut h, "ui.cancel", json!({})).unwrap();
    finish(&mut h);
    assert_eq!(h.state().session.inspect(), before);
}

#[test]
fn input_is_retained_until_an_accepted_paint_and_named_targets_cannot_move_under_a_click() {
    let mut h = ready();
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Connect:Wire"}})).unwrap();
    let ctx = h.ctx.clone();
    let mut first = h.input_mut().clone();
    h.state_mut().automation_input(&ctx, &mut first);
    let mut second = first.clone();
    h.state_mut().automation_input(&ctx, &mut second);
    assert_eq!(first.events, second.events);
    assert_eq!(h.state().automation.job.as_ref().unwrap().frames.len(), 2);
    *h.input_mut() = first;
    h.step();
    assert_eq!(h.state().automation.job.as_ref().unwrap().frames.len(), 1);
    let snapshot = h.state_mut().automation.snapshots.get_mut(&HostId(0)).unwrap();
    snapshot.targets.retain(|t| t.name != "toolbar:Connect:Wire");
    assert_eq!(finish(&mut h)["state"], "cancelled");
    assert!(h.state().views.active.schematic.tool.is_none());
}

#[test]
fn named_targets_are_window_scoped_and_covered_background_controls_are_unavailable() {
    let mut h = ready();
    h.state_mut().dialog = crate::Dialog::Name { command: "project.rename", text: "Circuit".into(), error: None, focused: false };
    h.run();
    let targets = command(&mut h, "ui.handles", json!({"window":0})).unwrap();
    assert!(targets["handles"].as_array().unwrap().iter().any(|t| t["name"] == "button:OK"), "{targets}");
    let covered = h.state().automation.snapshots[&HostId(0)].targets.iter().find(|t| t.name == "toolbar:Connect:Wire");
    assert!(covered.is_none_or(|target| !target.enabled));
    assert!(command(&mut h, "ui.handles", json!({"window":1})).is_err());
}

#[test]
fn live_transport_discovers_and_queues_ui_input_with_the_existing_token() {
    let mut h = ready();
    let live = h.state().live_control.as_ref().unwrap();
    let (port, token) = (live.port(), live.secret().to_owned());
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            use pcbcraft_mcp::Backend;
            let mut remote = pcbcraft_mcp::live::Remote::new(port, &token).unwrap();
            let catalog = remote.call("engine.commands", json!({})).unwrap();
            assert!(catalog.as_array().unwrap().iter().any(|c| c["id"] == "ui.drag"));
            remote.call("engine.execute", json!({"command":"ui.key","params":{"window":0,"key":"W"}})).unwrap()
        });
        let until = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < until {
            h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(worker.is_finished());
        assert_eq!(worker.join().unwrap()["state"], "queued");
    });
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { .. })));
}

#[test]
fn cancellation_during_an_inflight_button_pass_cannot_recreate_and_commit_a_drag() {
    let mut h = ready();
    let before = h.state().session.inspect();
    let from = symbol(&h);
    command(&mut h, "ui.drag", json!({"window":0,"from":{"x":from.x,"y":from.y},"to":{"x":from.x+80.0,"y":from.y},"steps":8})).unwrap();
    tick(&mut h);
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    h.state_mut().automation_input(&ctx, &mut raw);
    command(&mut h, "ui.cancel", json!({})).unwrap();
    *h.input_mut() = raw;
    h.step();
    assert_eq!(finish(&mut h)["state"], "cancelled");
    assert_eq!(h.state().session.inspect(), before);
    assert!(!h.ctx.input(|i| i.pointer.any_down()));
}

#[test]
fn a_client_disconnected_before_acceptance_cannot_leave_queued_ui_input() {
    use craft_control::loopback::{Connection, Token};
    let mut h = ready();
    let live = h.state().live_control.as_ref().unwrap();
    let token = Token::parse(live.secret()).unwrap();
    let client = Connection::connect(
        live.port(),
        &token,
        json!({"method":"engine.execute","params":{"command":"ui.key","params":{"window":0,"key":"W"}}}),
        Duration::from_secs(2),
    )
    .unwrap();
    drop(client);
    // Do not run the editor while the bounded socket worker observes EOF. This is longer than
    // its 20 ms sampling interval and does not depend on a rendering/event-loop race.
    std::thread::sleep(Duration::from_millis(100));
    for _ in 0..4 {
        tick(&mut h);
    }
    assert!(h.state().automation.job.is_none());
    assert!(h.state().automation.last.is_null());
    assert!(h.state().views.active.schematic.tool.is_none());
}

/// Provenance covers every synthetic pass and cleanup, then clears for ordinary human input.
#[test]
fn synthetic_input_provenance_is_set_by_the_raw_adapter_and_cleared_afterward() {
    let mut h = ready();
    command(&mut h, "ui.key", json!({"window":0,"key":"W"})).unwrap();
    tick(&mut h);
    assert!(!pcbcraft_chrome::automation::human_input(&h.ctx));
    tick(&mut h);
    assert!(pcbcraft_chrome::automation::human_input(&h.ctx));
}
