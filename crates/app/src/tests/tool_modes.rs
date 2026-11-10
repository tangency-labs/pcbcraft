//! Fast pointer input and persistent drawing modes exercise the real canvas and catalog.
use super::*;

/// Start with the shipped defaults rather than the one-shot fixtures used by unrelated tests.
fn editor() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    app.browser.open = false;
    app.properties_open = false;
    app.views.active.filters_open = false;
    let mut h = Harness::builder().with_size(egui::vec2(1400., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Deliver one frame within the double-click interval, without idle frames hiding timing bugs.
fn pointer(h: &mut Harness<'_, PcbApp>, at: egui::Pos2, pressed: bool) {
    h.input_mut().time = Some(h.ctx.input(|i| i.time) + 0.015);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// Fast placements must not turn small press/release motion into a drag or lose a repeat.
#[test]
fn rapid_moving_clicks_place_every_resistor_once() {
    let mut h = editor();
    h.state_mut().parts.set_repeat(true);
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Resistor");
    click(&mut h, "Place");
    let canvas = h.get_by_label("Schematic canvas").rect();
    for i in 0..6 {
        let at = canvas.center() + egui::vec2(-250. + i as f32 * 85., 0.);
        pointer(&mut h, at, true);
        pointer(&mut h, at + egui::vec2(15., 0.), false);
        assert_eq!(h.state().session.project().unwrap().symbols().len(), i + 1, "missed placement {i}");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::PlaceSymbol { .. })));
        assert!(matches!(h.state().dialog, Dialog::None));
    }
    h.input_mut().time = None;
    h.run();
}

/// One-shot placement also owns its release: held motion cannot pick up the new symbol and
/// publish a second move after the initial placement press.
#[test]
fn single_placement_does_not_drag_on_its_own_release() {
    let mut h = editor();
    h.state_mut().parts.set_repeat(false);
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Resistor");
    click(&mut h, "Place");
    let point = h.get_by_label("Schematic canvas").rect().center();
    let revision = h.state().session.revision();
    pointer(&mut h, point, true);
    let placed = h.state().session.snapshot().unwrap();
    h.event(egui::Event::PointerMoved(point + egui::vec2(40., 0.)));
    h.run_steps(1);
    pointer(&mut h, point + egui::vec2(40., 0.), false);
    h.input_mut().time = None;
    h.run();
    assert!(Arc::ptr_eq(&placed, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.state().views.active.schematic.tool.is_none());
}

/// Press and release can arrive in one render frame, including a double-click sequence.
/// Every press is one placement and does not open properties for the previous instance.
#[test]
fn same_frame_clicks_repeat_without_a_double_click_delay() {
    let mut h = editor();
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Resistor");
    click(&mut h, "Place");
    let at = h.get_by_label("Schematic canvas").rect().center();
    for i in 0..3 {
        h.input_mut().time = Some(h.ctx.input(|input| input.time) + 0.015);
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        h.run_steps(1);
        assert_eq!(h.state().session.project().unwrap().symbols().len(), i + 1);
        assert!(matches!(h.state().dialog, Dialog::None));
    }
    h.input_mut().time = None;
    h.run();
}

/// Right-click belongs to the canvas even when empty; closing or using the popup cannot place
/// a component behind it. Placement remains armed until its explicit cancellation action.
#[test]
fn idle_and_placement_context_menus_keep_input_and_repeat_choices() {
    use egui::accesskit::{Role, Toggled};

    let mut h = editor();
    assert!(h.state().parts.repeats());
    let point = h.get_by_label("Schematic canvas").rect().center();
    right_click(&mut h, point);
    assert!(h.get_by_label("Draw wires").rect().width() < 300., "menu width follows its content");
    for label in ["Draw wires", "Place symbol…", "Select all", "Unselect all", "Fit schematic", "Grid"] {
        assert!(h.query_all_by_label(label).next().is_some());
    }
    assert!(h.query_by_label("Finer grid").is_none());
    click(&mut h, "Grid");
    click(&mut h, "25 mil");
    assert_eq!(h.state().views.active.camera.snap_nm(), 25 * pcbcraft_chrome::grid::NM_PER_MIL);
    right_click(&mut h, point);
    click(&mut h, "Place symbol…");
    click(&mut h, "Resistor");
    click(&mut h, "Place");
    click_canvas_at(&mut h, point);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::PlaceSymbol { .. })));
    let before = h.state().session.snapshot().unwrap();
    right_click(&mut h, point + egui::vec2(150., 80.));
    let repeat = h.get_by_role_and_label(Role::CheckBox, "Repeat placement");
    assert_eq!(repeat.accesskit_node().toggled(), Some(Toggled::True));
    click(&mut h, "Rotate");
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::PlaceSymbol { turns: 1, .. })));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    right_click(&mut h, point + egui::vec2(150., 80.));
    h.get_by_role_and_label(Role::CheckBox, "Repeat placement").click();
    h.run_steps(1);
    assert!(!h.state().parts.repeats());
    assert!(h.output().platform_output.events.iter().any(|event| {
        matches!(event, egui::output::OutputEvent::Clicked(info)
            if info.label.as_deref() == Some("Repeat placement") && info.selected == Some(false))
    }));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    right_click(&mut h, point + egui::vec2(150., 80.));
    let repeat = h.get_by_role_and_label(Role::CheckBox, "Repeat placement");
    assert_eq!(repeat.accesskit_node().toggled(), Some(Toggled::False));
    click_canvas_at(&mut h, point + egui::vec2(-150., 80.));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click_canvas_at(&mut h, point + egui::vec2(150., 80.));
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    assert!(h.state().views.active.schematic.tool.is_none());
    right_click(&mut h, point + egui::vec2(-200., 180.));
    click(&mut h, "Select all");
    assert_eq!(h.state().views.active.schematic.selection.len(), 2);
    right_click(&mut h, point + egui::vec2(-200., 180.));
    assert_eq!(h.state().views.active.schematic.selection.len(), 2);
    click(&mut h, "Unselect all");
    assert!(h.state().views.active.schematic.selection.is_empty());
}

/// Finishing one wire clears its route draft but retains the drawing mode. The next route
/// starts independently, and each completion is exactly one shared undo step.
#[test]
fn wire_mode_stays_armed_for_independent_routes_until_cancelled() {
    let mut h = editor();
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.key_press(Key::W);
    h.run();
    for i in 0..2 {
        let from = canvas.center() + egui::vec2(-200., -100. + i as f32 * 160.);
        let to = from + egui::vec2(250., 0.);
        click_canvas_at(&mut h, from);
        h.hover_at(to);
        h.run();
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(h.state().session.project().unwrap().wires().len(), i + 1);
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    }
    let before = h.state().session.snapshot().unwrap();
    right_click(&mut h, canvas.center());
    click(&mut h, "Switch bend direction");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    right_click(&mut h, canvas.center());
    click(&mut h, "Cancel");
    assert!(h.state().views.active.schematic.tool.is_none());
    click(&mut h, "Undo");
    assert_eq!(h.state().session.project().unwrap().wires().len(), 1);
    click(&mut h, "Redo");
    assert_eq!(h.state().session.project().unwrap().wires().len(), 2);
}

/// A context menu opened in the second split starts and cancels only that pane's tool.
#[test]
fn tool_context_actions_belong_to_their_split_pane() {
    let mut h = editor();
    h.set_size(egui::vec2(1700., 900.));
    h.run();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let at = h.get_by_label("Schematic canvas 2").rect().center();
    let before = h.state().session.snapshot().unwrap();
    right_click(&mut h, at);
    click(&mut h, "Grid");
    click(&mut h, "10 mil");
    assert_eq!(h.state().views.active.camera.snap_nm(), 10 * pcbcraft_chrome::grid::NM_PER_MIL);
    assert_eq!(h.state().views.view(ViewId(0)).unwrap().camera.snap_nm(), 50 * pcbcraft_chrome::grid::NM_PER_MIL);
    right_click(&mut h, at);
    click(&mut h, "Draw wires");
    assert_eq!(h.state().views.active.id().0, 1);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    assert!(h.state().views.view(ViewId(0)).unwrap().schematic.tool.is_none());
    right_click(&mut h, at);
    click(&mut h, "Cancel");
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(h.state().views.view(ViewId(0)).unwrap().schematic.tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Open the pointer menu with complete secondary events without starting a selection rectangle.
fn right_click(h: &mut Harness<'_, PcbApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}
