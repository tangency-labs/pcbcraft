//! Sample real canvas navigation frame by frame instead of waiting for motion to settle.
use super::*;

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
