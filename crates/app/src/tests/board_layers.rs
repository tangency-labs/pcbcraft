//! Layer activation/visibility use the existing controls and remain independent view choices.
use super::*;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, persistence::encode_documents};

/// A configured board exercises copper and edge choices without authored placement side effects.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({}));
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h
}

fn layer(h: &Harness<'_, PcbApp>, name: &str) -> String {
    h.state().session.project().unwrap().boards().next().unwrap().layers().into_iter().find(|layer| layer.name == name).unwrap().id.to_string()
}

/// Picker, full rows and status agree; the visibility checkbox never activates a different layer.
#[test]
fn active_layer_picker_and_rows_share_state_without_document_edits() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let top = layer(&h, "Top");
    let edge = layer(&h, "Board Edge");
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    click(&mut h, "Active layer");
    click(&mut h, "Board Edge");
    assert_eq!(h.state().views.active.board.active_layer, Some(edge.clone()));
    assert_eq!(h.get_by_label("Activate layer Board Edge").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    click(&mut h, "Draw lines on Board Edge");
    h.get_by_label("Line · Board Edge");
    click(&mut h, "Show layer Board Edge");
    assert_eq!(h.state().views.active.board.active_layer, Some(edge.clone()));
    assert!(h.state().views.active.board.hidden_layers.contains(&edge));
    h.get_by_label("Board Edge (hidden)");
    assert!(h.get_by_label("Draw lines on Board Edge").accesskit_node().is_disabled());
    click(&mut h, "Activate layer Top");
    assert_eq!(h.state().views.active.board.active_layer, Some(top));
    assert!(h.state().views.active.board.hidden_layers.contains(&edge));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Visibility actions reveal the active layer and share the same handler with live control.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn isolate_and_show_all_remain_view_local_and_reject_invalid_targets() {
    let mut h = board();
    let before = h.state().session.inspect();
    let edge = layer(&h, "Board Edge");
    click(&mut h, "Activate layer Board Edge");
    click(&mut h, "Show layer Board Edge");
    click(&mut h, "Isolate active");
    let records = h.state().session.project().unwrap().boards().next().unwrap().layers();
    assert_eq!(h.state().views.active.board.hidden_layers.len(), records.len() - 1);
    assert!(!h.state().views.active.board.hidden_layers.contains(&edge));
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    h.state_mut().run_window_command(&ctx, "view.set_layer_visibility", &json!({"view":view,"mode":"all"})).unwrap();
    h.run();
    assert!(h.state().views.active.board.hidden_layers.is_empty());
    h.state_mut().run_window_command(&ctx, "view.set_layer_visibility", &json!({"view":view,"mode":"active"})).unwrap();
    h.run();
    assert_eq!(h.state().views.active.board.hidden_layers.len(), records.len() - 1);
    click(&mut h, "Show all");
    assert!(h.state().views.active.board.hidden_layers.is_empty());
    assert!(h.state_mut().run_window_command(&ctx, "view.set_layer_visibility", &json!({"view":999,"mode":"active"})).is_err());
    assert_eq!(h.state().session.inspect(), before);
}

/// Compact resizing reveals the active row without requiring a hunt through the scrolling card.
#[test]
fn compact_layers_reveal_the_active_row_after_resize() {
    let mut h = board();
    click(&mut h, "Active layer");
    click(&mut h, "Board Edge");
    h.set_size(egui::vec2(760., 560.));
    h.run();
    let header = h.get_by_label("Collapse Layers").rect();
    let row = h.get_by_label("Activate layer Board Edge").rect();
    assert!(row.center().y > header.bottom());
    assert!(row.bottom() < 528., "active row is outside the compact card: {row:?}");
    let selector = h.get_by_label("Active layer").rect();
    assert!(egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(760., 175.)).contains_rect(selector));
}
