//! Real editor interactions, queue authority and authenticated MCP discovery.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};

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
    result.map_err(|failure| failure.to_string())
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

/// Parts Library's early content return still publishes the shared host's named
/// controls and accepts queued input without changing the parked project.
#[test]
fn library_workspace_retains_shared_automation_host() {
    let mut h = ready();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let before = h.state().session.inspect();
    h.state_mut().run_window_command(&ctx, "app.set_workspace", &json!({"view":view,"workspace":"library"})).unwrap();
    h.run();
    let handles = command(&mut h, "ui.handles", json!({"window":0,"prefix":"toolbar:Sources:"})).unwrap();
    assert!(handles["handles"].as_array().unwrap().iter().any(|t| t["name"] == "toolbar:Sources:Add library" && t["enabled"] == true));
    command(&mut h, "ui.click", json!({"window":0,"target":{"handle":"toolbar:Sources:Add library"}})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(egui::Popup::is_any_open(&h.ctx));
    // Completion accepts the click's UI pass; egui paints the newly opened
    // popup on the following pass rather than making that presentation promise.
    h.run();
    h.get_by_label("Choose folder…");
    h.get_by_label("Choose bundle…");
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
    assert!(command(&mut h, "app.library_source", json!({"view":0,"action":"refresh"})).unwrap_err().contains("queued UI input"));
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

/// Logic-only callbacks can retain an unconsumed release. Keep ownership until egui accepts
/// that frame, allowing either reinjection or cancellation cleanup to release the button.
#[test]
fn a_retained_release_stays_owned_until_the_accepted_paint() {
    for cancel in [false, true] {
        let mut h = ready();
        let at = symbol(&h);
        command(&mut h, "ui.drag", json!({"window":0,"from":{"x":at.x,"y":at.y},"to":{"x":at.x+40.0,"y":at.y},"steps":2})).unwrap();
        for _ in 0..4 {
            tick(&mut h);
        }
        assert!(h.ctx.input(|input| input.pointer.any_down()));
        let ctx = h.ctx.clone();
        let mut raw = std::mem::take(h.input_mut());
        h.state_mut().automation_input(&ctx, &mut raw);
        let first = raw.events.clone();
        if cancel {
            command(&mut h, "ui.cancel", json!({})).unwrap();
        }
        h.state_mut().automation_input(&ctx, &mut raw);
        assert_eq!(raw.events, first);
        assert!(h.state().automation.job.is_some());
        *h.input_mut() = raw;
        h.step();
        assert_eq!(finish(&mut h)["state"], if cancel { "cancelled" } else { "completed" });
        assert!(!h.ctx.input(|input| input.pointer.any_down()));
        command(&mut h, "ui.key", json!({"window":0,"key":"Escape"})).unwrap();
        assert_eq!(finish(&mut h)["state"], "completed");
    }
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
fn idle_cancel_preserves_the_persons_wire_tool_and_draft() {
    let mut h = ready();
    h.state_mut().views.active.schematic.tool = Some(crate::Tool::Wire { revision: h.state().session.revision(), start: None });
    h.state_mut().views.active.schematic.wire_draft.horizontal_first = false;
    command(&mut h, "ui.cancel", json!({})).unwrap();
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { .. })));
    assert!(!h.state().views.active.schematic.wire_draft.horizontal_first);
}

/// A stationary physical press remains owned by the person, including a press that arrives
/// between admission and injection. Cancellation must leave its held state and gesture intact.
#[test]
fn stationary_held_buttons_block_automation_without_cancelling_the_persons_gesture() {
    for before_admission in [true, false] {
        let mut h = ready();
        let at = symbol(&h);
        if !before_admission {
            command(&mut h, "ui.key", json!({"window":0,"key":"W"})).unwrap();
        }
        h.input_mut().events.extend([
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        ]);
        h.step();
        h.input_mut().events.push(Event::PointerMoved(at + egui::vec2(40.0, 0.0)));
        h.step();
        assert!(h.ctx.input(|input| input.pointer.any_down()));
        assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Move { .. })));
        let before = h.state().session.inspect();
        if before_admission {
            assert!(command(&mut h, "ui.key", json!({"window":0,"key":"W"})).unwrap_err().contains("held"));
        } else {
            tick(&mut h);
            assert_eq!(h.state().automation.status()["state"], "cancelled");
        }
        assert!(h.ctx.input(|input| input.pointer.any_down()));
        assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Move { .. })));
        assert_eq!(h.state().session.inspect(), before);
    }
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

/// Provisioning and credential export require a human even when coordinates or Enter target
/// the dialog; real enable/copy and synthetic revocation remain available.
#[test]
fn synthetic_input_cannot_enable_or_export_a_local_connection() {
    use egui_kittest::kittest::Queryable;
    let mut h = crate::tests::harness();
    h.state_mut().dialog = crate::Dialog::LocalMcp { error: None, copied: false };
    h.run();
    let at = h.get_by_label("Enable local connection").rect().center();
    pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.step();
    assert!(h.state().live_control.is_none());
    pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, false);
    h.run();
    h.get_by_label("Enable local connection").click();
    h.run();
    assert!(h.state().live_control.is_some());
    h.get_by_label("Copy configuration").focus();
    h.run();
    pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
    h.key_press(egui::Key::Enter);
    h.step();
    assert!(matches!(h.state().dialog, crate::Dialog::LocalMcp { copied: false, .. }));
    assert!(!h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
    pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, false);
    h.run();
    h.get_by_label("Copy configuration").click();
    h.step();
    assert!(matches!(h.state().dialog, crate::Dialog::LocalMcp { copied: true, .. }));
    assert!(h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
    h.run();
    let at = h.get_by_label("Disable connection").rect().center();
    pcbcraft_chrome::automation::mark_synthetic_input(&h.ctx, true);
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.step();
    assert!(h.state().live_control.is_none());
}

/// Project a model point through the production dispatcher.
fn project(h: &mut Harness<'_, PcbApp>, view: u64, x_nm: i64, y_nm: i64) -> Result<Value, String> {
    command(h, "ui.project", json!({"view":view,"at":{"x_nm":x_nm,"y_nm":y_nm}}))
}

/// Clicking the projection of a known symbol selects it; the query itself changes no camera,
/// selection or project data, and its layout guards the click.
#[test]
fn projected_symbol_click_selects_it_without_the_query_changing_state() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    let at = h.state().session.project().unwrap().symbols()[0].position;
    let (before, camera) = (h.state().session.inspect(), h.state().views.active.camera.saved_state());
    let projected = project(&mut h, view, at.x_nm, at.y_nm).unwrap();
    assert_eq!((projected["window"].clone(), projected["view"].clone()), (json!(0), json!(view)));
    assert_eq!((projected["on_canvas"].clone(), projected["uncovered"].clone()), (json!(true), json!(true)), "{projected}");
    let expected = symbol(&h);
    assert!(
        (projected["x"].as_f64().unwrap() as f32 - expected.x).abs() < 1e-3 && (projected["y"].as_f64().unwrap() as f32 - expected.y).abs() < 1e-3
    );
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(h.state().views.active.camera.saved_state(), camera);
    assert!(h.state().views.active.schematic.selection.single_symbol().is_none());
    let target = json!({"x":projected["x"],"y":projected["y"]});
    command(&mut h, "ui.click", json!({"window":0,"layout":projected["layout"],"target":target})).unwrap();
    assert_eq!(finish(&mut h)["state"], "completed");
    assert!(h.state().views.active.schematic.selection.single_symbol().is_some());
}

/// Pan and zoom change the projection; coordinates from before the navigation are refused
/// both before and after the next paint.
#[test]
fn navigation_makes_projected_coordinates_stale() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    let click = |p: &Value| json!({"window":0,"layout":p["layout"],"target":{"x":p["x"],"y":p["y"]}});
    for navigate in [0, 1] {
        let first = project(&mut h, view, 0, 0).unwrap();
        let camera = &mut h.state_mut().views.active.camera;
        if navigate == 0 {
            camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 2_540_000, y_nm: 0 })
        } else {
            camera.set_zoom(2.0)
        }
        assert!(command(&mut h, "ui.click", click(&first)).unwrap_err().contains("stale"));
        assert!(project(&mut h, view, 0, 0).unwrap_err().contains("stale"));
        h.run();
        assert!(command(&mut h, "ui.click", click(&first)).unwrap_err().contains("stale"));
        let moved = project(&mut h, view, 0, 0).unwrap();
        assert_ne!(
            (moved["x"].clone(), moved["y"].clone(), moved["layout"].clone()),
            (first["x"].clone(), first["y"].clone(), first["layout"].clone())
        );
    }
}

/// Each split pane projects through its own camera into its own canvas; a closed view is refused.
#[test]
fn split_panes_project_into_their_own_canvas_and_closed_views_are_refused() {
    let mut h = ready();
    let ctx = h.ctx.clone();
    let first = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.split", &json!({"view":first.0,"split":true})).unwrap();
    h.run();
    h.run();
    let views = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    assert_eq!(views.len(), 2);
    let mut points = Vec::new();
    for view in &views {
        let projected = project(&mut h, view.0, 0, 0).unwrap();
        let rect = h.state().views.view(*view).unwrap().camera.schematic_rect().unwrap();
        let at = egui::pos2(projected["x"].as_f64().unwrap() as f32, projected["y"].as_f64().unwrap() as f32);
        assert!(rect.contains(at) && projected["uncovered"] == true, "{projected}");
        points.push(at);
    }
    assert_ne!(points[0], points[1]);
    let second = *views.iter().find(|view| **view != first).unwrap();
    h.state_mut().run_window_command(&ctx, "view.close", &json!({"view":second.0})).unwrap();
    h.run();
    assert!(project(&mut h, second.0, 0, 0).unwrap_err().contains("view"));
    assert!(project(&mut h, 999, 0, 0).unwrap_err().contains("Unknown view"));
}

/// A flipped board mirrors the projection, and clicking a projected pad selects its footprint.
#[test]
fn flipped_board_projection_reaches_the_pad() {
    for flipped in [false, true] {
        let mut h = ready_board();
        let ctx = h.ctx.clone();
        let view = h.state().views.active.id().0;
        h.state_mut().run_window_command(&ctx, "view.set_board_flipped", &json!({"view":view,"flipped":flipped})).unwrap();
        h.run();
        let pad = {
            let project = h.state().session.project().unwrap();
            let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
            board.footprints().next().unwrap().pads[0].center
        };
        let projected = project(&mut h, view, pad.x_nm, pad.y_nm).unwrap();
        let mirrored = project(&mut h, view, -pad.x_nm, pad.y_nm).unwrap();
        let center = project(&mut h, view, 0, pad.y_nm).unwrap();
        let (x, left) = (projected["x"].as_f64().unwrap(), center["x"].as_f64().unwrap());
        assert_eq!(x > left, (pad.x_nm > 0) != flipped, "flipped={flipped} {projected}");
        assert!((x - left + mirrored["x"].as_f64().unwrap() - left).abs() < 1e-3);
        assert_eq!(projected["uncovered"], true, "{projected}");
        command(&mut h, "ui.click", json!({"window":0,"layout":projected["layout"],"target":{"x":projected["x"],"y":projected["y"]}})).unwrap();
        assert_eq!(finish(&mut h)["state"], "completed");
        assert!(!h.state().views.active.board.selection.is_empty(), "flipped={flipped}");
    }
}

/// Display scaling changes physical pixels, not the logical coordinates input uses.
#[test]
fn projection_uses_logical_points_at_every_ui_scale() {
    for scale in [1.0, 1.5, 2.0] {
        let mut h = ready();
        h.ctx.set_pixels_per_point(scale);
        h.run();
        h.run();
        let view = h.state().views.active.id().0;
        let at = h.state().session.project().unwrap().symbols()[0].position;
        let projected = project(&mut h, view, at.x_nm, at.y_nm).unwrap();
        assert_eq!(projected["scale"], json!(scale));
        let expected = symbol(&h);
        assert!((projected["x"].as_f64().unwrap() as f32 - expected.x).abs() < 1e-3, "scale={scale} {projected}");
        command(&mut h, "ui.click", json!({"window":0,"layout":projected["layout"],"target":{"x":projected["x"],"y":projected["y"]}})).unwrap();
        assert_eq!(finish(&mut h)["state"], "completed");
        assert!(h.state().views.active.schematic.selection.single_symbol().is_some(), "scale={scale}");
    }
}

/// A point under an open menu still projects, but is reported as covered; one off the canvas
/// is reported as off-canvas.
#[test]
fn covered_and_off_canvas_points_are_identified() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    crate::tests::click(&mut h, "File");
    h.run();
    let menu = egui::Popup::is_any_open(&h.ctx);
    assert!(menu);
    let rect = h.state().views.active.camera.schematic_rect().unwrap();
    let region = h.state().automation.snapshots[&HostId::ROOT].targets.iter().find(|t| t.name == format!("canvas:{view}")).unwrap().rect;
    // The lowest menu rows reach over the canvas, partly clear of its floating cards.
    let popup = ["Quit", "Keyboard Shortcuts…", "Preferences…"]
        .into_iter()
        .map(|label| h.get_by_label(label).rect().intersect(region))
        .find(|overlap| overlap.is_positive())
        .expect("a File menu row over the uncovered canvas")
        .center();
    let under = h.state().views.active.camera.unsnapped(popup, rect);
    let covered = project(&mut h, view, under.x_nm, under.y_nm).unwrap();
    assert_eq!((covered["on_canvas"].clone(), covered["uncovered"].clone()), (json!(true), json!(false)), "{covered}");
    let far = project(&mut h, view, pcbcraft_engine::schematic::MAX_CENTER_NM, 0).unwrap();
    assert_eq!((far["on_canvas"].clone(), far["uncovered"].clone()), (json!(false), json!(false)));
}

/// Open the board workspace of a project with one placed footprint at the origin.
fn ready_board() -> Harness<'static, PcbApp> {
    let mut h = crate::tests::harness();
    h.set_size(egui::vec2(1600.0, 1000.0));
    crate::tests::resistor_with_board_at_origin(&mut h);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.browser.open = false;
    app.run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":2}),
    );
    let view = app.views.active.id().0;
    app.run_window_command(&ctx, "view.set_workspace", &json!({"view":view,"workspace":"board"})).unwrap();
    h.run();
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 });
    h.state_mut().live_control = Some(crate::live_control::LiveControl::start(&ctx).unwrap());
    h.state_mut().automation.observing = true;
    h.run();
    h
}

/// A definition view projects through its own presentation camera; Parts Library browsing
/// has no model canvas and is refused.
#[test]
fn definition_views_project_and_library_browsing_is_refused() {
    let mut h = ready();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "app.set_workspace", &json!({"view":view.0,"workspace":"symbol_editor"})).unwrap();
    h.run_steps(3);
    let projected = project(&mut h, view.0, 0, 0).unwrap();
    let definition = h.state().views.view(view).unwrap().definition.unwrap();
    let camera = h.state().library_editors.view_camera(definition, view).unwrap();
    let expected = camera.screen(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }, camera.painted_rect().unwrap());
    assert!((projected["x"].as_f64().unwrap() as f32 - expected.x).abs() < 1e-3, "{projected}");
    assert_eq!(projected["on_canvas"], true, "{projected}");
    h.state_mut().run_window_command(&ctx, "app.set_workspace", &json!({"view":view.0,"workspace":"library"})).unwrap();
    h.run_steps(3);
    assert!(project(&mut h, view.0, 0, 0).unwrap_err().contains("no model canvas"));
}

/// Out-of-range model coordinates, including the extreme integers, are refused without overflow.
#[test]
fn projection_refuses_out_of_range_coordinates() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    let bound = pcbcraft_engine::schematic::MAX_CENTER_NM + pcbcraft_engine::schematic::MAX_PIN_OFFSET_NM;
    for (x, y) in [(i64::MIN, 0), (0, i64::MIN), (i64::MAX, 0), (0, bound + 1), (-bound - 1, 0)] {
        assert!(project(&mut h, view, x, y).unwrap_err().contains(&format!("±{bound}")), "({x}, {y})");
    }
    // A pin or pad beyond the largest placement center is still a valid target.
    project(&mut h, view, bound, -bound).unwrap();
}

/// Home hides the main workspace: its stale canvases are not projected, and returning to the
/// workspace changes the layout token.
#[test]
fn home_hides_the_main_canvas_from_projection() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    let before = project(&mut h, view, 0, 0).unwrap();
    h.state_mut().show_home();
    h.run();
    assert!(project(&mut h, view, 0, 0).unwrap_err().contains("Home"));
    h.state_mut().home = false;
    h.run();
    let after = project(&mut h, view, 0, 0).unwrap();
    assert_ne!(after["layout"], before["layout"]);
    assert_eq!((after["x"].clone(), after["y"].clone()), (before["x"].clone(), before["y"].clone()));
}

/// A canvas that moves after a coordinate click was admitted, as an eased zoom does, cancels
/// the click before its button press instead of landing on another model point.
#[test]
fn a_canvas_moving_after_admission_cancels_a_coordinate_click() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    let at = h.state().session.project().unwrap().symbols()[0].position;
    let projected = project(&mut h, view, at.x_nm, at.y_nm).unwrap();
    command(&mut h, "ui.click", json!({"window":0,"layout":projected["layout"],"target":{"x":projected["x"],"y":projected["y"]}})).unwrap();
    tick(&mut h);
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 2_540_000, y_nm: 0 });
    let status = finish(&mut h);
    assert_eq!(status["state"], "cancelled", "{status}");
    assert!(h.state().views.active.schematic.selection.single_symbol().is_none());
}

/// A clear canvas point beside a floating card, outside the largest clear rectangle that the
/// `canvas:` handle publishes, is still uncovered; a point on a card is not.
#[test]
fn coverage_uses_the_actual_cards_not_the_largest_clear_rectangle() {
    let mut h = ready();
    let view = h.state().views.active.id();
    let snapshot = &h.state().automation.snapshots[&HostId::ROOT];
    let region = snapshot.targets.iter().find(|t| t.name == format!("canvas:{}", view.0)).unwrap().rect;
    let canvas = snapshot.canvases.iter().find(|c| c.view == view).unwrap().clone();
    let covers: Vec<Rect> = canvas.cards.iter().chain(&snapshot.overlays).copied().collect();
    let grid = (0..64).flat_map(|i| (0..64).map(move |j| (i, j)));
    let points: Vec<Pos2> = grid.map(|(i, j)| canvas.rect.lerp_inside(egui::vec2(i as f32 / 63.0, j as f32 / 63.0))).collect();
    let clear = *points.iter().find(|p| !region.contains(**p) && !covers.iter().any(|c| c.contains(**p))).expect("a clear point beside a card");
    let carded = *points.iter().find(|p| canvas.cards.iter().any(|c| c.contains(**p))).expect("a point on a card");
    for (point, expected) in [(clear, true), (carded, false)] {
        let camera = &h.state().views.active.camera;
        let at = camera.unsnapped(point, canvas.rect);
        let projected = project(&mut h, view.0, at.x_nm, at.y_nm).unwrap();
        assert_eq!(projected["uncovered"], expected, "{point:?} {projected}");
    }
}

/// Deleting a sheet that has already painted leaves an unavailable placeholder with its old
/// camera; projecting into it is refused rather than reported as an uncovered canvas.
#[test]
fn an_unavailable_sheet_view_is_not_projected() {
    let mut h = ready();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    crate::tests::click(&mut h, "Power");
    h.run();
    let view = h.state().views.active.id().0;
    project(&mut h, view, 0, 0).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::sheets::DELETE_SHEET, &json!({"sheet":power.id(),"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.active.unavailable());
    assert!(project(&mut h, view, 0, 0).unwrap_err().contains("unavailable"));
}

/// Fit defers navigation until paint. Neither queries nor input can use the previous mapping
/// in that interval; the new projection is available as soon as the fit is painted.
#[test]
fn pending_fit_invalidates_projection_and_input_until_paint() {
    let mut h = ready();
    let view = h.state().views.active.id().0;
    command(&mut h, "view.center", json!({"view":view,"at":{"x_nm":12_700_000,"y_nm":0}})).unwrap();
    h.run();
    let before = project(&mut h, view, 0, 0).unwrap();
    command(&mut h, "view.fit", json!({"view":view})).unwrap();
    assert!(project(&mut h, view, 0, 0).unwrap_err().contains("stale"));
    let input = json!({"window":0,"layout":before["layout"],"target":{"x":before["x"],"y":before["y"]}});
    assert!(command(&mut h, "ui.move", input).unwrap_err().contains("stale"));
    h.run();
    let after = project(&mut h, view, 0, 0).unwrap();
    assert_ne!(before["layout"], after["layout"]);
    assert_ne!(before["x"], after["x"]);
}

/// A new overlay over a queued coordinate press cancels it; a new overlay elsewhere leaves
/// the same intended canvas click available, even when it changes the layout token.
#[test]
fn an_overlay_cancels_a_coordinate_press_only_if_it_covers_the_target() {
    use std::{cell::Cell, rc::Rc};
    let overlay_at = Rc::new(Cell::new(None::<Pos2>));
    let source = overlay_at.clone();
    let activated = Rc::new(Cell::new(false));
    let button_activated = activated.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_ui_state(
        move |ui, app: &mut PcbApp| {
            if let Some(at) = source.get() {
                egui::Area::new(egui::Id::unique("coordinate_press_test_overlay"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(at - egui::vec2(50.0, 12.0))
                    .show(ui.ctx(), |ui| {
                        if ui.add_sized([100.0, 24.0], egui::Button::new("Overlay action")).clicked() {
                            button_activated.set(true);
                        }
                    });
            }
            app.frame(ui);
        },
        PcbApp::default(),
    );
    crate::tests::resistor_at_origin(&mut h);
    let ctx = h.ctx.clone();
    h.state_mut().live_control = Some(crate::live_control::LiveControl::start(&ctx).unwrap());
    h.state_mut().automation.observing = true;
    h.run();
    let view = h.state().views.active.id().0;
    for (offset, uncovered, expected) in [(egui::Vec2::ZERO, false, "cancelled"), (egui::vec2(200.0, 0.0), true, "completed")] {
        overlay_at.set(None);
        h.run_steps(3);
        let projected = project(&mut h, view, 0, 0).unwrap();
        assert_eq!(projected["uncovered"], true);
        let click = json!({"window":0,"layout":projected["layout"],"target":{"x":projected["x"],"y":projected["y"]}});
        command(&mut h, "ui.click", click).unwrap();
        tick(&mut h);
        let at = egui::pos2(projected["x"].as_f64().unwrap() as f32, projected["y"].as_f64().unwrap() as f32);
        overlay_at.set(Some(at + offset));
        h.run_steps(3);
        let latest = project(&mut h, view, 0, 0).unwrap();
        assert_eq!(latest["uncovered"], uncovered, "{latest}");
        assert_ne!(latest["layout"], projected["layout"]);
        let outcome = finish(&mut h);
        assert_eq!(outcome["state"], expected, "{outcome}; overlay activated: {}", activated.get());
        assert!(!activated.get());
        assert_eq!(h.state().views.active.schematic.selection.single_symbol().is_some(), uncovered);
    }
}
