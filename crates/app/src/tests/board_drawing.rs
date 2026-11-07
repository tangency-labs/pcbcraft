//! Board Edge tests exercise canvas input and the same transactions used by MCP.
use super::*;
use pcbcraft_engine::{ADD_DRAWING, persistence::encode_documents, schematic::Point};

/// An unconfigured board exposes technical layers without inventing any copper topology.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({}));
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Activate layer Board Edge");
    h
}

fn drawings(h: &Harness<'_, PcbApp>) -> Vec<pcbcraft_engine::board::drawing::Drawing> {
    h.state().session.project().unwrap().boards().next().unwrap().drawings()
}

fn at(h: &Harness<'_, PcbApp>, x: i64, y: i64) -> egui::Pos2 {
    h.state().views.active.camera.screen(Point { x_nm: x, y_nm: y }, h.get_by_label("PCB canvas").rect())
}

/// Original physical pads overlap an edge, exercising input ownership rather than distant hits.
fn overlapping_footprint<'a>() -> Harness<'a, PcbApp> {
    let mut h = board();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    let symbol = h.state().session.project().unwrap().symbols().first().unwrap().id();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(
        ADD_DRAWING,
        &json!({"layer":layer,"points":[{"x_nm":-20_320_000,"y_nm":0},{"x_nm":20_320_000,"y_nm":0}],"closed":false,"expected_revision":revision}),
    );
    h.run();
    h
}

/// Placing an M-move on an edge must commit the footprint once, leaving edge geometry intact.
#[test]
fn footprint_move_placement_on_board_edge_keeps_input_ownership() {
    let mut h = overlapping_footprint();
    let edges = drawings(&h);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let origin = at(&h, 0, 0);
    h.hover_at(origin);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    let destination = at(&h, 10_160_000, 0);
    click_canvas_at(&mut h, destination);
    assert!(h.state().views.active.board.draft.is_none());
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap();
    assert_eq!(footprint.position, Point { x_nm: 10_160_000, y_nm: 0 });
    assert_eq!(drawings(&h), edges);
    assert_eq!(h.state().session.revision(), revision + 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A pad-over-edge drag moves the footprint rather than the edge, with one undoable edit.
#[test]
fn footprint_drag_overlapping_board_edge_keeps_input_ownership() {
    let mut h = overlapping_footprint();
    let edges = drawings(&h);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let edge_only = at(&h, 20_320_000, 0);
    click_canvas_at(&mut h, edge_only);
    assert!(h.state().views.active.board.drawing.selected.is_some());
    let pad = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().pads[0].center;
    let origin = at(&h, pad.x_nm, pad.y_nm);
    h.hover_at(origin);
    h.run();
    h.drag_at(origin);
    h.run();
    let destination = at(&h, pad.x_nm + 10_160_000, pad.y_nm + 5_080_000);
    h.hover_at(destination);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    assert!(h.state().views.active.board.drawing.source().is_none());
    assert!(h.state().views.active.board.drawing.selected.is_none());
    h.drop_at(destination);
    h.run();
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap();
    assert_eq!(footprint.position, Point { x_nm: 10_160_000, y_nm: 5_080_000 });
    assert_eq!(drawings(&h), edges);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Explicit drawing tools still accept canvas clicks over footprints instead of selecting them.
#[test]
fn explicit_board_edge_tool_can_draw_over_footprint() {
    let mut h = overlapping_footprint();
    click(&mut h, "Draw lines on Board Edge");
    let origin = at(&h, 0, 0);
    click_canvas_at(&mut h, origin);
    let destination = at(&h, 0, 10_160_000);
    click_canvas_at(&mut h, destination);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(drawings(&h).len(), 2);
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap();
    assert_eq!(footprint.position, Point { x_nm: 0, y_nm: 0 });
    assert!(h.state().views.active.board.draft.is_none());
}

/// An edge edit started away from a footprint retains its gesture when moved onto the pad.
#[test]
fn existing_board_edge_edit_can_finish_over_footprint() {
    let mut h = overlapping_footprint();
    let original = drawings(&h)[0].clone();
    let origin = at(&h, -20_320_000, 0);
    h.hover_at(origin);
    h.run();
    h.drag_at(origin);
    h.run();
    let destination = at(&h, 0, 0);
    h.hover_at(destination);
    h.run();
    assert!(h.state().views.active.board.drawing.source().is_some());
    assert!(h.state().views.active.board.draft.is_none());
    h.drop_at(destination);
    h.run();
    let edited = drawings(&h)[0].clone();
    assert_eq!(edited.id, original.id);
    assert_eq!(edited.points[0], Point { x_nm: 0, y_nm: 0 });
    assert_eq!(edited.points[1], original.points[1]);
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap();
    assert_eq!(footprint.position, Point { x_nm: 0, y_nm: 0 });
}

/// Rectangle clicks author one closed layer graphic; previews and cancellations author nothing.
#[test]
fn rectangle_canvas_create_cancel_delete_and_undo() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Draw rectangle on Board Edge");
    let a = at(&h, -10_000_000, -8_000_000);
    let b = at(&h, 10_000_000, 8_000_000);
    click_canvas_at(&mut h, a);
    h.hover_at(b);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.key_press(Key::Escape);
    h.run();
    assert!(drawings(&h).is_empty());
    click_canvas_at(&mut h, a);
    click_canvas_at(&mut h, b);
    assert_eq!(h.state().session.revision(), revision + 1);
    let graphic = drawings(&h)[0].clone();
    assert!(graphic.closed);
    assert_eq!(graphic.points.len(), 4);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().setup().is_none());
    let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::Escape);
    h.run();
    let corner = at(&h, graphic.points[0].x_nm, graphic.points[0].y_nm);
    click_canvas_at(&mut h, corner);
    assert_eq!(h.state().views.active.board.drawing.selected, Some(graphic.id.to_string()));
    h.key_press(Key::Delete);
    h.run();
    assert!(drawings(&h).is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Open polylines finish with Enter, close on their first point, and retain arbitrary vertices.
#[test]
fn lines_open_close_vertex_drag_and_stale_cancellation() {
    let mut h = board();
    click(&mut h, "Draw lines on Board Edge");
    let positions = [at(&h, -15_000_000, -8_000_000), at(&h, 10_000_000, -8_000_000), at(&h, 15_000_000, 8_000_000)];
    for pos in positions {
        click_canvas_at(&mut h, pos);
        h.run_steps(20);
    }
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(drawings(&h).len(), 1, "state {:?}, message {:?}", h.state().views.active.board.drawing, h.state().message);
    assert!(!drawings(&h)[0].closed);
    assert_eq!(drawings(&h)[0].points.len(), 3);
    h.key_press(Key::Escape);
    h.run();
    let graphic = drawings(&h)[0].clone();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let a = at(&h, graphic.points[1].x_nm, graphic.points[1].y_nm);
    let b = at(&h, graphic.points[1].x_nm + 2_000_000, graphic.points[1].y_nm - 2_000_000);
    h.hover_at(a);
    h.run();
    h.drag_at(a);
    h.run();
    h.hover_at(b);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.drop_at(b);
    h.run();
    let edited = drawings(&h)[0].clone();
    assert_ne!(edited.points[1], graphic.points[1]);
    assert_eq!(edited.points[0], graphic.points[0]);
    assert_eq!(edited.points[2], graphic.points[2]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Draw lines on Board Edge");
    for pos in positions {
        click_canvas_at(&mut h, pos);
        h.run_steps(20);
    }
    click_canvas_at(&mut h, positions[0]);
    assert_eq!(drawings(&h).len(), 2);
    assert!(drawings(&h).iter().any(|drawing| drawing.closed));
    click_canvas_at(&mut h, positions[0]);
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed during preview"})).unwrap();
    h.run();
    let concurrent = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click_canvas_at(&mut h, positions[1]);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), concurrent);
}

/// Hidden geometry cannot be selected, and hiding a layer cancels an in-progress vertex move.
#[test]
fn layer_visibility_is_view_only_and_cancels_editing() {
    let mut h = board();
    click(&mut h, "Draw rectangle on Board Edge");
    let a = at(&h, -10_000_000, -8_000_000);
    let b = at(&h, 10_000_000, 8_000_000);
    click_canvas_at(&mut h, a);
    click_canvas_at(&mut h, b);
    h.key_press(Key::Escape);
    h.run();
    let graphic = drawings(&h)[0].clone();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.hover_at(a);
    h.run();
    h.drag_at(a);
    h.run();
    h.hover_at(a + egui::vec2(20., 20.));
    h.run();
    h.state_mut().views.active.board.hidden_layers.insert(graphic.layer.to_string());
    h.run();
    h.drop_at(a + egui::vec2(20., 20.));
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert!(h.state().views.active.board.drawing.selected.is_none());
    click_canvas_at(&mut h, a);
    assert!(h.state().views.active.board.drawing.selected.is_none());
}

/// Live engine commands and canvas edits share the same board, revision checks and undo history.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_drawing_command_shares_gui_history() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    let board = h.state().session.project().unwrap().board_target().board;
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    {
        let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
        let request = json!({"method":"engine.execute","params":{"command":ADD_DRAWING,"params":{"board":board,"layer":layer,"points":[{"x_nm":-10_000_000,"y_nm":0},{"x_nm":10_000_000,"y_nm":0}],"closed":false,"expected_revision":revision}}});
        let updated = pcbcraft_mcp::live::call_editor(&mut editor, &request).unwrap();
        assert_eq!(updated["project"]["boards"][0]["drawings"].as_array().unwrap().len(), 1);
        assert!(pcbcraft_mcp::live::call_editor(&mut editor, &request).is_err());
    }
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Layer control addresses a view, shares the GUI handler, and never consumes document history.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_layer_choices_preserve_document_and_cancel_preview() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let before = h.state().session.inspect();
    click(&mut h, "Draw rectangle on Board Edge");
    let a = at(&h, 0, 0);
    click_canvas_at(&mut h, a);
    assert!(h.state().views.active.board.drawing.source().is_some());
    h.state_mut().run_window_command(&ctx, "view.set_layer", &json!({"view":view,"layer":layer,"visible":false})).unwrap();
    h.run();
    assert!(h.state().views.active.board.drawing.source().is_none());
    assert!(h.state().views.active.board.hidden_layers.contains(&layer));
    assert_eq!(h.state().session.inspect(), before);
    assert!(h.state_mut().run_window_command(&ctx, "view.set_layer", &json!({"view":view,"layer":"unknown","active":true})).is_err());
    assert_eq!(h.state().session.inspect(), before);
}

/// Whole-graphic moves retain shape, undo once, and never commit after release over chrome.
#[test]
fn drawing_moves_and_panel_release_use_one_captured_edit() {
    let mut h = board();
    click(&mut h, "Draw rectangle on Board Edge");
    let a = at(&h, -10_000_000, -8_000_000);
    let b = at(&h, 10_000_000, 8_000_000);
    click_canvas_at(&mut h, a);
    click_canvas_at(&mut h, b);
    h.key_press(Key::Escape);
    h.run();
    let graphic = drawings(&h)[0].clone();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click_canvas_at(&mut h, a);
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.drawing.source().is_some());
    let destination = a + egui::vec2(60., 50.);
    h.hover_at(destination);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click_canvas_at(&mut h, destination);
    let moved = drawings(&h)[0].clone();
    assert_eq!(moved.id, graphic.id);
    let delta = (moved.points[0].x_nm - graphic.points[0].x_nm, moved.points[0].y_nm - graphic.points[0].y_nm);
    assert_ne!(delta, (0, 0));
    for (after, before) in moved.points.iter().zip(&graphic.points) {
        assert_eq!((after.x_nm - before.x_nm, after.y_nm - before.y_nm), delta);
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.hover_at(a);
    h.run();
    h.drag_at(a);
    h.run();
    h.hover_at(destination);
    h.run();
    h.drop_at(h.get_by_label("Choose workspace").rect().center());
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    assert!(h.state().views.active.board.drawing.source().is_none());
}
