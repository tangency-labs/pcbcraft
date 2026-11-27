use super::*;

/// Collections start collapsed, empty kinds disappear and the anchored menu controls all branches.
#[test]
fn browser_defaults_and_tree_actions_hide_empty_categories_in_both_themes() {
    for theme in [crate::ThemeChoice::Dark, crate::ThemeChoice::Light] {
        let mut h = fixture(1800.);
        h.state_mut().preferences.theme = theme;
        h.run();
        assert!(h.state().views.active.library.as_ref().unwrap().opened.is_empty());
        assert!(h.query_all_by_label("Generic parts in Native Parts Demo").next().is_none());
        assert!(h.query_all_by_label("Project library Project-only").next().is_none());
        let before = h.state().session.snapshot().unwrap();
        let selected = h.state().views.active.library.as_ref().unwrap().selected.clone();
        click(&mut h, "Browser tree actions");
        click(&mut h, "Expand all");
        h.get_by_label("Generic parts in Native Parts Demo");
        h.get_by_label("Symbols in Basic components");
        assert!(h.query_all_by_label("Orderable parts in Basic components").next().is_none());
        assert!(h.query_all_by_label("Footprints in Power").next().is_none());
        click(&mut h, "Browser tree actions");
        click(&mut h, "Collapse all");
        assert!(h.query_all_by_label("Native Parts Demo").next().is_none());
        assert!(h.state().views.active.library.as_ref().unwrap().opened.is_empty());
        assert_eq!(h.state().views.active.library.as_ref().unwrap().selected, selected);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Keyboard focus draws an outline around the overflow button in both themes.
#[test]
fn browser_tree_actions_draw_focus_outline_and_open_from_keyboard() {
    for theme in [crate::ThemeChoice::Dark, crate::ThemeChoice::Light] {
        let mut h = fixture(1600.);
        h.state_mut().preferences.theme = theme;
        h.run();
        let label = "Browser tree actions";
        let has_outline = |h: &Harness<'_, PcbApp>| {
            let button = h.get_by_label(label).rect();
            h.output().shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(outline)
                    if outline.rect == button && outline.stroke.width > 0.0 && outline.stroke.color != egui::Color32::TRANSPARENT)
            })
        };
        assert!(!has_outline(&h));
        h.get_by_label("Collapse Browser").focus();
        h.run();
        for _ in 0..128 {
            if h.get_by_label(label).is_focused() {
                break;
            }
            h.key_press(egui::Key::Tab);
            h.run();
        }
        assert!(h.get_by_label(label).is_focused(), "Tab must reach the overflow button");
        assert!(has_outline(&h), "focused overflow must paint an outline in {theme:?}");

        h.key_press(egui::Key::Enter);
        h.run();
        h.get_by_label("Expand all");
        h.key_press(egui::Key::ArrowDown);
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(!h.state().views.active.library.as_ref().unwrap().opened.is_empty());
    }
}

/// The shared splitter grows the Browser instead of scrolling its contents sideways. Gesture
/// cancellation and keyboard adjustment retain the same project and preserve sibling layout.
#[test]
fn browser_edge_resizes_cancels_and_is_independent_between_panes() {
    let mut h = fixture(1800.);
    let snapshot = h.state().session.snapshot().unwrap();
    let edge = h.get_by_label("Resize Browser width").rect();
    h.drag_at(edge.center());
    h.run();
    h.hover_at(edge.center() + egui::vec2(180., 0.));
    h.run();
    h.drop_at(edge.center() + egui::vec2(180., 0.));
    h.run();
    let width = h.state().views.active.library.as_ref().unwrap().browser.width();
    assert!((width - 410.).abs() < 2., "width={width}");
    let edge = h.get_by_label("Resize Browser width").rect();
    assert!((edge.center().x - 410. - 22.).abs() < 3.);
    h.drag_at(edge.center());
    h.run();
    h.hover_at(edge.center() + egui::vec2(80., 0.));
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    h.drop_at(edge.center());
    h.run();
    assert_eq!(h.state().views.active.library.as_ref().unwrap().browser.width(), width);
    h.get_by_label("Resize Browser width").click();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert_eq!(h.state().views.active.library.as_ref().unwrap().browser.width(), width + 16.);
    let root = h.state().views.active.id();
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let other = app.views.open_beside(root).unwrap();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":other.0,"browser_width":300,"tree_action":"expand_all"})).unwrap();
    assert_eq!(app.views.view(root).unwrap().library.as_ref().unwrap().browser.width(), width + 16.);
    assert_eq!(app.views.view(other).unwrap().library.as_ref().unwrap().browser.width(), 300.);
    let before = app.views.view(other).unwrap().library.as_ref().unwrap().inspect();
    assert!(app.run_window_command(&ctx, "app.library_browse", &json!({"view":other.0,"browser_width":999,"tree_action":"collapse_all"})).is_err());
    assert_eq!(app.views.view(other).unwrap().library.as_ref().unwrap().inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &app.session.snapshot().unwrap()));
}

/// Docked Preview uses the same divider as Browser on either side; its preference is independent
/// of floating size, stays inside the host and cancels without changing project history.
#[test]
fn preview_column_divider_resizes_both_dock_sides_and_cancels() {
    use crate::workspace_panels::PROPERTIES;
    use pcbcraft_chrome::workspace::panels::Position;
    let mut h = fixture(1800.);
    let snapshot = h.state().session.snapshot().unwrap();
    for (position, direction) in [(Position::Right, -1.), (Position::Left, 1.)] {
        h.state_mut().views.active.panels[PROPERTIES].set_position(position, None);
        h.run();
        let before = h.state().views.active.library.as_ref().unwrap().preview_width.width(440.);
        let edge = h.get_by_label("Resize Preview column width").rect();
        h.drag_at(edge.center());
        h.run();
        h.hover_at(edge.center() + egui::vec2(direction * 70., 0.));
        h.run();
        h.drop_at(edge.center() + egui::vec2(direction * 70., 0.));
        h.run();
        let width = h.state().views.active.library.as_ref().unwrap().preview_width.width(440.);
        assert!((width - before - 70.).abs() < 2., "side={position:?}, width={width}, before={before}");
        let edge = h.get_by_label("Resize Preview column width").rect();
        h.drag_at(edge.center());
        h.run();
        h.hover_at(edge.center() - egui::vec2(direction * 40., 0.));
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
        h.drop_at(edge.center());
        h.run();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().preview_width.width(440.), width);
        h.get_by_label("Resize Preview column width").click();
        h.run();
        h.key_press(if direction > 0. { egui::Key::ArrowLeft } else { egui::Key::ArrowRight });
        h.run();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().preview_width.width(440.), width - 16.);
    }
    h.state_mut().views.active.panels[PROPERTIES].set_position(Position::Floating, None);
    h.run();
    assert!(h.query_all_by_label("Resize Preview column width").next().is_none());
    assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
}

/// Live layout commands validate before publication and share the same pane-local preview width.
#[test]
fn preview_column_width_commands_are_atomic_and_pane_local() {
    let mut h = fixture(1800.);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let root = app.views.active.id();
    let other = app.views.open_beside(root).unwrap();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":other.0,"preview_width":500})).unwrap();
    assert_eq!(app.views.view(other).unwrap().library.as_ref().unwrap().preview_width.preferred(), Some(500.));
    assert_eq!(app.views.view(root).unwrap().library.as_ref().unwrap().preview_width.preferred(), None);
    let before = app.views.view(other).unwrap().library.as_ref().unwrap().inspect();
    for params in [json!({"view":other.0,"preview_width":100}), json!({"view":other.0,"preview_width":300,"preview":"symbol","pan_x":20})] {
        assert!(app.run_window_command(&ctx, "app.library_browse", &params).is_err());
        assert_eq!(app.views.view(other).unwrap().library.as_ref().unwrap().inspect(), before);
    }
}

/// Preview uses the remaining host width instead of a 640-point ceiling, while Browser and
/// Items retain useful space. Both docked edges share real pointer and keyboard behavior.
#[test]
fn preview_can_grow_beyond_640_without_changing_browser_or_project() {
    use crate::workspace_panels::PROPERTIES;
    use pcbcraft_chrome::workspace::panels::Position;
    for side in [Position::Left, Position::Right] {
        let mut h = fixture(3200.);
        h.state_mut().views.active.panels[PROPERTIES].set_position(side, None);
        h.run();
        let snapshot = h.state().session.snapshot().unwrap();
        let browser = h.state().views.active.library.as_ref().unwrap().browser.width();
        let edge = h.get_by_label("Resize Preview column width").rect();
        let delta = if side == Position::Left { 1000. } else { -1000. };
        h.drag_at(edge.center());
        h.run();
        h.hover_at(edge.center() + egui::vec2(delta, 0.));
        h.run();
        h.drop_at(edge.center() + egui::vec2(delta, 0.));
        h.run();
        let width = h.state().views.active.library.as_ref().unwrap().preview_width.width(440.);
        assert!(width > 1300., "width={width}");
        assert_eq!(h.state().views.active.library.as_ref().unwrap().browser.width(), browser);
        let ctx = h.ctx.clone();
        let view = h.state().views.active.id();
        h.state_mut().run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"preview_width":2000})).unwrap();
        h.run();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().preview_width.preferred(), Some(2000.));
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    }
}
