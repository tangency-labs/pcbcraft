//! Accepted connections remain a presentation overlay over the actual physical board.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_FOOTPRINT_TRANSFORM, board::update::INITIALIZE_BASELINE, persistence::encode_documents,
};

/// Author and accept one connection through production commands before opening its PCB view.
fn board<'a>(accept: bool) -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    let mut symbols = Vec::new();
    for index in 0..2 {
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
        let symbol = h.state().session.project().unwrap().symbols().last().unwrap().id();
        symbols.push(symbol.clone());
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
    }
    let revision = h.state().session.revision();
    h.state_mut().run(
        CONNECT_PINS,
        &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":revision}),
    );
    if accept {
        let revision = h.state().session.revision();
        h.state_mut().run(INITIALIZE_BASELINE, &json!({"expected_revision":revision}));
    }
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    h
}

/// Extract only the chrome-owned thin accepted-connection strokes from the PCB canvas.
fn airwires(h: &Harness<'_, PcbApp>) -> Vec<[egui::Pos2; 2]> {
    let canvas = h.get_by_label("PCB canvas").rect();
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, stroke }
                if shape.clip_rect == canvas && stroke.width == 1. && stroke.color == Tokens::get().board_ink.gamma_multiply(0.55) =>
            {
                Some(*points)
            }
            _ => None,
        })
        .collect()
}

/// Real canvas strokes use engine-mapped centers, remain behind pads and track authored transforms/Undo.
#[test]
fn ratsnest_projects_accepted_pad_centers_and_recomputes_after_move_and_undo() {
    let mut h = board(true);
    assert_eq!(airwires(&h).len(), 1);
    h.get_by_label("Board Edge · 1 airwire");
    let wire_index = h.output().shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if stroke.width == 1. && stroke.color == Tokens::get().board_ink.gamma_multiply(0.55))).unwrap();
    let pad_index =
        h.output().shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.fill == Tokens::get().board_front)).unwrap();
    assert!(wire_index < pad_index);

    let canvas = h.get_by_label("PCB canvas").rect();
    let cached = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    let line = &cached.lines[0];
    assert_eq!(
        airwires(&h)[0],
        [h.state().views.active.camera.screen(line.from.position, canvas), h.state().views.active.camera.screen(line.to.position, canvas)]
    );
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let footprint = line.from.footprint.clone();
    h.state_mut()
        .run(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":footprint,"x_nm":5_000_000,"y_nm":4_000_000,"rotation":90,"expected_revision":revision}));
    h.run();
    let moved = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    assert_ne!(cached.lines, moved.lines);
    assert_eq!(cached.source, moved.source);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.as_ref(), cached.as_ref());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DELETE_FOOTPRINTS, &json!({"footprints":[line.from.footprint],"expected_revision":revision}));
    h.run();
    assert!(airwires(&h).is_empty());
    assert_eq!(h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.missing.len(), 1);
    h.get_by_label("Board Edge · 0 airwires · 1 missing endpoint");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    let after_undo = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 2_000_000, y_nm: 1_000_000 });
    h.run();
    assert!(Arc::ptr_eq(&h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1, &after_undo));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The local toggle is restored per view, uses the live command handler too, and never edits the design.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ratsnest_visibility_is_saved_per_view_and_exposed_to_live_control() {
    let mut h = board(true);
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Ratsnest");
    assert!(airwires(&h).is_empty());
    let saved = h.state().views.active.saved_state(&h.state().session).unwrap();
    let mut restored = crate::editor_view::EditorView::from_state(ViewId(99), &saved, &h.state().session);
    restored.sync(&h.state().session);
    assert!(restored.board.hidden_layers.contains("ratsnest"));
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":view,"visible":true})).unwrap();
    h.run();
    assert_eq!(airwires(&h).len(), 1);
    assert!(restored.board.hidden_layers.contains("ratsnest"));
    assert!(h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":999,"visible":false})).is_err());
    h.set_size(egui::vec2(760., 560.));
    h.run();
    let checkbox = h.get_by_label("Ratsnest").rect();
    let toolbar = h.get_by_label("Show all").rect();
    assert!((checkbox.center().y - toolbar.center().y).abs() < 2.);
    assert!(checkbox.right() <= h.get_by_label("Collapse Layers").rect().right());
    assert!(checkbox.bottom() < 540.);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Proposed connections are neither drawn nor counted until the board has accepted inputs.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ratsnest_controls_and_counts_are_unavailable_before_acceptance() {
    let mut h = board(false);
    assert!(airwires(&h).is_empty());
    assert!(h.query_all_by_label("Ratsnest").next().is_none());
    assert!(h.state().views.active.board.ratsnest_cache.is_none());
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let before = h.state().session.inspect();
    assert!(h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":view,"visible":true})).is_err());
    assert_eq!(h.state().session.inspect(), before);
}
