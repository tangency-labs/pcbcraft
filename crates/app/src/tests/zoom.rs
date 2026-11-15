//! Sample real canvas navigation frame by frame instead of waiting for motion to settle.
use super::*;

/// A new empty PCB has a useful physical drawing scale; empty Fit restores it, while pane
/// switching, saved-view restoration and ordinary edits preserve deliberate navigation.
#[test]
fn empty_boards_start_close_and_keep_remembered_navigation() {
    let mut h = harness();
    h.state_mut().start_new_project_in(Workspace::Board);
    h.run();
    assert_eq!(h.state().views.active.camera.zoom(), 4.0);
    let initial = h.state().session.snapshot().unwrap();
    h.state_mut().views.active.camera.set_zoom(12.0);
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 4_000_000, y_nm: -2_000_000 });
    let remembered = h.state().views.active.camera.saved_state();
    let app = h.state_mut();
    app.views.active.switch(Workspace::Schematic, &app.session);
    assert_eq!(h.state().views.active.camera.zoom(), 1.0);
    let app = h.state_mut();
    app.views.active.switch(Workspace::Board, &app.session);
    h.run();
    assert_eq!(h.state().views.active.camera.saved_state(), remembered);
    let saved = h.state().views.active.saved_state(&h.state().session).unwrap();
    let restored = crate::editor_view::EditorView::from_state(ViewId(99), &saved, &h.state().session);
    assert_eq!(restored.camera.saved_state(), remembered);
    assert!(std::sync::Arc::ptr_eq(&initial, &h.state().session.snapshot().unwrap()));
    h.state_mut().run("project.rename", &json!({"name":"Still empty"}));
    h.run();
    assert_eq!(h.state().views.active.camera.saved_state(), remembered);
    h.state_mut().views.active.camera.request_fit();
    h.run();
    assert_eq!(h.state().views.active.camera.zoom(), 4.0);
    assert_eq!(h.state().views.active.camera.center(), pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 });
    h.state_mut().start_new_project_in(Workspace::Board);
    h.run();
    assert_eq!(h.state().views.active.camera.zoom(), 4.0);
}

/// Wheel zoom traverses intermediate scales and keeps its anchor fixed in either workspace,
/// while a pinch follows the fingers in its input frame. Navigation never edits the project.
#[test]
fn wheel_zoom_is_continuous_and_pinch_is_immediate() {
    for workspace in [Workspace::Schematic, Workspace::Board] {
        for around in [ZoomAround::Pointer, ZoomAround::Center] {
            let mut app = PcbApp::default();
            app.start_new_project_in(workspace);
            app.preferences.navigation.zoom_around = around;
            let mut h = Harness::builder()
                .with_step_dt(1. / 60.)
                .with_max_steps(20)
                .with_size(egui::vec2(1400., 900.))
                .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let name = if workspace == Workspace::Board { "PCB canvas" } else { "Schematic canvas" };
            let rect = h.get_by_label(name).rect();
            let pointer = rect.center() + egui::vec2(31., -17.);
            h.hover_at(pointer);
            h.run();
            let anchor = if around == ZoomAround::Pointer { pointer } else { rect.center() };
            let model = h.state().views.active.camera.unsnapped(anchor, rect);
            let initial = h.state().views.active.camera.zoom();
            let target = initial * 0.32_f32.exp();
            h.event(wheel(2.));
            let mut previous = initial;
            for _ in 0..3 {
                h.step();
                let camera = &h.state().views.active.camera;
                assert!(
                    camera.zoom() > previous && camera.zoom() < target,
                    "{workspace:?} {around:?}: {previous} -> {} (target {target})",
                    camera.zoom()
                );
                assert!(camera.screen(model, rect).distance(anchor) < 0.002);
                previous = camera.zoom();
            }
            h.run();
            assert!((h.state().views.active.camera.zoom() - target).abs() < 1e-6);
            h.event(egui::Event::Zoom(1.2));
            h.step();
            assert!((h.state().views.active.camera.zoom() - target * 1.2).abs() < 1e-6);
            h.run();
            assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Leaving the canvas stops its wheel tail, and returning cannot replay scroll input from
/// a panel. Changed navigation settings, an editing click or focus loss stop it too.
#[test]
fn pending_zoom_does_not_cross_input_owners() {
    let mut h = harness();
    new_project(&mut h);
    let pointer = canvas_center(&h);
    h.hover_at(pointer);
    h.run();
    h.event(wheel(2.));
    h.step();
    let stopped = h.state().views.active.camera.zoom();
    let browser = h.get_by_label("Main").rect().center();
    h.hover_at(browser);
    h.run();
    h.event(wheel(3.));
    h.run();
    h.hover_at(pointer);
    h.run();
    assert_eq!(h.state().views.active.camera.zoom(), stopped);
    h.event(wheel(2.));
    h.step();
    let stopped = h.state().views.active.camera.zoom();
    h.state_mut().preferences.navigation.scroll = ScrollAction::Pan;
    h.run();
    assert_eq!(h.state().views.active.camera.zoom(), stopped);
    h.state_mut().preferences.navigation = Navigation::default();
    for focus_loss in [false, true] {
        h.event(wheel(2.));
        h.step();
        let stopped = h.state().views.active.camera.zoom();
        if focus_loss {
            h.input_mut().focused = false;
            h.event(egui::Event::WindowFocused(false));
        } else {
            h.event(egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        }
        h.run();
        assert_eq!(h.state().views.active.camera.zoom(), stopped);
        if focus_loss {
            h.input_mut().focused = true;
            h.event(egui::Event::WindowFocused(true));
        } else {
            h.event(egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        }
        h.run();
        assert_eq!(h.state().views.active.camera.zoom(), stopped);
    }
}

/// Use the same line-based notch events delivered by an ordinary mouse wheel.
fn wheel(lines: f32) -> egui::Event {
    egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0., lines),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }
}

/// With middle-button drag set to zoom, dragging up magnifies about the press point and
/// dragging down shrinks again, in either workspace, without moving that point or editing.
#[test]
fn middle_drag_zooms_around_the_press_point_when_enabled() {
    for workspace in [Workspace::Schematic, Workspace::Board] {
        let mut app = PcbApp::default();
        app.start_new_project_in(workspace);
        app.preferences.navigation.middle_drag_zoom = true;
        let mut h = Harness::builder()
            .with_step_dt(1. / 60.)
            .with_max_steps(20)
            .with_size(egui::vec2(1400., 900.))
            .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let name = if workspace == Workspace::Board { "PCB canvas" } else { "Schematic canvas" };
        let rect = h.get_by_label(name).rect();
        let start = rect.center() + egui::vec2(40., 0.);
        h.hover_at(start);
        h.run();
        let model = h.state().views.active.camera.unsnapped(start, rect);
        let initial = h.state().views.active.camera.zoom();
        let middle = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
        h.event(middle(start, true));
        h.run();
        let up = start - egui::vec2(0., 48.);
        h.hover_at(up);
        h.run();
        let zoomed = h.state().views.active.camera.zoom();
        assert!(zoomed > initial, "{workspace:?}: {initial} -> {zoomed}");
        assert!(h.state().views.active.camera.screen(model, rect).distance(start) < 0.01, "{workspace:?}");
        h.event(middle(up, false));
        h.run();
        h.event(middle(start, true));
        h.run();
        let down = start + egui::vec2(0., 48.);
        h.hover_at(down);
        h.run();
        assert!(h.state().views.active.camera.zoom() < zoomed, "{workspace:?}");
        h.event(middle(down, false));
        h.run();
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// With right-button drag set to pan, dragging moves the view in either workspace without
/// starting a selection box or editing. A placed symbol stays unselected.
#[test]
fn right_drag_pans_without_selecting_when_enabled() {
    for workspace in [Workspace::Schematic, Workspace::Board] {
        let mut app = PcbApp::default();
        app.start_new_project_in(workspace);
        app.preferences.navigation.right_drag_pan = true;
        let mut h = Harness::builder()
            .with_step_dt(1. / 60.)
            .with_max_steps(20)
            .with_size(egui::vec2(1400., 900.))
            .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        if workspace == Workspace::Schematic {
            let revision = h.state().session.revision();
            h.state_mut().session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision})).unwrap();
            h.run();
        }
        let before = h.state().session.snapshot().unwrap();
        let name = if workspace == Workspace::Board { "PCB canvas" } else { "Schematic canvas" };
        let rect = h.get_by_label(name).rect();
        let start = rect.center() + egui::vec2(-80., -60.);
        let end = start + egui::vec2(140., 90.);
        h.hover_at(start);
        h.run();
        let model = h.state().views.active.camera.unsnapped(start, rect);
        let secondary =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
        h.event(secondary(start, true));
        h.run();
        h.hover_at(end);
        h.run();
        h.event(secondary(end, false));
        h.run();
        assert!(h.state().views.active.camera.screen(model, rect).distance(end) < 0.01, "{workspace:?}");
        if workspace == Workspace::Schematic {
            assert!(h.state().views.active.schematic.selection.is_empty());
        }
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// The Shortcuts page lists the wheel under Pan when it pans, and Ctrl+wheel under Zoom, so
/// the Pan row survives middle-drag zoom without right-drag pan.
#[test]
fn shortcut_list_follows_wheel_scroll_mode() {
    let ctx = egui::Context::default();
    ctx.set_os(egui::os::OperatingSystem::Nix);
    let view_row = |navigation: Navigation, action: &str| {
        crate::shortcut_list(&ctx, navigation).into_iter().find(|(section, row, _)| *section == "View" && *row == action).map(|(_, _, keys)| keys)
    };
    let panning = Navigation { scroll: ScrollAction::Pan, middle_drag_zoom: true, ..Navigation::default() };
    assert_eq!(view_row(panning, "Pan").as_deref(), Some("mouse wheel"));
    assert_eq!(view_row(panning, "Zoom").as_deref(), Some("Ctrl+mouse wheel or middle-button drag"));
    let zooming = Navigation { middle_drag_zoom: true, ..Navigation::default() };
    assert_eq!(view_row(zooming, "Pan"), None);
    assert_eq!(view_row(zooming, "Zoom").as_deref(), Some("Mouse wheel or middle-button drag"));
}
