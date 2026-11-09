//! A marker dropped onto an existing end must drag the same as one wired after placement.
use super::*;
use pcbcraft_engine::{
    DRAW_WIRE, PLACE_SYMBOL,
    schematic::{GRID_NM, WireEndpoint},
};
use std::sync::Arc;

/// Place an LED, draw its wire first, then use the ribbon to put GND on the free end.
fn ground_on_existing_wire() -> Harness<'static, PcbApp> {
    let mut h = editor(1400.);
    h.state_mut().properties_open = false;
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"led","x_nm":-20*GRID_NM,"y_nm":0,"expected_revision":revision}));
    let pin = h.state().session.project().unwrap().symbols()[0].terminals()[1];
    let revision = h.state().session.revision();
    h.state_mut().run(DRAW_WIRE, &json!({"from":pin.inspect(),"to":{"x_nm":0,"y_nm":0},"bends":[],"expected_revision":revision}));
    h.run();
    click(&mut h, "Place ground");
    let origin = at(&h, Point { x_nm: 0, y_nm: 0 });
    click_canvas_at(&mut h, origin);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().message, None);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.net_labels()[0].at, Point { x_nm: 0, y_nm: 0 });
    assert!(matches!(project.wires()[0].endpoints()[1], WireEndpoint::Junction(_)));
    h
}

/// The user's wire-first/GND-second drag keeps the LED named GND. Its painted preview and
/// committed route agree, original wire identity survives, and one Undo restores all contacts.
#[test]
fn dragging_ground_placed_on_a_wire_keeps_the_connection() {
    for auto_heal in [false, true] {
        let mut h = ground_on_existing_wire();
        h.state_mut().preferences.auto_heal_wires = auto_heal;
        h.state_mut().parts.set_repeat(true);
        let before = h.state().session.snapshot().unwrap();
        let label = before.net_labels()[0].clone();
        let from = at(&h, label.at) + egui::vec2(5., 17.) * (h.state().views.active.camera.unit_step() / 10.);
        let delta = egui::vec2(81.28, 60.96);
        let to = from + delta;
        let target = h.state().views.active.camera.snap(at(&h, label.at) + delta, h.get_by_label("Schematic canvas").rect());
        h.drag_at(from);
        h.run();
        h.hover_at(from + egui::vec2(8., 0.));
        h.run();
        h.hover_at(to);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { drag: Some(_), .. })));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let expected = before.dragged_net_label(label.id, target, 0, h.state().session.revision(), auto_heal).unwrap();
        let points: Vec<_> = expected.wire_path(&expected.wires()[0]).into_iter().map(|point| at(&h, point)).collect();
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.points==points)));
        h.drop_at(to);
        h.run();
        assert_eq!(h.state().message, None);
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.net_labels()[0].at, target);
        assert_eq!(after.wires()[0].id(), before.wires()[0].id());
        assert_eq!(after.wires()[0].endpoints()[1], WireEndpoint::NetLabel(label.id));
        assert_eq!(after.symbols()[0].position, before.symbols()[0].position);
        let graph = after.electrical_connectivity();
        let pin = before.symbols()[0].terminals()[1];
        assert_eq!(graph.net_index_for(pin), graph.net_index_for_endpoint(WireEndpoint::NetLabel(label.id)));
        assert_eq!(graph.net_name(graph.net_index_for(pin).unwrap()), "GND");
        assert!(h.state().views.active.schematic.tool.is_none());
        assert_eq!(after.net_labels().len(), 1);
        click(&mut h, "Undo");
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        click(&mut h, "Redo");
        assert_same_content(&after, &h.state().session.snapshot().unwrap());
    }
}

/// M, property coordinates and an explicit live-MCP connected move use the same contact
/// transaction as held dragging, with the editor's Undo restoring the wire-first fixture.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ground_pickup_properties_and_live_moves_share_connected_geometry() {
    for mode in ["move", "properties", "live"] {
        let mut h = ground_on_existing_wire();
        let before = h.state().session.snapshot().unwrap();
        let label = &before.net_labels()[0];
        let target = Point { x_nm: 6 * GRID_NM, y_nm: 8 * GRID_NM };
        if mode == "live" {
            let ctx = h.ctx.clone();
            let revision = h.state().session.revision();
            let result = pcbcraft_mcp::live::call_editor(
                &mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx },
                &json!({"method":"engine.execute","params":{"command":engine::SET,"params":{
                    "id":label.object_id(),"name":"GND","style":"ground","x_nm":target.x_nm,"y_nm":target.y_nm,"keep_connected":true,"expected_revision":revision
                }}}),
            );
            assert!(result.is_ok(), "{result:?}");
            h.run();
        } else {
            let body = at(&h, label.at) + egui::vec2(5., 17.) * (h.state().views.active.camera.unit_step() / 10.);
            h.hover_at(body);
            h.run();
            h.key_press(if mode == "move" { Key::M } else { Key::E });
            h.run();
            if mode == "move" {
                let to = at(&h, target);
                click_canvas_at(&mut h, to);
            } else {
                for (field, value) in [("X", "300 mil"), ("Y", "400 mil")] {
                    h.get_by_label(field).click();
                    h.run();
                    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
                    h.get_by_label(field).type_text(value);
                    h.run();
                }
                click(&mut h, "Apply");
            }
        }
        assert_eq!(h.state().message, None, "{mode}");
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.net_labels()[0].at, target, "{mode}");
        assert_eq!(after.wires()[0].endpoints()[1], WireEndpoint::NetLabel(label.id), "{mode}");
        click(&mut h, "Undo");
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
    }
}

/// Capturing formerly positional contacts is preview-only; cancelled, obstructed, unfocused
/// and stale drags cannot publish a split, endpoint replacement or marker move.
#[test]
fn cancelling_ground_contact_capture_leaves_the_wire_and_marker_unchanged() {
    for mode in ["escape", "focus", "outside", "stale"] {
        let mut h = ground_on_existing_wire();
        let original = h.state().session.snapshot().unwrap();
        let from = at(&h, original.net_labels()[0].at) + egui::vec2(0., 17.) * (h.state().views.active.camera.unit_step() / 10.);
        let to = from + egui::vec2(81.28, 60.96);
        h.drag_at(from);
        h.run();
        h.hover_at(to);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { drag: Some(_), .. })));
        let mut release = to;
        match mode {
            "escape" => h.key_press(Key::Escape),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "outside" => release = egui::pos2(3., 3.),
            _ => h.state_mut().run("project.rename", &json!({"name":"Concurrent"})),
        }
        h.run();
        let before = h.state().session.snapshot().unwrap();
        h.drop_at(release);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "{mode}");
        assert_eq!(before.wires(), original.wires());
        assert_eq!(before.net_labels(), original.net_labels());
    }
}
