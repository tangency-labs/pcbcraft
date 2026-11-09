use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint};
use std::sync::Arc;

/// Retracing the active W draft must shorten its painted route before commit, including
/// the vertical reversal in the reported two-resistor example. The preview is disposable.
#[test]
fn new_wire_preview_retracts_self_overlap_before_commit() {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({"name":"Retrace"})).unwrap();
    h.state_mut().browser.open = false;
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.key_press(egui::Key::W);
    h.run();
    for point in [grid(-8, 0), grid(-6, 0), grid(-6, 5), grid(-2, 5), grid(-2, 0)] {
        let pos = screen(&h, point);
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
    }
    h.hover_at(screen(&h, grid(-2, 9)));
    h.run();
    let expected = vec![grid(-8, 0), grid(-6, 0), grid(-6, 5), grid(-2, 5), grid(-2, 9)];
    let points: Vec<_> = expected.iter().map(|&point| screen(&h, point)).collect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(pcbcraft_chrome::Tokens::get().accent) && path.points == points)),
        "the active preview should remove the doubled-back vertical spur");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![expected]);
    let after = h.state().session.snapshot().unwrap();
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
    click(&mut h, "Redo");
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
}

/// Each click retains its own reversible draft checkpoint even when retracing removes a
/// corner. Disabling automatic healing keeps the explicit detour in both preview and commit.
#[test]
fn retraced_draft_corners_keep_backspace_and_cleanup_off_behavior() {
    let from = grid(0, 0);
    for automatic in [false, true] {
        let mut draft = super::super::Draft::default();
        assert!(draft.corner(from, grid(0, 10), automatic));
        assert!(draft.corner(from, grid(0, 3), automatic));
        let expected = if automatic { vec![from, grid(0, 3), grid(6, 3)] } else { vec![from, grid(0, 10), grid(0, 3), grid(6, 3)] };
        assert_eq!(draft.path(from, grid(6, 3), automatic), Some(expected));
        assert!(draft.back());
        assert_eq!(draft.path(from, grid(6, 10), automatic), Some(vec![from, grid(0, 10), grid(6, 10)]));
        assert!(draft.back());
        assert!(!draft.back());
    }
}

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
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { start: None, .. })));
    let project = h.state().session.project().unwrap();
    assert_eq!(project.wires().len(), 3);
    assert_eq!(project.connectivity().nets.len(), 1);
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// Reproduce W drawing past a horizontal wire then returning to it: finishing the branch
/// removes the little U-shaped return in the same undo step and preserves the trunk's tail.
#[test]
fn wire_tool_heals_a_return_to_the_connected_trunk() {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({"name":"Return bend"})).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE, &json!({"from":grid(2,12),"to":grid(14,12),"bends":[],"expected_revision":revision})).unwrap();
    h.state_mut().browser.open = false;
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let start = screen(&h, grid(-10, 0));
    h.hover_at(start);
    h.run();
    h.key_press(egui::Key::W);
    h.run();
    for point in [grid(-10, 0), grid(6, 13), grid(8, 12)] {
        let pos = screen(&h, point);
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
    }
    assert_eq!(h.state().message, None);
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { start: None, .. })));
    assert_eq!(routes(&h), vec![vec![grid(2, 12), grid(6, 12)], vec![grid(6, 12), grid(14, 12)], vec![grid(-10, 0), grid(6, 0), grid(6, 12)]]);
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// The remembered Preferences toggle affects actual W gestures without adding document history.
/// Off retains a loop; explicit Heal still removes it and Undo brings the loop back.
#[test]
fn auto_heal_preferences_control_same_wire_shortcuts_and_persist() {
    for automatic in [false, true] {
        let mut h = harness();
        h.state_mut().session.apply("project.new", &json!({"name":"Optional healing"})).unwrap();
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(
                DRAW_WIRE,
                &json!({"from":grid(-10,0),"to":grid(20,12),"bends":[grid(6,0),grid(6,6),grid(-4,6),grid(-4,12)],"expected_revision":revision}),
            )
            .unwrap();
        h.state_mut().browser.open = false;
        h.run();
        assert!(h.state().preferences.auto_heal_wires);
        let before = h.state().session.snapshot().unwrap();
        if !automatic {
            click(&mut h, "Preferences");
            click(&mut h, "Schematic");
            click(&mut h, "Enabled");
            click(&mut h, "OK");
            assert!(!h.state().preferences.auto_heal_wires);
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            let restored = crate::preferences::Preferences::parse(&h.state().preferences.to_text());
            assert!(!restored.auto_heal_wires);
        }
        h.hover_at(screen(&h, grid(6, 6)));
        h.run();
        h.key_press(egui::Key::W);
        h.run();
        for point in [grid(6, 6), grid(6, 12)] {
            let pos = screen(&h, point);
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run();
            }
        }
        assert_eq!(h.state().message, None);
        assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { start: None, .. })));
        assert_eq!(h.state().session.project().unwrap().wires().len(), if automatic { 2 } else { 5 });
        if !automatic {
            let looped = h.state().session.snapshot().unwrap();
            let wire = looped.wires().last().unwrap().id();
            h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire(wire));
            h.run();
            click(&mut h, "Modify menu");
            click(&mut h, "Heal");
            assert_eq!(h.state().session.project().unwrap().wires().len(), 2);
            assert!(!h.state().preferences.auto_heal_wires);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&looped, &h.state().session.snapshot().unwrap()));
        }
    }
    assert!(crate::preferences::Preferences::parse("{}").auto_heal_wires);
}

/// The ported Modify menu invokes the same selected-leg commands. Heal can
/// reconnect an old visually touching endpoint, and unavailable actions remain disabled.
#[test]
fn modify_menu_wire_edits_preserve_selected_leg_commands() {
    for (label, count) in [("Slice", 2), ("Delete", 1)] {
        let mut h = harness();
        click(&mut h, "Modify menu");
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        h.key_press(egui::Key::Escape);
        h.run();
        let pos = screen(&h, grid(8, 3));
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
        let before = h.state().session.snapshot().unwrap();
        click(&mut h, "Modify menu");
        click(&mut h, label);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.project().unwrap().wires().len(), count);
        click(&mut h, "Undo");
        crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
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
    click(&mut h, "Modify menu");
    click(&mut h, "Heal");
    assert_eq!(h.state().message, None);
    let healed = h.state().session.project().unwrap().wires().iter().find(|w| w.id() == wire).unwrap();
    assert!(healed.endpoints().contains(&WireEndpoint::Terminal(pin)));
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
