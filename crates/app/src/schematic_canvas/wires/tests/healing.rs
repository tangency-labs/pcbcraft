use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint};
use std::sync::Arc;

/// A part and a whole-wire drop both connect the coincident pin through shared commands;
/// previews never change the live snapshot and undo restores the disconnected state.
#[test]
fn part_and_whole_wire_drops_connect() {
    for whole_wire in [false, true] {
        let mut h = harness();
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":grid(14,10).x_nm,"y_nm":grid(14,10).y_nm,"expected_revision":revision}))
            .unwrap();
        h.run();
        let pin = h.state().session.project().unwrap().symbols()[0].terminals()[0];
        let (from, to) = if whole_wire {
            right_click(&mut h, grid(8, 3), egui::Vec2::ZERO);
            click(&mut h, "Select Wire");
            (grid(8, 3), grid(10, 7))
        } else {
            (grid(14, 10), grid(12, 6))
        };
        let before = h.state().session.snapshot().unwrap();
        let to = screen(&h, to);
        drag(&mut h, from, to);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        drop(&mut h, to);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints()[1], WireEndpoint::Terminal(pin));
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Drawing can finish on an interior leg without requiring Slice first; the branch is atomic.
#[test]
fn wire_tool_finishes_on_a_segment() {
    let mut h = harness();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Draw wire (W)");
    for point in [grid(-4, 8), grid(0, 0)] {
        let pos = screen(&h, point);
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
    }
    assert_eq!(h.state().message, None);
    assert!(h.state().views.active.tool.is_none());
    let project = h.state().session.project().unwrap();
    assert_eq!(project.wires().len(), 3);
    assert_eq!(project.connectivity().nets.len(), 1);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Ribbon buttons and caption-menu entries invoke the same selected-leg commands. Heal can
/// reconnect an old visually touching endpoint, and unavailable actions remain disabled.
#[test]
fn ribbon_wire_edits_work_from_buttons_and_caption_menu() {
    for caption in [false, true] {
        for (label, button, count) in
            [("Slice", "Slice selected wire", 2), ("Break", "Break selected wire segment", 2), ("Delete", "Delete selected wire or segment", 1)]
        {
            let mut h = harness();
            assert!(h.get_by_label(button).accesskit_node().is_disabled());
            let pos = screen(&h, grid(8, 3));
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run();
            }
            let before = h.state().session.snapshot().unwrap();
            if caption {
                click(&mut h, "Wire edits menu");
                click(&mut h, label);
            } else {
                click(&mut h, button);
            }
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.project().unwrap().wires().len(), count);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
    // Draw a free endpoint at an already placed pin, explicitly as coordinates to reproduce
    // a pre-existing touching connection. Heal targets exactly the selected wire.
    let mut h = harness();
    let revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":grid(14,10).x_nm,"y_nm":grid(14,10).y_nm,"expected_revision":revision}))
        .unwrap();
    let pin = h.state().session.project().unwrap().symbols()[0].terminals()[0];
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE, &json!({"from":grid(10,15),"to":grid(10,10),"bends":[],"expected_revision":revision})).unwrap();
    h.run();
    let wire = h.state().session.project().unwrap().wires().last().unwrap().id();
    h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire(wire.clone()));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Heal selected wires");
    assert_eq!(h.state().message, None);
    let healed = h.state().session.project().unwrap().wires().iter().find(|w| w.id() == wire).unwrap();
    assert!(healed.endpoints().contains(&WireEndpoint::Terminal(pin)));
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
