use super::*;
use pcbcraft_engine::{net_labels, schematic::WireEndpoint};
use std::sync::Arc;

/// Click a model point with `modifiers`, preserving hover, press and release frames.
fn click_with(h: &mut Harness<'_, PcbApp>, point: Point, modifiers: Modifiers) {
    let pos = screen(h, point);
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
        h.run();
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Wire `a` runs along row 0 and is crossed by `b` at (0, 0) and by `c` close by at (4, 0).
/// All three are free wires drawn through the shared command; nothing is connected.
fn crossing_harness() -> Harness<'static, PcbApp> {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.apply("project.new", &json!({"name":"demo-join"})).unwrap();
    for (from, to) in [(grid(-8, 0), grid(12, 0)), (grid(0, -6), grid(0, 6)), (grid(4, -6), grid(4, 6))] {
        s.apply(DRAW_WIRE, &json!({"from":from,"to":to,"bends":[],"expected_revision":s.revision()})).unwrap();
    }
    h.state_mut().browser.open = false;
    h.run();
    h
}

/// Select wires `a` and `b` with a click and a Shift-click away from the crossings, then open
/// the group menu at their crossing.
fn open_join_menu(h: &mut Harness<'_, PcbApp>) {
    click_with(h, grid(-6, 0), Modifiers::NONE);
    click_with(h, grid(0, -4), Modifiers::SHIFT);
    assert_eq!(h.state().views.active.schematic.selection.wires.len(), 2);
    right_click(h, grid(0, 0), egui::Vec2::ZERO);
}

/// The two wires' nets in Net Inspector order, and whether they are one net.
fn joined(h: &Harness<'_, PcbApp>, first: usize, second: usize) -> bool {
    let project = h.state().session.project().unwrap();
    let graph = project.connectivity();
    let net = |index: usize| graph.net_index_for_endpoint(project.wires()[index].endpoints()[0]);
    net(first) == net(second)
}

/// The group menu of two selected crossing wires joins them at that crossing through the shared
/// command: the nearby crossing stays separate, and Undo and Redo each take one step.
#[test]
fn join_at_crossing_joins_only_the_selected_wires() {
    let mut h = crossing_harness();
    let before = h.state().session.snapshot().unwrap();
    open_join_menu(&mut h);
    let row = h.get_by_label("Join at Crossing");
    assert!(!row.accesskit_node().is_disabled());
    row.hover();
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "opening and hovering the menu edits nothing");
    click(&mut h, "Join at Crossing");
    assert_eq!(h.state().message, None);
    let project = h.state().session.snapshot().unwrap();
    assert!(joined(&h, 0, 1));
    assert!(!joined(&h, 0, 2), "the nearby crossing stays disconnected");
    let junction = project.junctions().find(|(_, at)| *at == grid(0, 0)).map(|(id, _)| WireEndpoint::Junction(id)).unwrap();
    assert_eq!(project.wires().iter().filter(|wire| wire.endpoints().contains(&junction)).count(), 4);

    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert!(!joined(&h, 0, 1), "one Undo separates them again");
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z);
    h.run();
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()), "one Redo restores the join");
}

/// A join between differently named nets is offered disabled, and an open menu whose revision
/// went stale refuses the edit with a message instead of changing the project.
#[test]
fn refused_and_stale_joins_change_nothing() {
    let mut h = crossing_harness();
    let s = &mut h.state_mut().session;
    for (name, at) in [("demo-a", grid(12, 0)), ("demo-b", grid(0, 6))] {
        s.apply(net_labels::ADD, &json!({"name":name,"style":"local","x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":s.revision()})).unwrap();
    }
    h.run();
    let before = h.state().session.snapshot().unwrap();
    open_join_menu(&mut h);
    assert!(h.get_by_label("Join at Crossing").accesskit_node().is_disabled());
    click(&mut h, "Join at Crossing");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(egui::Key::Escape);
    h.run();

    let mut h = crossing_harness();
    open_join_menu(&mut h);
    let s = &mut h.state_mut().session;
    s.apply(DRAW_WIRE, &json!({"from":grid(-8, 10),"to":grid(8, 10),"bends":[],"expected_revision":s.revision()})).unwrap();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Join at Crossing");
    assert!(h.state().message.is_some(), "a stale menu reports the refusal");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
