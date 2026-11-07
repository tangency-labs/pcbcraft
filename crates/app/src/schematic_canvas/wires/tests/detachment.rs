use super::*;
use crate::Tool;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint};
use std::sync::Arc;

/// Click an actual canvas position, preserving hover, press and release frames.
fn click_at(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = screen(h, point);
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Build an original three-leg wire, then invoke Break from its context menu.
fn broken_harness() -> Harness<'static, PcbApp> {
    broken_harness_axis(false)
}

/// Rotate the command-built fixture to exercise horizontal and vertical detached pieces.
fn broken_harness_axis(horizontal: bool) -> Harness<'static, PcbApp> {
    let point = |x, y| if horizontal { grid(y, x) } else { grid(x, y) };
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.apply("project.new", &json!({"name":"Detached piece"})).unwrap();
    s.apply(DRAW_WIRE, &json!({"from":point(-8,0),"to":point(16,6),"bends":[point(8,0),point(8,6)],"expected_revision":s.revision()})).unwrap();
    h.state_mut().browser.open = false;
    h.run();
    right_click(&mut h, point(8, 3), egui::Vec2::ZERO);
    click(&mut h, "Break");
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), 3);
    h
}

/// A broken piece can leave even while one end slides along its old neighboring wire.
/// After dropping away, a subsequent drop back at the original contacts joins them normally.
#[test]
fn break_departure_stays_disconnected_then_a_later_drop_reconnects() {
    let mut h = broken_harness();
    let before = h.state().session.snapshot().unwrap();
    let start = screen(&h, grid(8, 3));
    let end = screen(&h, grid(12, 3));
    h.drag_at(start);
    h.run();
    h.hover_at(end);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Reshape { .. })));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.drop_at(end);
    h.run();
    assert_eq!(h.state().message, None);
    let away = h.state().session.snapshot().unwrap();
    assert_eq!(away.connectivity().nets.len(), 3);
    assert!(routes(&h).contains(&vec![grid(12, 0), grid(12, 6)]));
    let start = screen(&h, grid(12, 3));
    let end = screen(&h, grid(8, 3));
    h.drag_at(start);
    h.run();
    h.hover_at(end);
    h.run();
    h.drop_at(end);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), 1);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&away, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Returning during the first gesture also reconnects, provided the ends were first pulled
/// clear. Escape discards the shared transient guard together with the entire preview.
#[test]
fn break_can_rejoin_after_departing_in_the_same_gesture() {
    for cancel in [false, true] {
        let mut h = broken_harness();
        let before = h.state().session.snapshot().unwrap();
        let start = screen(&h, grid(8, 3));
        let away = screen(&h, grid(20, 3));
        h.drag_at(start);
        h.run();
        h.hover_at(away);
        h.run();
        h.hover_at(start);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        if cancel {
            h.key_press(egui::Key::Escape);
            h.run();
            h.drop_at(start);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        } else {
            h.drop_at(start);
            h.run();
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), 1);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// A wired symbol for context-menu pickup and replacement, with real terminal identities.
fn symbol_harness() -> Harness<'static, PcbApp> {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.apply("project.new", &json!({"name":"Replace part"})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let pins = s.project().unwrap().symbols()[0].terminals();
    for (pin, to) in [(pins[0], grid(-12, 0)), (pins[1], grid(12, 0))] {
        s.apply(DRAW_WIRE, &json!({"from":WireEndpoint::Terminal(pin).inspect(),"to":to,"bends":[],"expected_revision":s.revision()})).unwrap();
    }
    h.state_mut().browser.open = false;
    h.run();
    h
}

/// Context-menu disconnect-and-move previews without touching history, leaves old routes
/// exactly in place on click, and restores pins and position together on Undo.
#[test]
fn symbol_disconnect_and_move_is_one_cancelable_edit() {
    for cancel in [false, true] {
        let mut h = symbol_harness();
        let before = h.state().session.snapshot().unwrap();
        let paths = routes(&h);
        right_click(&mut h, grid(0, 0), egui::Vec2::ZERO);
        click(&mut h, "Disconnect and Move");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Move { disconnect: true, .. })));
        let end = screen(&h, grid(0, 12));
        h.hover_at(end);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        if cancel {
            h.key_press(egui::Key::Escape);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        } else {
            click_at(&mut h, grid(0, 12));
            assert_eq!(h.state().message, None);
            assert!(h.state().views.active.schematic.tool.is_none());
            let after = h.state().session.snapshot().unwrap();
            assert_eq!(after.symbols()[0].position, grid(0, 12));
            assert_eq!(routes(&h), paths);
            assert!(after.wires().iter().all(|wire| wire.terminals().is_empty()));
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            click(&mut h, "Redo");
            assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// A click without departure keeps Break intact, while stale or reopened symbol pickups and
/// secondary-click cancellation never commit a disconnect to the current document.
#[test]
fn no_departure_and_interrupted_symbol_pickups_do_not_edit() {
    let mut h = broken_harness();
    let before = h.state().session.snapshot().unwrap();
    let at = screen(&h, grid(8, 3));
    h.hover_at(at);
    h.run();
    h.key_press(egui::Key::G);
    h.run();
    click_at(&mut h, grid(8, 3));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    for mode in 0..3 {
        let mut h = symbol_harness();
        right_click(&mut h, grid(0, 0), egui::Vec2::ZERO);
        click(&mut h, "Disconnect and Move");
        let to = screen(&h, grid(0, 12));
        h.hover_at(to);
        h.run();
        if mode == 0 {
            right_click(&mut h, grid(0, 12), egui::Vec2::ZERO);
            assert_eq!(h.state().session.project().unwrap().symbols()[0].position, grid(0, 0));
        } else {
            if mode == 1 {
                h.state_mut().run("project.rename", &json!({"name":"Changed"}));
            } else {
                let snapshot = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(snapshot);
            }
            let before = h.state().session.snapshot().unwrap();
            h.run();
            click_at(&mut h, grid(0, 12));
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
        assert!(h.state().views.active.schematic.tool.is_none());
    }
}

/// A detached endpoint handle also pulls clear without snapping immediately back to its old
/// coincident neighbor, and can deliberately reconnect there later in the same drag.
#[test]
fn broken_endpoint_handles_can_depart_and_return() {
    let mut h = broken_harness();
    let before = h.state().session.snapshot().unwrap();
    let wire = before.wires().iter().find(|wire| before.wire_path(wire) == vec![grid(8, 0), grid(8, 6)]).unwrap().id();
    h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire(wire));
    h.run();
    let start = screen(&h, grid(8, 0));
    let away = screen(&h, grid(20, 4));
    h.drag_at(start);
    h.run();
    h.hover_at(away);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.hover_at(start);
    h.run();
    h.drop_at(start);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), 2);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Detached horizontal and vertical pieces follow both pointer axes, including diagonal
/// travel. Preview, cancellation and the committed geometry preserve their length and shape.
#[test]
fn detached_pieces_translate_freely_in_both_axes() {
    for horizontal in [false, true] {
        for (dx, dy) in [(4, 0), (0, 4), (4, 4)] {
            let mut h = broken_harness_axis(horizontal);
            let point = |x, y| if horizontal { grid(y, x) } else { grid(x, y) };
            let from = point(8, 3);
            let to = Point { x_nm: from.x_nm + dx * GRID_NM, y_nm: from.y_nm + dy * GRID_NM };
            let before = h.state().session.snapshot().unwrap();
            let expected = [point(8, 0), point(8, 6)].map(|point| Point { x_nm: point.x_nm + dx * GRID_NM, y_nm: point.y_nm + dy * GRID_NM });
            let start = screen(&h, from);
            let end = screen(&h, to);
            h.drag_at(start);
            h.run();
            h.hover_at(end);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            let preview_points = expected.map(|point| screen(&h, point)).to_vec();
            assert!(
                h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Path(path)
                if path.points==preview_points && path.stroke.color==egui::epaint::ColorMode::Solid(pcbcraft_chrome::Tokens::get().accent))),
                "the detached preview must follow both pointer axes"
            );
            h.key_press(egui::Key::Escape);
            h.run();
            h.drop_at(end);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            h.drag_at(start);
            h.run();
            h.hover_at(end);
            h.run();
            h.drop_at(end);
            h.run();
            assert_eq!(h.state().message, None);
            assert!(routes(&h).contains(&expected.to_vec()));
            assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), 3);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Break is enabled for a straight wire with one attached pin and one free end, releases
/// the pin, then disables itself for the fully detached result. Departure and return remain
/// deliberate for both whole-piece dragging and its vacated pin's endpoint handle.
#[test]
fn break_on_one_attached_side_supports_free_movement_and_pin_return() {
    for handle in [false, true] {
        let mut h = symbol_harness();
        right_click(&mut h, grid(8, 0), egui::Vec2::ZERO);
        assert!(!h.get_by_label("Break").accesskit_node().is_disabled());
        click(&mut h, "Break");
        let before = h.state().session.snapshot().unwrap();
        let wire = before.wires().iter().find(|wire| before.wire_path(wire) == vec![grid(4, 0), grid(12, 0)]).unwrap().id();
        assert!(before.wire_moves_freely(&wire));
        right_click(&mut h, grid(8, 0), egui::Vec2::ZERO);
        assert!(h.get_by_label("Break").accesskit_node().is_disabled());
        h.key_press(egui::Key::Escape);
        h.run();
        h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire(wire.clone()));
        h.run();
        let from = if handle { grid(4, 0) } else { grid(8, 0) };
        let start = screen(&h, from);
        let clear = screen(&h, Point { y_nm: 8 * GRID_NM, ..from });
        h.drag_at(start);
        h.run();
        h.hover_at(clear);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        h.hover_at(start);
        h.run();
        h.drop_at(start);
        h.run();
        assert_eq!(h.state().message, None);
        let project = h.state().session.project().unwrap();
        let moved = project.wires().iter().find(|candidate| candidate.id() == wire).unwrap();
        assert_eq!(moved.terminals().len(), 1, "returning to the old pin reconnects only that side");
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Both ends of a single pin-to-pin segment detach from the context menu and can rejoin
/// after a free diagonal movement, leaving the component placements unchanged throughout.
#[test]
fn break_on_two_attached_sides_releases_both_pins() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.apply("project.new", &json!({"name":"Pin-to-pin break"})).unwrap();
    for x in [-12, 12] {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":grid(x,0).x_nm,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let from = WireEndpoint::Terminal(s.project().unwrap().symbols()[0].terminals()[1]);
    let to = WireEndpoint::Terminal(s.project().unwrap().symbols()[1].terminals()[0]);
    s.apply(DRAW_WIRE, &json!({"from":from.inspect(),"to":to.inspect(),"bends":[],"expected_revision":s.revision()})).unwrap();
    h.state_mut().browser.open = false;
    h.run();
    right_click(&mut h, grid(0, 0), egui::Vec2::ZERO);
    click(&mut h, "Break");
    let before = h.state().session.snapshot().unwrap();
    assert!(before.wires()[0].terminals().is_empty());
    let start = screen(&h, grid(0, 0));
    let clear = screen(&h, grid(4, 8));
    h.drag_at(start);
    h.run();
    h.hover_at(clear);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.hover_at(start);
    h.run();
    h.drop_at(start);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints(), [from, to]);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
