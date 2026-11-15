use super::*;
use crate::Tool;
use pcbcraft_engine::{persistence, wire::Anchor};
use std::sync::Arc;

/// Carry Ctrl through complete frames, as a held physical modifier rather than a key combo
/// that releases its modifiers before the canvas paints.
fn point_with_ctrl(h: &mut Harness<'_, PcbApp>, pos: Pos2, held: bool) {
    h.event(egui::Event::ModifiersChanged(if held { Modifiers::CTRL } else { Modifiers::NONE }));
    h.event(egui::Event::PointerMoved(pos));
    h.run();
}

/// Place a corner or endpoint while Ctrl remains held after button release.
fn click_ctrl(h: &mut Harness<'_, PcbApp>, pos: Pos2) {
    point_with_ctrl(h, pos, true);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::CTRL });
        h.run();
    }
}

/// Send a wire-tool control without releasing the snap override.
fn key_ctrl(h: &mut Harness<'_, PcbApp>, key: egui::Key) {
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::CTRL });
        h.run();
    }
}

/// Locate the actual blue wire preview's end, independently of the cursor primitives.
fn preview_ends_at(h: &Harness<'_, PcbApp>, end: Pos2) -> bool {
    h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape,egui::Shape::Path(path)
        if path.stroke.color==egui::epaint::ColorMode::Solid(pcbcraft_chrome::Tokens::get().accent)
        && path.points.last().is_some_and(|p|p.distance(end)<0.001))
    })
}

/// Ctrl bypasses grid snapping for the start, clicked corners, preview and committed free end.
/// Releasing it restores the grid immediately; no document/preference change occurs until
/// completion. Ctrl+Enter, undo/redo and native persistence preserve the off-grid route.
#[test]
fn ctrl_temporarily_bypasses_grid_for_the_complete_wire_gesture() {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({"name":"Off grid"})).unwrap();
    h.state_mut().browser.open = false;
    h.run();
    click(&mut h, "Draw wire (W)");
    let before = h.state().session.snapshot().unwrap();
    let spacing = h.state().views.active.camera.snap_nm();
    let start = screen(&h, grid(-8, 8)) + vec2(3.0, 2.0);
    click_ctrl(&mut h, start);
    let from = match h.state().views.active.schematic.tool {
        Some(Tool::Wire { start: Some(Anchor::Free(point)), .. }) => point,
        _ => panic!("Ctrl-click starts a free wire"),
    };
    assert!(from.x_nm % spacing != 0 && from.y_nm % spacing != 0, "holding Ctrl must bypass the placement grid");
    assert!(screen(&h, from).distance(start) < 0.001);
    let corner = screen(&h, grid(0, 8)) + vec2(4.0, 3.0);
    click_ctrl(&mut h, corner);
    let end = screen(&h, grid(6, 14)) + vec2(2.0, 4.0);
    point_with_ctrl(&mut h, end, true);
    assert!(preview_ends_at(&h, end));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let snapped = h.state().views.active.camera.snap(end, h.get_by_label("Schematic canvas").rect());
    assert!(preview_ends_at(&h, screen(&h, snapped)));
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    h.run();
    assert!(preview_ends_at(&h, end));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().views.active.camera.snap_nm(), spacing);
    key_ctrl(&mut h, egui::Key::Enter);
    assert_eq!(h.state().message, None);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    let project = h.state().session.project().unwrap();
    let route = project.wire_path(&project.wires()[0]);
    assert_eq!(route[0], from);
    assert!(route.contains(&h.state().views.active.camera.unsnapped(corner, h.get_by_label("Schematic canvas").rect())));
    assert!(screen(&h, *route.last().unwrap()).distance(end) < 0.001);
    assert!(route.windows(2).all(|leg| leg[0].x_nm == leg[1].x_nm || leg[0].y_nm == leg[1].y_nm));
    let after = h.state().session.snapshot().unwrap();
    let saved = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
    click(&mut h, "Redo");
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
}

/// Grid bypass still finishes on an exact pin identity or splits a stroke at its projected
/// off-grid coordinate, rather than creating a disconnected free end nearby.
#[test]
fn ctrl_preserves_pin_and_stroke_connections() {
    for pin in [false, true] {
        let mut h = harness();
        let endpoint = if pin {
            let s = &mut h.state_mut().session;
            s.apply(
                pcbcraft_engine::PLACE_SYMBOL,
                &json!({"definition":"resistor","x_nm":grid(-12,0).x_nm,
                "y_nm":0,"expected_revision":s.revision()}),
            )
            .unwrap();
            Some(pcbcraft_engine::schematic::WireEndpoint::Terminal(s.project().unwrap().symbols()[0].terminals()[1]))
        } else {
            None
        };
        h.state_mut().browser.open = false;
        h.run();
        click(&mut h, "Draw wire (W)");
        let start = screen(&h, grid(-12, 12)) + vec2(3.0, 2.0);
        click_ctrl(&mut h, start);
        let point = endpoint.map_or(grid(0, 0), |end| h.state().session.project().unwrap().endpoint_position(end).unwrap());
        let target = screen(&h, point) + vec2(4.0, 3.0);
        let expected =
            if pin { point } else { Point { y_nm: 0, ..h.state().views.active.camera.unsnapped(target, h.get_by_label("Schematic canvas").rect()) } };
        click_ctrl(&mut h, target);
        assert_eq!(h.state().message, None);
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
        let project = h.state().session.project().unwrap();
        let connected = project
            .wires()
            .iter()
            .find(|wire| {
                let route = project.wire_path(wire);
                screen(&h, route[0]).distance(start) < 0.001 || screen(&h, *route.last().unwrap()).distance(start) < 0.001
            })
            .unwrap();
        assert!(project.wire_path(connected).contains(&expected));
        if let Some(end) = endpoint {
            assert!(connected.endpoints().contains(&end));
        } else {
            let junction = connected.endpoints().into_iter().find(|&end| project.endpoint_position(end) == Some(expected)).unwrap();
            assert_eq!(project.wires().iter().filter(|wire| wire.endpoints().contains(&junction)).count(), 3);
            assert_ne!(expected.x_nm % h.state().views.active.camera.snap_nm(), 0);
        }
    }
}

/// While Ctrl remains held, elbow flip, per-click backtracking and cancellation still work
/// without touching the document or changing the view's grid spacing.
#[test]
fn held_ctrl_keeps_wire_controls_available() {
    let mut h = harness();
    h.state_mut().browser.open = false;
    h.run();
    click(&mut h, "Draw wire (W)");
    let before = h.state().session.snapshot().unwrap();
    let start = screen(&h, grid(-12, 12)) + vec2(3.0, 2.0);
    click_ctrl(&mut h, start);
    let corner = screen(&h, grid(0, 12)) + vec2(4.0, 3.0);
    click_ctrl(&mut h, corner);
    let orientation = h.state().views.active.schematic.wire_draft.horizontal_first;
    key_ctrl(&mut h, egui::Key::Space);
    assert_ne!(h.state().views.active.schematic.wire_draft.horizontal_first, orientation);
    key_ctrl(&mut h, egui::Key::Backspace);
    assert!(h.state().views.active.schematic.wire_draft.corners.is_empty());
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: Some(_), .. })));
    key_ctrl(&mut h, egui::Key::Escape);
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Whether the wiring crosshair's right arm, in the wire color, starts beside `pointer`.
fn crosshair_at(h: &Harness<'_, PcbApp>, pointer: Pos2) -> bool {
    let arm = pointer + vec2(pcbcraft_chrome::theme::WIRE_CURSOR_GAP, 0.0);
    h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
            if stroke.color == pcbcraft_chrome::Tokens::get().schematic_wire && points[0].distance(arm) < 0.001)
    })
}

/// Whether the snapped free-point dot is painted at `at`.
fn snap_dot_at(h: &Harness<'_, PcbApp>, at: Pos2) -> bool {
    h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Circle(circle)
            if circle.radius == pcbcraft_chrome::theme::WIRE_CURSOR_SNAP_RADIUS && circle.center.distance(at) < 0.001)
    })
}

/// Within one grid cell the crosshair follows every pointer move, before and during a wire,
/// while the snap dot and the route preview stay on the grid point a click would use.
#[test]
fn the_wiring_crosshair_follows_the_pointer_between_grid_points() {
    let mut h = harness();
    h.state_mut().browser.open = false;
    h.run();
    click(&mut h, "Draw wire (W)");
    let step = h.state().views.active.camera.snap_step();
    let rect = h.get_by_label("Schematic canvas").rect();
    let start = screen(&h, grid(-12, 12));
    let end = screen(&h, grid(-4, 12));
    for started in [false, true] {
        if started {
            h.hover_at(start);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: start, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run();
            }
            assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: Some(Anchor::Free(_)), .. })));
        }
        let snapped = h.state().views.active.camera.snap(end, rect);
        for fraction in [0.1, 0.25, 0.4] {
            let pointer = end + vec2(step * fraction, -step * fraction);
            assert_eq!(h.state().views.active.camera.snap(pointer, rect), snapped);
            h.hover_at(pointer);
            h.run();
            assert!(crosshair_at(&h, pointer), "the crosshair must track the pointer at {fraction} of a grid step");
            assert!(snap_dot_at(&h, end), "the snap marker must stay on the grid point");
            assert_eq!(started, preview_ends_at(&h, end));
        }
    }
}
