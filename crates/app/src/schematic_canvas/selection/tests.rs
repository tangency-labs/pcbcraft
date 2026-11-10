use crate::{PcbApp, schematic_canvas::Tool, tests::click};
use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    DRAW_WIRE, MOVE_SYMBOL, PLACE_SYMBOL,
    schematic::{GRID_NM, Point},
};
use serde_json::json;

/// Keep the fixture clear of the floating Browser; use real engine commands for its contents.
fn harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({"name":"Selection"})).unwrap();
    for x in [-15, 15] {
        app.session
            .apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x*GRID_NM,"y_nm":0,"expected_revision":app.session.revision()}))
            .unwrap();
    }
    app.session.apply(DRAW_WIRE,&json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"bends":[],"expected_revision":app.session.revision()})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(20).build_ui_state(|ui, app| app.frame(ui), app);
    h.run();
    h
}

/// Position a test gesture in model coordinates using the actual canvas transform.
fn point(h: &Harness<'_, PcbApp>, x: i64, y: i64) -> Pos2 {
    h.state().views.active.camera.screen(Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }, h.get_by_label("Schematic canvas").rect())
}

/// Hold either mouse button and carry its modifiers into every frame of the drag.
fn start(h: &mut Harness<'_, PcbApp>, from: Pos2, to: Pos2, button: PointerButton, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers });
    h.run();
    h.event(egui::Event::PointerMoved(from + (to - from) * 0.5));
    h.run();
    h.event(egui::Event::PointerMoved(to));
    h.run();
}

/// Release the captured mouse button without synthesizing an unrelated click.
fn release(h: &mut Harness<'_, PcbApp>, to: Pos2, button: PointerButton, modifiers: Modifiers) {
    h.event(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers });
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Both buttons render a real Window/Crossing overlay, select symbols and wires, and leave
/// project data/history untouched. Right drags never open the stationary-click context menu.
#[test]
fn both_buttons_draw_boxes_and_respect_direction() {
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        let mut h = harness();
        let before = h.state().session.inspect();
        let from = point(&h, -21, -7);
        let to = point(&h, 21, 7);
        start(&mut h, from, to, button, Modifiers::NONE);
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::BoxSelect { .. })));
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()=="Window · 3 items")));
        release(&mut h, to, button, Modifiers::NONE);
        assert_eq!(h.state().views.active.schematic.selection.len(), 3);
        assert_eq!(h.state().session.inspect(), before);
        assert!(!egui::Popup::is_any_open(&h.ctx));
        // A partial symbol body excludes the wire and reverses from containment to crossing.
        let a = point(&h, -16, -5);
        let b = point(&h, -14, 5);
        start(&mut h, a, b, button, Modifiers::NONE);
        release(&mut h, b, button, Modifiers::NONE);
        assert!(h.state().views.active.schematic.selection.is_empty());
        start(&mut h, b, a, button, Modifiers::NONE);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()=="Crossing · 1 items")));
        release(&mut h, a, button, Modifiers::NONE);
        assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some("symbol-2"));
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Expand Symbols");
        let row = h.get_by_label("R1  R");
        assert_eq!(row.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    }
}

/// Shift/Ctrl apply to the starting selection, and cancelling or invalidating the drag restores
/// it. Releasing over a floating card cannot commit a canvas selection.
#[test]
fn modifiers_escape_stale_edits_and_panel_release_are_safe() {
    let mut h = harness();
    for (x, modifiers, expected) in [(-15, Modifiers::NONE, 1), (15, Modifiers::SHIFT, 2), (-15, Modifiers::CTRL, 1)] {
        let from = point(&h, x - 5, -5);
        let to = point(&h, x + 5, 5);
        start(&mut h, from, to, PointerButton::Secondary, modifiers);
        release(&mut h, to, PointerButton::Secondary, modifiers);
        assert_eq!(h.state().views.active.schematic.selection.symbols.len(), expected);
    }
    let selected = h.state().views.active.schematic.selection.clone();
    let from = point(&h, -22, -8);
    let to = point(&h, 22, 8);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    h.key_press(Key::Escape);
    h.run();
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().views.active.schematic.selection, selected);
    start(&mut h, from, to, PointerButton::Secondary, Modifiers::NONE);
    let revision = h.state().session.revision();
    h.state_mut().session.apply(MOVE_SYMBOL, &json!({"symbol":"symbol-2","x_nm":-15*GRID_NM,"y_nm":GRID_NM,"expected_revision":revision})).unwrap();
    h.run();
    release(&mut h, to, PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(h.state().views.active.schematic.selection, selected);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    let panel = h.state().browser.rect().center();
    h.hover_at(panel);
    h.run();
    release(&mut h, panel, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().views.active.schematic.selection, selected);
}

/// A selected member drags the whole group through one command, including the connecting route;
/// Delete also commits once and undo restores the exact project. Cancelled moves change nothing.
#[test]
fn group_drag_delete_and_undo_use_atomic_commands() {
    let mut h = harness();
    let from = point(&h, -22, -8);
    let to = point(&h, 22, 8);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let from = point(&h, -15, 0);
    let to = point(&h, -10, 4);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::MoveItems { .. })));
    assert_eq!(h.state().session.revision(), revision);
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().session.revision(), revision + 1);
    for (old, new) in before.symbols().iter().zip(h.state().session.project().unwrap().symbols()) {
        assert_eq!(new.position, Point { x_nm: old.position.x_nm + 5 * GRID_NM, y_nm: old.position.y_nm + 4 * GRID_NM });
    }
    click(&mut h, "Undo");
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    h.key_press(Key::Escape);
    h.run();
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let revision = h.state().session.revision();
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert!(h.state().session.project().unwrap().symbols().is_empty());
    assert!(h.state().session.project().unwrap().wires().is_empty());
    click(&mut h, "Undo");
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Crossing tests actual segments: an empty corner of an L-shaped wire's bounding box misses.
#[test]
fn crossing_ignores_empty_wire_bounds_and_middle_drag_only_pans() {
    let mut h = harness();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE,&json!({"from":{"x_nm":0,"y_nm":10*GRID_NM},"to":{"x_nm":10*GRID_NM,"y_nm":20*GRID_NM},"bends":[{"x_nm":10*GRID_NM,"y_nm":10*GRID_NM}],"expected_revision":revision})).unwrap();
    h.run();
    let from = point(&h, 5, 17);
    let to = point(&h, 2, 14);
    start(&mut h, from, to, PointerButton::Secondary, Modifiers::NONE);
    release(&mut h, to, PointerButton::Secondary, Modifiers::NONE);
    assert!(h.state().views.active.schematic.selection.is_empty());
    let from = point(&h, 5, 12);
    let to = point(&h, 2, 8);
    start(&mut h, from, to, PointerButton::Secondary, Modifiers::NONE);
    release(&mut h, to, PointerButton::Secondary, Modifiers::NONE);
    assert!(h.state().views.active.schematic.selection.single_wire().is_some());
    let selected = h.state().views.active.schematic.selection.clone();
    let before = h.state().session.inspect();
    let original = point(&h, 0, 0);
    start(&mut h, from, to, PointerButton::Middle, Modifiers::NONE);
    release(&mut h, to, PointerButton::Middle, Modifiers::NONE);
    assert_ne!(point(&h, 0, 0), original);
    assert_eq!(h.state().views.active.schematic.selection, selected);
    assert_eq!(h.state().session.inspect(), before);
}

/// A fast drag can move and release between frames; it must finish rather than leave a stuck tool.
#[test]
fn coalesced_move_and_release_finishes_selection() {
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        let mut h = harness();
        let from = point(&h, -22, -8);
        let to = point(&h, 22, 8);
        h.hover_at(from);
        h.event(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers: Modifiers::NONE });
        h.run();
        h.hover_at(to);
        h.event(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers: Modifiers::NONE });
        h.run();
        assert!(h.state().views.active.schematic.tool.is_none());
        assert_eq!(h.state().views.active.schematic.selection.len(), 3);
    }
}

/// Losing window focus cancels a held box or group move even without a mouse-up event.
#[test]
fn focus_loss_cancels_uncommitted_gestures() {
    let mut h = harness();
    let from = point(&h, -22, -8);
    let to = point(&h, 22, 8);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    h.event(egui::Event::WindowFocused(false));
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(h.state().views.active.schematic.selection.is_empty());
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    h.event(egui::Event::WindowFocused(true));
    h.run();
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    let before = h.state().session.inspect();
    let selected = h.state().views.active.schematic.selection.clone();
    let from = point(&h, -15, 0);
    let to = point(&h, -10, 4);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    h.event(egui::Event::WindowFocused(false));
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(h.state().views.active.schematic.selection, selected);
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().session.inspect(), before);
}

/// Ground/power symbols and text labels use their painted bounds for box selection. Group
/// dragging and Delete must keep every selected anchor together and share one undo step.
#[test]
fn named_anchors_box_select_move_delete_and_filter() {
    let mut h = harness();
    for (style, x) in [("gnd", -15), ("vcc", -5), ("local", 5), ("global", 15)] {
        let revision = h.state().session.revision();
        if matches!(style, "gnd" | "vcc") {
            let d = h.state().session.libraries().latest_symbol("power", style).unwrap();
            h.state_mut().session.apply(PLACE_SYMBOL,&json!({"library":"power","definition":style,"definition_revision":d.revision(),"definition_digest":pcbcraft_engine::library::sources::symbol_digest(&d).unwrap(),"x_nm":x*GRID_NM,"y_nm":15*GRID_NM,"expected_revision":revision})).unwrap();
        } else {
            h.state_mut()
                .session
                .apply(
                    pcbcraft_engine::net_labels::ADD,
                    &json!({"name":"GND","style":style,"x_nm":x*GRID_NM,"y_nm":15*GRID_NM,"expected_revision":revision}),
                )
                .unwrap();
        }
    }
    h.run();
    let from = point(&h, -22, 10);
    let to = point(&h, 22, 24);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text()=="Window · 4 items")));
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().views.active.schematic.selection.len(), 4);
    let glyph = point(&h, -15, 17);
    for (modifiers, count) in [(Modifiers::CTRL, 3), (Modifiers::COMMAND, 4)] {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(glyph));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: glyph, button: PointerButton::Primary, pressed, modifiers });
            h.run();
        }
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run();
        assert_eq!(h.state().views.active.schematic.selection.len(), count);
    }
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    // Grab the ground glyph rather than its wire-start anchor.
    let from = point(&h, -15, 17);
    let to = point(&h, -10, 21);
    start(&mut h, from, to, PointerButton::Primary, Modifiers::NONE);
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::MoveItems { .. })));
    release(&mut h, to, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().session.revision(), revision + 1);
    for (old, new) in before
        .symbols()
        .iter()
        .filter(|s| s.definition.library == "power")
        .zip(h.state().session.project().unwrap().symbols().iter().filter(|s| s.definition.library == "power"))
    {
        assert_eq!(new.position, Point { x_nm: old.position.x_nm + 5 * GRID_NM, y_nm: old.position.y_nm + 4 * GRID_NM });
    }
    for (old, new) in before.net_labels().iter().zip(h.state().session.project().unwrap().net_labels()) {
        assert_eq!(new.at, Point { x_nm: old.at.x_nm + 5 * GRID_NM, y_nm: old.at.y_nm + 4 * GRID_NM });
    }
    click(&mut h, "Undo");
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    // Delete while hovering a selected ground must delete the entire group.
    let glyph = point(&h, -15, 17);
    h.hover_at(glyph);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    click(&mut h, "Undo");
    let view = h.state().views.active.id();
    h.state_mut().set_selection_filters(view, &json!({"symbols":false})).unwrap();
    let from = point(&h, -22, 10);
    let to = point(&h, 22, 24);
    start(&mut h, from, to, PointerButton::Secondary, Modifiers::NONE);
    release(&mut h, to, PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(h.state().views.active.schematic.selection.net_labels.len(), 2);
    assert!(h.state().views.active.schematic.selection.net_labels.iter().all(|id| before.net_labels().iter().any(|label| label.object_id() == *id
        && matches!(label.style, pcbcraft_engine::net_labels::Style::Local | pcbcraft_engine::net_labels::Style::Global))));
}
