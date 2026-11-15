use super::*;
use crate::Tool;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint, wire::Anchor};
use std::sync::Arc;

/// Click the canvas without using tool shortcuts, preserving the real pointer event sequence.
fn click_at(h: &mut Harness<'_, PcbApp>, pos: Pos2) {
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// A terminal or wire node is reachable beyond the former eight-point radius. Hover shows
/// the magnetic wiring cursor, and clicking starts an exact-identity draft without W.
/// Starting/cancelling the draft leaves history untouched; finishing is one undoable edit.
#[test]
fn clicking_near_a_pin_or_wire_node_starts_a_wire_directly() {
    for (pin, scale) in [(false, 100), (true, 100), (false, 150), (true, 150)] {
        let mut h = harness();
        let endpoint = if pin {
            let revision = h.state().session.revision();
            h.state_mut()
                .session
                .apply(
                    PLACE_SYMBOL,
                    &json!({"definition":"resistor","x_nm":grid(-12,10).x_nm,
                "y_nm":grid(-12,10).y_nm,"expected_revision":revision}),
                )
                .unwrap();
            WireEndpoint::Terminal(h.state().session.project().unwrap().symbols()[0].terminals()[1])
        } else {
            h.state().session.project().unwrap().wires()[0].endpoints()[0]
        };
        h.state_mut().browser.open = false;
        h.state_mut().preferences.scale_percent = scale;
        h.run();
        let point = h.state().session.project().unwrap().endpoint_position(endpoint).unwrap();
        let center = screen(&h, point);
        let pointer = center + vec2(11.0, 0.0);
        let before = h.state().session.snapshot().unwrap();
        h.hover_at(pointer);
        h.run();
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if circle.center == center && circle.radius == pcbcraft_chrome::theme::WIRE_CURSOR_TARGET_RADIUS)));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click_at(&mut h, pointer);
        assert!(matches!(h.state().views.active.schematic.tool,
            Some(Tool::Wire { start: Some(Anchor::Existing(end)), .. }) if end == endpoint));
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click_at(&mut h, pointer);
        h.hover_at(screen(&h, grid(0, 12)));
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().message, None);
        assert!(h.state().session.project().unwrap().wires().iter().any(|wire| wire.endpoints().contains(&endpoint)));
        crate::tests::click(&mut h, "Undo");
        crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
    }
}

/// A near-stroke click still selects a segment outside wiring mode; W uses the same wider
/// stroke target to begin an explicitly connected branch at the projected point.
#[test]
fn wider_wire_targets_preserve_segment_selection_and_branch_starts() {
    let mut h = harness();
    h.state_mut().browser.open = false;
    h.run();
    let pointer = screen(&h, grid(0, 0)) + vec2(0.0, 9.0);
    click_at(&mut h, pointer);
    assert!(h.state().views.active.schematic.selected_leg.is_some());
    assert!(h.state().views.active.schematic.tool.is_none());
    let before = h.state().session.snapshot().unwrap();
    h.key_press(egui::Key::W);
    h.run();
    click_at(&mut h, pointer);
    assert!(matches!(h.state().views.active.schematic.tool,
        Some(Tool::Wire { start: Some(Anchor::Split { at, .. }), .. }) if at == grid(0,0)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Coincident terminals remain ambiguous despite larger targets; neither direct clicking nor
/// the active Wire tool may silently choose one of their distinct electrical identities.
#[test]
fn larger_targets_do_not_connect_ambiguous_nodes() {
    let mut h = harness();
    for _ in 0..2 {
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(
                PLACE_SYMBOL,
                &json!({"definition":"resistor","x_nm":grid(-12,10).x_nm,
            "y_nm":grid(-12,10).y_nm,"expected_revision":revision}),
            )
            .unwrap();
    }
    h.state_mut().browser.open = false;
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let pointer = screen(&h, grid(-8, 10));
    click_at(&mut h, pointer);
    assert!(h.state().views.active.schematic.tool.is_none());
    h.key_press(egui::Key::W);
    h.run();
    click_at(&mut h, pointer);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A direct node click in either pane activates that view's draft, including the first click
/// in an inactive pane. The other pane keeps its own input state and the shared project is clean.
#[test]
fn direct_node_starts_belong_to_the_clicked_split_pane() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            let end = h.state().session.project().unwrap().wires()[0].endpoints()[1];
            let before = h.state().session.snapshot().unwrap();
            let pointer = view(&h, id).camera.screen(grid(8, 6), canvas(&h, id)) + vec2(11.0, 0.0);
            click_at(&mut h, pointer);
            assert!(matches!(view(&h,id).schematic.tool,
                Some(Tool::Wire {start:Some(Anchor::Existing(target)),..}) if target==end));
            assert!(view(&h, 1 - id).schematic.tool.is_none());
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Occupied pins and junctions remain selectable with an ordinary click. W explicitly starts
/// a branch at the hovered identity without another click, and its green cursor uses wire ink.
/// Completing and undoing the branch preserves the original connection.
#[test]
fn connected_nodes_require_explicit_w_and_start_at_the_pointer() {
    for pin in [false, true] {
        let mut h = harness();
        let endpoint = if pin {
            let s = &mut h.state_mut().session;
            s.apply("project.new", &json!({"name":"Connected pin"})).unwrap();
            s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":grid(-12,0).x_nm,"y_nm":0,"expected_revision":s.revision()})).unwrap();
            let end = WireEndpoint::Terminal(s.project().unwrap().symbols()[0].terminals()[1]);
            s.apply(DRAW_WIRE, &json!({"from":end.inspect(),"to":grid(10,8),"bends":[grid(10,0)],"expected_revision":s.revision()})).unwrap();
            end
        } else {
            let s = &mut h.state_mut().session;
            let end = s.project().unwrap().wires()[0].endpoints()[1];
            s.apply(DRAW_WIRE, &json!({"from":end.inspect(),"to":grid(20,6),"bends":[],"expected_revision":s.revision()})).unwrap();
            end
        };
        h.state_mut().browser.open = false;
        h.run();
        let point = h.state().session.project().unwrap().endpoint_position(endpoint).unwrap();
        let before = h.state().session.snapshot().unwrap();
        let pointer = screen(&h, point);
        click_at(&mut h, pointer);
        assert!(h.state().views.active.schematic.tool.is_none());
        h.key_press(egui::Key::W);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool,
            Some(Tool::Wire{start:Some(Anchor::Existing(end)),..}) if end==endpoint));
        let center = screen(&h, point);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if circle.center==center && circle.radius==pcbcraft_chrome::theme::WIRE_CURSOR_TARGET_RADIUS
                && circle.stroke.color==pcbcraft_chrome::Tokens::get().schematic_wire)));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let to = Point { y_nm: point.y_nm + 6 * GRID_NM, ..point };
        h.hover_at(screen(&h, to));
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().message, None);
        assert_eq!(
            h.state().session.project().unwrap().wires().iter().filter(|wire| wire.endpoints().contains(&endpoint)).count(),
            if pin { 2 } else { 3 }
        );
        crate::tests::click(&mut h, "Undo");
        crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
    }
}

/// W starts immediately at a hovered grid point; toolbar activation still waits for its first
/// canvas click. Neither activation creates document data or an undo step.
#[test]
fn w_starts_on_the_hovered_grid_and_toolbar_waits_for_a_click() {
    let mut h = harness();
    h.state_mut().browser.open = false;
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.hover_at(screen(&h, grid(-4, 12)));
    h.run();
    h.key_press(egui::Key::W);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::Wire{start:Some(Anchor::Free(point)),..})if point==grid(-4,12)));
    h.key_press(egui::Key::Escape);
    h.run();
    crate::tests::click(&mut h, "Draw wire (W)");
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
