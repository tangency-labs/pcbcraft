//! Layer activation/visibility use the existing controls and remain independent view choices.
use super::*;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, persistence::encode_documents};

/// A configured board exercises copper and edge choices without authored placement side effects.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
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

/// Old default names display consistently after reopening, without rewriting layer identities,
/// drawings or saved bytes. Custom names remain visible as authored.
#[test]
fn legacy_outline_names_preserve_saved_geometry_and_custom_names() {
    for (saved_name, displayed_name) in [("Board Edge", "Board Outline"), ("Board outline", "Board Outline"), ("Panel perimeter", "Panel perimeter")]
    {
        let mut h = board();
        let outline = layer(&h, "Board Outline");
        let mut setup = h.state().session.inspect()["project"]["board"]["setup"].clone();
        for entry in setup["layers"].as_array_mut().unwrap() {
            if entry["id"] == outline {
                entry["name"] = json!(saved_name);
            }
        }
        let revision = h.state().session.revision();
        h.state_mut().session.apply(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision})).unwrap();
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(
                pcbcraft_engine::ADD_DRAWING,
                &json!({
                    "layer":outline,"points":[{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0}],
                    "closed":false,"expected_revision":revision
                }),
            )
            .unwrap();
        let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.state_mut().session.open_snapshot(pcbcraft_engine::persistence::decode_documents(&saved).unwrap());
        let before = h.state().session.inspect();
        h.run();
        click(&mut h, &format!("Activate layer {displayed_name}"));
        click(&mut h, "Draw lines on Board Outline");
        h.get_by_label(&format!("Line · {displayed_name}"));
        assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(outline.as_str()));
        assert_eq!(layer(&h, saved_name), outline);
        assert_eq!(h.state().session.inspect(), before);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// Picker, full rows and status agree; the visibility eye never activates a different layer.
#[test]
fn active_layer_picker_and_rows_share_state_without_document_edits() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let top = layer(&h, "Top");
    let edge = layer(&h, "Board Outline");
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    click(&mut h, "Active layer");
    click(&mut h, "Board Outline");
    assert_eq!(h.state().views.active.board.active_layer, Some(edge.clone()));
    assert_eq!(h.get_by_label("Activate layer Board Outline").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    click(&mut h, "Draw lines on Board Outline");
    h.get_by_label("Line · Board Outline");
    click(&mut h, "Show layer Board Outline");
    assert_eq!(h.state().views.active.board.active_layer, Some(edge.clone()));
    assert!(h.state().views.active.board.hidden_layers.contains(&edge));
    h.get_by_label("Board Outline (hidden)");
    assert!(h.get_by_label("Draw lines on Board Outline").accesskit_node().is_disabled());
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
    let edge = layer(&h, "Board Outline");
    click(&mut h, "Activate layer Board Outline");
    click(&mut h, "Show layer Board Outline");
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
    click(&mut h, "Board Outline");
    h.set_size(egui::vec2(760., 560.));
    h.run();
    let header = h.get_by_label("Collapse Layers").rect();
    let setup = h.get_by_label("Board setup…").rect();
    assert!(setup.top() >= header.bottom(), "setup action scrolled beneath the header: {setup:?}");
    assert!(setup.height() >= pcbcraft_chrome::Tokens::get().control_height);
    let visibility = h.get_by_label("Show all").rect();
    assert!(visibility.top() >= setup.bottom());
    let row = h.get_by_label("Activate layer Board Outline").rect();
    assert!(row.center().y > visibility.bottom());
    assert!(row.bottom() < 528., "active row is outside the compact card: {row:?}");
    let selector = h.get_by_label("Active layer").rect();
    assert!(egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(760., 175.)).contains_rect(selector));
}

/// Eyes have separate hit targets and toggle semantics; keyboard visibility changes never activate a row.
#[test]
fn visibility_eye_is_separate_from_layer_activation_and_keyboard_operable() {
    let mut h = board();
    click(&mut h, "Activate layer Board Outline");
    let active = h.state().views.active.board.active_layer.clone();
    let top = layer(&h, "Top");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let row = h.get_by_label("Activate layer Top").rect();
    let eye = h.get_by_label("Show layer Top").rect();
    assert!(eye.left() >= row.right(), "eye overlaps the activation target");
    h.get_by_label("Show layer Top").focus();
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert!(h.state().views.active.board.hidden_layers.contains(&top));
    assert_eq!(h.state().views.active.board.active_layer, active);
    assert_eq!(h.get_by_label("Show layer Top").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    click(&mut h, "Activate layer Top");
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    assert!(h.state().views.active.board.hidden_layers.contains(&top));
    click(&mut h, "Show layer Top");
    assert!(!h.state().views.active.board.hidden_layers.contains(&top));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Manual-based PCB shortcuts share picker order, wrap through hidden/technical layers and
/// resolve copper by side without changing visibility, authored bytes or another view.
#[test]
fn keyboard_layer_navigation_matches_the_picker_and_preserves_history() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let top = layer(&h, "Top");
    let bottom = layer(&h, "Bottom");
    let edge = layer(&h, "Board Outline");
    click(&mut h, "Show layer Bottom");
    let hidden = h.state().views.active.board.hidden_layers.clone();
    h.key_press(Key::Plus);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(bottom.clone()));
    h.key_press(Key::Plus);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(edge.clone()));
    h.key_press(Key::Minus);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(bottom.clone()));
    h.key_press(Key::PageUp);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    h.key_press(Key::PageDown);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(bottom));
    h.key_press_modifiers(Modifiers::SHIFT, Key::Equals);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(edge));
    h.key_press(Key::PageUp);
    h.run();
    let order = crate::board_panels::ordered_layers(h.state().session.project().unwrap().boards().next().unwrap());
    for _ in 0..order.len() {
        h.key_press(Key::Plus);
        h.run();
    }
    assert_eq!(h.state().views.active.board.active_layer, Some(top));
    assert_eq!(h.state().views.active.board.hidden_layers, hidden);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Text entry, open pickers and modal forms own their keys; layer switching expires edge drafts.
#[test]
fn layer_shortcuts_respect_fields_popups_and_discard_old_layer_gestures() {
    let mut h = board();
    let top = layer(&h, "Top");
    let edge = layer(&h, "Board Outline");
    click(&mut h, "Active layer");
    h.key_press(Key::Plus);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    h.key_press(Key::Escape);
    h.run();
    super::board_views::open_silkscreen(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Text height").click();
    h.run();
    h.key_press(Key::Minus);
    h.run();
    h.key_press(Key::PageDown);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer, Some(top.clone()));
    click(&mut h, "Cancel");
    click(&mut h, "Activate layer Board Outline");
    click(&mut h, "Draw rectangle on Board Outline");
    let canvas = h.get_by_label("PCB canvas").rect();
    click_canvas_at(&mut h, canvas.center());
    assert!(h.state().views.active.board.drawing.source().is_some());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::PageUp);
    h.run();
    assert!(h.state().views.active.board.drawing.source().is_none());
    assert_eq!(h.state().views.active.board.active_layer, Some(top));
    assert_ne!(h.state().views.active.board.active_layer, Some(edge));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Live layer navigation reaches the keyboard's view-local handler and refuses stale addresses.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_layer_navigation_targets_only_the_addressed_view() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let before = h.state().session.inspect();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let other = h.state().views.hosts[&h.state().views.main].layout.visible_views()[1];
    let old = h.state().views.view(other).unwrap().board.active_layer.clone();
    h.state_mut().run_window_command(&ctx, "view.navigate_layer", &json!({"view":view,"direction":"back"})).unwrap();
    h.run();
    assert_eq!(h.state().views.view(other).unwrap().board.active_layer, old);
    assert_eq!(h.state().views.view(crate::ViewId(view)).unwrap().board.active_layer, Some(layer(&h, "Bottom")));
    assert!(h.state_mut().run_window_command(&ctx, "view.navigate_layer", &json!({"view":999,"direction":"next"})).is_err());
    assert_eq!(h.state().session.inspect(), before);
}
