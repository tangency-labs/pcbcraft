//! Queued UI input in a detached editor window reaches only that window's ordinary input.
use super::*;

/// Enable the live endpoint and open the main view in window 1, which holds the OS focus.
fn detached_input() -> (Harness<'static, PcbApp>, Rc<RefCell<Peer>>, ViewId) {
    let (mut h, peer) = multi_window();
    let ctx = h.ctx.clone();
    h.state_mut().live_control = Some(crate::live_control::LiveControl::start(&ctx).unwrap());
    h.state_mut().automation.observing = true;
    let root = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.open_window", &json!({"view":root.0})).unwrap();
    focus(&mut h, &peer, true);
    h.run_steps(4);
    let view = h.state().views.hosts[&HostId(1)].focused;
    (h, peer, view)
}

/// Send a command as the live endpoint does, attaching its authority to accepted input.
fn command(h: &mut Harness<'_, PcbApp>, id: &str, params: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    let result = h.state_mut().automation_command(&ctx, id, &params);
    if let Some(live) = &h.state().live_control {
        let owner = live.automation_owner(false, live.current());
        h.state_mut().automation.authorize(owner);
    }
    result
}

/// Run passes, with window 0's raw hook as eframe calls it, until the operation settles.
fn finish(h: &mut Harness<'_, PcbApp>) -> Value {
    for _ in 0..130 {
        let ctx = h.ctx.clone();
        let mut input = std::mem::take(h.input_mut());
        h.state_mut().automation_input(&ctx, &mut input);
        *h.input_mut() = input;
        h.step();
        let operation = command(h, "ui.inspect", json!({})).unwrap()["operation"].clone();
        if operation["state"] != "pending" {
            return operation;
        }
    }
    panic!("UI input did not finish");
}

/// The window-local point of the first symbol in `view`, projected from window 1's paint.
fn symbol_in(h: &mut Harness<'_, PcbApp>, view: ViewId) -> Value {
    let at = h.state().session.project().unwrap().symbols()[0].position;
    let point = command(h, "ui.project", json!({"view":view.0,"at":{"x_nm":at.x_nm,"y_nm":at.y_nm}})).unwrap();
    assert_eq!((&point["window"], &point["uncovered"]), (&json!(1), &json!(true)), "{point}");
    point
}

/// A 20-step drag of the first symbol in window 1, long enough to interrupt part-way.
fn drag(h: &mut Harness<'_, PcbApp>, view: ViewId, steps: u16) -> Result<Value, String> {
    let from = symbol_in(h, view);
    let (x, y) = (from["x"].as_f64().unwrap(), from["y"].as_f64().unwrap());
    command(h, "ui.drag", json!({"window":1,"layout":from["layout"],"from":{"x":x,"y":y},"to":{"x":x+40.64,"y":y},"steps":steps}))
}

/// A drag in the detached schematic edits the one shared project; the main window then
/// inspects the same revision and undoes and redoes it through its own keyboard input.
#[test]
fn detached_drag_edits_the_shared_project_and_the_main_window_undoes_it() {
    let (mut h, peer, view) = detached_input();
    let before = h.state().session.project().unwrap().symbols().to_vec();
    let revision = h.state().session.revision();
    assert_eq!(drag(&mut h, view, 4).unwrap()["state"], "queued");
    assert_eq!(finish(&mut h)["state"], "completed");
    let moved = h.state().session.project().unwrap().symbols().to_vec();
    assert_ne!(moved[0].position, before[0].position);
    assert_eq!(h.state().session.revision(), revision + 1);
    focus(&mut h, &peer, false);
    for (key, symbols) in [("Z", &before), ("Y", &moved)] {
        command(&mut h, "ui.key", json!({"window":0,"key":key,"modifiers":{"command":true}})).unwrap();
        assert_eq!(finish(&mut h)["state"], "completed");
        assert_eq!(h.state().session.project().unwrap().symbols(), symbols.as_slice());
    }
}

/// Input for window 1 needs that window's focus and current layout, and an unknown
/// window is refused; none of these refusals edits the project.
#[test]
fn detached_input_needs_its_own_focus_layout_and_window() {
    let (mut h, peer, view) = detached_input();
    let before = h.state().session.inspect();
    let child = symbol_in(&mut h, view);
    let main = command(&mut h, "ui.at", json!({"window":0,"handle":"tab:View"})).unwrap();
    assert_ne!(main["layout"], child["layout"]);
    let click = |layout: &Value| json!({"window":1,"layout":layout,"target":{"x":child["x"],"y":child["y"]}});
    assert!(command(&mut h, "ui.click", click(&main["layout"])).unwrap_err().contains("stale"));
    assert!(command(&mut h, "ui.click", json!({"window":9,"target":{"x":1.0,"y":1.0}})).unwrap_err().contains("Unknown editor window"));
    focus(&mut h, &peer, false);
    assert!(command(&mut h, "ui.click", json!({"window":1,"target":{"x":child["x"],"y":child["y"]}})).unwrap_err().contains("Focus"));
    assert_eq!(h.state().session.inspect(), before);
}

/// One way to interrupt a queued drag in window 1.
type Interrupt = fn(&mut Harness<'static, PcbApp>, &Rc<RefCell<Peer>>, ViewId);

/// A drag interrupted part-way is cancelled with its reason, leaves no edit and no held
/// button, and the next input is accepted: focus loss, physical input, a resize, a
/// revoked connection, redocking and closing the window.
#[test]
fn detached_input_is_cancelled_without_an_edit() {
    let interruptions: [(&str, Interrupt); 6] = [
        ("Editor window lost focus or visibility", |_, peer, _| peer.borrow_mut().focused = false),
        ("Physical input interrupted automation", |_, peer, _| peer.borrow_mut().events.push(Event::PointerMoved(pos2(5.0, 5.0)))),
        ("Window size or scale changed", |_, peer, _| peer.borrow_mut().size = Some(vec2(1000.0, 700.0))),
        ("Automation connection or Assistant turn ended", |h, _, _| h.state_mut().live_control = None),
        ("Project or input destination changed", |h, _, view| {
            let ctx = h.ctx.clone();
            h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":view.0})).unwrap();
        }),
        ("Project or input destination changed", |_, peer, _| peer.borrow_mut().close = true),
    ];
    for (reason, interrupt) in interruptions {
        let (mut h, peer, view) = detached_input();
        let before = (h.state().session.inspect(), h.state().session.revision());
        assert_eq!(drag(&mut h, view, 20).unwrap()["state"], "queued", "{reason}");
        h.run_steps(4);
        interrupt(&mut h, &peer, view);
        let operation = finish(&mut h);
        assert_eq!((&operation["state"], &operation["reason"]), (&json!("cancelled"), &json!(reason)));
        assert_eq!((h.state().session.inspect(), h.state().session.revision()), before, "{reason}");
        h.run_steps(3);
        let ctx = h.ctx.clone();
        h.state_mut().live_control.get_or_insert_with(|| crate::live_control::LiveControl::start(&ctx).unwrap());
        let window = if h.state().views.hosts.contains_key(&HostId(1)) { 1 } else { 0 };
        focus(&mut h, &peer, window == 1);
        assert_eq!(command(&mut h, "ui.key", json!({"window":window,"key":"Escape"})).unwrap()["state"], "queued", "{reason}");
        assert_eq!(finish(&mut h)["state"], "completed", "{reason}");
    }
}
