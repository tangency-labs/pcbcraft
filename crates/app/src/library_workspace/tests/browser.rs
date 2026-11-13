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
