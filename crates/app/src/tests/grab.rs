//! Endpoint handles and click-to-place connected pickup use the existing shared geometry edits.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, DRAW_WIRE,
    schematic::{GRID_NM, Point},
};

/// Keep gesture fixtures away from floating panels while retaining the real canvas.
fn editor() -> Harness<'static, PcbApp> {
    let mut h = harness();
    new_project(&mut h);
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().browser.open = false;
    h.state_mut().properties_open = false;
    h.run();
    h
}

/// Original bent free wire, created through the shared command rather than fixture surgery.
fn free_wire() -> Harness<'static, PcbApp> {
    let mut h = editor();
    let revision = h.state().session.revision();
    h.state_mut().run(DRAW_WIRE,&json!({"from":{"x_nm":-8*GRID_NM,"y_nm":0},"to":{"x_nm":8*GRID_NM,"y_nm":6*GRID_NM},"bends":[{"x_nm":8*GRID_NM,"y_nm":0}],"expected_revision":revision}));
    h.run();
    h
}

/// Resolve the current view's transform instead of relying on a fixed camera scale.
fn screen(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect())
}

/// Both selection and the ready wire tool accept a handle drag slightly off the stroke.
/// Only the addressed endpoint moves; its ID and the opposite endpoint survive undo/redo.
#[test]
fn free_endpoint_drag_uses_the_full_handle_without_prior_selection() {
    for ready_wire in [false, true] {
        let mut h = free_wire();
        if ready_wire {
            click(&mut h, "Draw wire (W)");
        }
        let before = h.state().session.snapshot().unwrap();
        let wire = &before.wires()[0];
        let [endpoint, other] = wire.endpoints();
        let origin = before.endpoint_position(endpoint).unwrap();
        let target = Point { y_nm: origin.y_nm + 5 * GRID_NM, ..origin };
        let from = screen(&h, origin) + egui::vec2(0., 11.);
        let to = screen(&h, target) + egui::vec2(0., 11.);
        h.drag_at(from);
        h.run();
        h.hover_at(to);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Reshape { .. })));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        h.drop_at(to);
        h.run();
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.endpoint_position(endpoint), Some(target));
        assert_eq!(after.endpoint_position(other), before.endpoint_position(other));
        assert_eq!(after.wires()[0].endpoints(), wire.endpoints());
        click(&mut h, "Undo");
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        click(&mut h, "Redo");
        assert_same_content(&after, &h.state().session.snapshot().unwrap());
    }
}

/// G and the wire menu pick up an endpoint without a held button; hover stays a preview
/// until the next click commits exactly one edit.
#[test]
fn endpoint_grab_follows_the_pointer_until_click() {
    for context in [false, true] {
        let mut h = free_wire();
        let before = h.state().session.snapshot().unwrap();
        let endpoint = before.wires()[0].endpoints()[0];
        let origin = before.endpoint_position(endpoint).unwrap();
        let at = screen(&h, origin);
        h.hover_at(at);
        h.run();
        if context {
            right_click(&mut h, at);
            click(&mut h, "Grab");
        } else {
            h.key_press(Key::G);
            h.run();
        }
        assert!(matches!(&h.state().views.active.schematic.tool,Some(Tool::Reshape{drag,..}) if drag.pickup));
        let target = Point { y_nm: origin.y_nm + 5 * GRID_NM, ..origin };
        let to = screen(&h, target);
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click_canvas_at(&mut h, to);
        assert_eq!(h.state().session.project().unwrap().endpoint_position(endpoint), Some(target));
        assert!(h.state().views.active.schematic.tool.is_none());
    }
}

/// Connected G/menu pickup changes the symbol and attached wire geometry together, leaving
/// terminal identities and connectivity unchanged. M's separate disconnect workflow remains.
#[test]
fn symbol_grab_keeps_its_wires_and_commits_without_a_held_button() {
    for context in [false, true] {
        let mut h = harness();
        h.set_size(egui::vec2(1400., 900.));
        h.state_mut().browser.open = false;
        h.state_mut().properties_open = false;
        h.run();
        wiring_project(&mut h);
        let revision = h.state().session.revision();
        h.state_mut().run(
            CONNECT_PINS,
            &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
        );
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let origin = before.symbols()[0].position;
        let at = screen(&h, origin);
        if context {
            right_click(&mut h, at);
            click(&mut h, "Grab (keep connected)");
        } else {
            h.hover_at(at);
            h.run();
            h.key_press(Key::G);
            h.run();
        }
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Move { disconnect: false, pickup: true, .. })));
        let target = Point { y_nm: origin.y_nm + 5 * GRID_NM, ..origin };
        let to = screen(&h, target);
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click_canvas_at(&mut h, to);
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.symbols()[0].position, target);
        assert!(before.same_connectivity(&after));
        assert_eq!(
            before.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>(),
            after.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>()
        );
        click(&mut h, "Undo");
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        click(&mut h, "Redo");
        assert_same_content(&after, &h.state().session.snapshot().unwrap());
    }
}

/// Shared wired-symbol fixture leaves only the real canvas available for pickup gestures.
fn wired_editor() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().browser.open = false;
    h.state_mut().properties_open = false;
    h.run();
    wiring_project(&mut h);
    let revision = h.state().session.revision();
    h.state_mut().run(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
    );
    h.run();
    h
}

/// R changes only the picked-up pose until placement. G and held movement retain wires;
/// M leaves their original routes in place. Move+rotation undo together as one edit.
#[test]
fn grabbed_symbols_rotate_before_placement_with_one_undo_step() {
    for mode in ["grab", "move", "held"] {
        let mut h = wired_editor();
        // Start rotated so a quarter turn proves relative orientation rather than a reset to 90°.
        let revision = h.state().session.revision();
        h.state_mut().run(ROTATE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":revision}));
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        let origin = before.symbols()[0].position;
        let from = screen(&h, origin);
        h.hover_at(from);
        h.run();
        if mode == "held" {
            h.drag_at(from);
            h.run();
            h.hover_at(from + egui::vec2(10., 0.));
            h.run();
        } else {
            h.key_press(if mode == "grab" { Key::G } else { Key::M });
            h.run();
        }
        let target = Point { y_nm: origin.y_nm + 8 * GRID_NM, ..origin };
        let to = screen(&h, target);
        h.hover_at(to);
        h.run();
        h.key_press(Key::R);
        h.run();
        assert!(matches!(&h.state().views.active.schematic.tool, Some(Tool::Move {symbol,turns,..}) if (symbol.turns() + turns) % 4 == 2), "{mode}");
        assert_eq!(h.state().session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let preview = before.transformed_symbol(&before.symbols()[0].id(), target, Some(2), mode == "move", revision, true).unwrap();
        for wire in preview.wires() {
            let points: Vec<_> = preview.wire_path(wire).into_iter().map(|point| screen(&h, point)).collect();
            assert!(
                h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.points == points)),
                "{mode}: the painted wire must reach the rotated preview pin"
            );
        }
        if mode == "held" {
            h.drop_at(to);
            h.run();
        } else {
            click_canvas_at(&mut h, to);
        }
        assert_eq!(h.state().message, None, "{mode}");
        let after = h.state().session.snapshot().unwrap();
        assert_eq!((after.symbols()[0].position, after.symbols()[0].turns()), (target, 2), "{mode}");
        assert!(h.state().views.active.schematic.tool.is_none());
        if mode == "move" {
            assert_eq!(
                before.wires().iter().map(|wire| before.wire_path(wire)).collect::<Vec<_>>(),
                after.wires().iter().map(|wire| after.wire_path(wire)).collect::<Vec<_>>()
            );
            assert!(after.wires().iter().all(|wire| wire.terminals().iter().all(|pin| !before.symbols()[0].terminals().contains(pin))));
        } else {
            assert!(before.same_connectivity(&after));
            assert_eq!(
                before.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>(),
                after.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>()
            );
        }
        click(&mut h, "Undo");
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        click(&mut h, "Redo");
        assert_same_content(&after, &h.state().session.snapshot().unwrap());
    }
}

/// A full turn at the original position is a no-op in either pickup mode. R on an active
/// symbol preview takes priority over an unrelated named marker under the pointer.
#[test]
fn pickup_rotation_wraps_and_owns_the_shortcut_over_net_labels() {
    for disconnect in [false, true] {
        let mut h = wired_editor();
        let origin = h.state().session.project().unwrap().symbols()[0].position;
        let from = screen(&h, origin);
        h.hover_at(from);
        h.run();
        h.key_press(if disconnect { Key::M } else { Key::G });
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        for _ in 0..4 {
            h.key_press(Key::R);
            h.run();
        }
        assert!(matches!(&h.state().views.active.schematic.tool, Some(Tool::Move {symbol,turns,..}) if (symbol.turns() + turns) % 4 == 0));
        click_canvas_at(&mut h, from);
        assert_eq!(h.state().session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
    let mut h = wired_editor();
    let revision = h.state().session.revision();
    h.state_mut()
        .run(pcbcraft_engine::net_labels::ADD, &json!({"name":"TARGET","style":"local","x_nm":0,"y_nm":10*GRID_NM,"expected_revision":revision}));
    h.run();
    assert_eq!(h.state().message, None);
    let before = h.state().session.snapshot().unwrap();
    h.hover_at(screen(&h, before.symbols()[0].position));
    h.run();
    h.key_press(Key::G);
    h.run();
    let label = &before.net_labels()[0];
    let to = screen(&h, label.at) + egui::vec2(10., 0.);
    h.hover_at(to);
    h.run();
    assert!(crate::net_labels::hovered(&before, &h.state().views.active.camera, &h.ctx, to, h.state().views.active.filters));
    h.key_press(Key::R);
    h.run();
    assert!(matches!(&h.state().views.active.schematic.tool, Some(Tool::Move {symbol,turns,..}) if (symbol.turns() + turns) % 4 == 1));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Cancel/focus loss, release over a panel and intervening revisions discard both parts of
/// a rotated pickup without publishing an early turn or disconnection.
#[test]
fn rotated_pickups_cancel_without_changing_symbol_or_wiring() {
    for disconnect in [false, true] {
        for mode in ["escape", "right", "focus", "stale", "panel"] {
            let mut h = wired_editor();
            let before = h.state().session.snapshot().unwrap();
            h.hover_at(screen(&h, before.symbols()[0].position));
            h.run();
            h.key_press(if disconnect { Key::M } else { Key::G });
            h.run();
            h.hover_at(screen(&h, Point { y_nm: before.symbols()[0].position.y_nm + 8 * GRID_NM, ..before.symbols()[0].position }));
            h.run();
            h.key_press(Key::R);
            h.run();
            match mode {
                "escape" => h.key_press(Key::Escape),
                "right" => {
                    let to = h.ctx.input(|i| i.pointer.hover_pos()).unwrap();
                    right_click(&mut h, to);
                }
                "focus" => h.event(egui::Event::WindowFocused(false)),
                "panel" => {
                    click(&mut h, "View");
                }
                _ => {
                    h.state_mut().run("project.rename", &json!({"name":"Concurrent"}));
                }
            }
            h.run();
            if mode == "panel" {
                // A toolbar click cannot commit the pose; Escape can still discard it.
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                h.key_press(Key::Escape);
                h.run();
            }
            assert!(h.state().views.active.schematic.tool.is_none(), "{disconnect} {mode}");
            let after = h.state().session.snapshot().unwrap();
            assert_eq!(after.symbols()[0].position, before.symbols()[0].position);
            assert_eq!(after.symbols()[0].turns(), before.symbols()[0].turns());
            assert_eq!(
                after.wires().iter().map(|wire| after.wire_path(wire)).collect::<Vec<_>>(),
                before.wires().iter().map(|wire| before.wire_path(wire)).collect::<Vec<_>>()
            );
            if mode != "stale" {
                assert!(Arc::ptr_eq(&before, &after));
            }
        }
    }
}

/// Live MCP uses the same compound move command as a completed pickup, including stale
/// rejection and the editor's single Undo restoring position, orientation and connections.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_move_rotation_shares_the_pickup_transaction() {
    use crate::live_control::LiveEditor;
    use pcbcraft_mcp::live::call_editor;
    let mut h = wired_editor();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let origin = before.symbols()[0].position;
    let ctx = h.ctx.clone();
    let request = json!({"method":"engine.execute","params":{"command":MOVE_SYMBOL,"params":{
        "symbol":before.symbols()[0].id(),"x_nm":origin.x_nm,"y_nm":origin.y_nm+8*GRID_NM,"rotation":90,"expected_revision":revision
    }}});
    let result = call_editor(&mut LiveEditor { app: h.state_mut(), ctx: &ctx }, &request);
    assert!(result.is_ok(), "{result:?}");
    h.run();
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(after.symbols()[0].turns(), 1);
    assert_eq!(after.symbols()[0].position.y_nm, origin.y_nm + 8 * GRID_NM);
    assert!(before.same_connectivity(&after));
    assert!(call_editor(&mut LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Undo");
    assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// Several routes ending on one junction share one movable handle, retaining that identity
/// and each opposite endpoint while their end legs adjust together.
#[test]
fn shared_junction_grab_moves_all_attached_wire_ends() {
    let mut h = free_wire();
    let shared = h.state().session.project().unwrap().wires()[0].endpoints()[0];
    let revision = h.state().session.revision();
    h.state_mut()
        .run(DRAW_WIRE, &json!({"from":shared.inspect(),"to":{"x_nm":-8*GRID_NM,"y_nm":-6*GRID_NM},"bends":[],"expected_revision":revision}));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    assert_eq!(before.wires().iter().filter(|wire| wire.endpoints().contains(&shared)).count(), 2);
    let origin = before.endpoint_position(shared).unwrap();
    h.hover_at(screen(&h, origin));
    h.run();
    h.key_press(Key::G);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Reshape { .. })));
    let target = Point { x_nm: origin.x_nm - 4 * GRID_NM, ..origin };
    let to = screen(&h, target);
    click_canvas_at(&mut h, to);
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(after.endpoint_position(shared), Some(target));
    for wire in before.wires() {
        let retained = after.wires().iter().find(|next| next.id() == wire.id()).unwrap();
        assert_eq!(retained.endpoints(), wire.endpoints());
        for endpoint in wire.endpoints().into_iter().filter(|endpoint| *endpoint != shared) {
            assert_eq!(after.endpoint_position(endpoint), before.endpoint_position(endpoint));
        }
    }
    assert!(before.same_connectivity(&after));
    click(&mut h, "Undo");
    assert_same_content(&before, &h.state().session.snapshot().unwrap());
}

/// Coincident corners with different wire identities require explicit selection rather than
/// letting a G pickup silently choose a route from their drawing order.
#[test]
fn ambiguous_handle_grab_requires_an_explicit_wire_selection() {
    let mut h = editor();
    for (from, to) in [
        (Point { x_nm: -4 * GRID_NM, y_nm: 0 }, Point { x_nm: 0, y_nm: 4 * GRID_NM }),
        (Point { x_nm: 0, y_nm: -4 * GRID_NM }, Point { x_nm: 4 * GRID_NM, y_nm: 0 }),
    ] {
        let revision = h.state().session.revision();
        h.state_mut().run(DRAW_WIRE, &json!({"from":from,"to":to,"bends":[{"x_nm":0,"y_nm":0}],"auto_heal":false,"expected_revision":revision}));
    }
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.hover_at(screen(&h, Point { x_nm: 0, y_nm: 0 }));
    h.run();
    h.key_press(Key::G);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let selected = before.wires()[0].id();
    h.state_mut().select(Selection::one_wire(selected.clone()));
    h.key_press(Key::G);
    h.run();
    assert!(matches!(&h.state().views.active.schematic.tool,Some(Tool::Reshape{drag,..}) if drag.wire==selected));
    h.key_press(Key::Escape);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Cancellation, focus loss and competing revisions discard pointer-following drafts.
#[test]
fn endpoint_pickup_cancellation_and_stale_input_never_commit_geometry() {
    for mode in ["escape", "right", "focus", "stale"] {
        let mut h = free_wire();
        let original = h.state().session.snapshot().unwrap();
        let at = screen(&h, original.endpoint_position(original.wires()[0].endpoints()[0]).unwrap());
        h.hover_at(at);
        h.run();
        h.key_press(Key::G);
        h.run();
        let to = at + egui::vec2(0., 50.);
        h.hover_at(to);
        h.run();
        match mode {
            "escape" => h.key_press(Key::Escape),
            "right" => right_click(&mut h, to),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            _ => {
                h.state_mut().run("project.rename", &json!({"name":"Concurrent"}));
            }
        }
        h.run();
        assert!(h.state().views.active.schematic.tool.is_none(), "{mode}");
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.wire_path(&after.wires()[0]), original.wire_path(&original.wires()[0]), "{mode}");
        if mode != "stale" {
            assert!(Arc::ptr_eq(&original, &after));
        }
    }
}

/// Context gestures use actual secondary input, not direct command or selection injection.
fn right_click(h: &mut Harness<'_, PcbApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}
