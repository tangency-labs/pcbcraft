use pcbcraft_engine::{persistence::encode, schematic::Point};

use super::*;

/// Real canvas navigation and selection affect one view only, leaving snapshot bytes, dirty
/// status and undo availability unchanged while another view is bound to the same session.
#[test]
fn navigation_and_selection_belong_to_the_view() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    let mut other = EditorView::new(ViewId(1));
    other.sync(&harness.state().session);
    let snapshot = harness.state().session.snapshot().unwrap();
    let bytes = encode(snapshot.clone()).unwrap();
    let inspected = harness.state().session.inspect();
    let dirty = harness.state().files.dirty(&harness.state().session);
    click_canvas(&mut harness);
    harness.hover_at(canvas_center(&harness));
    harness.run();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 2.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    harness.run();
    assert_eq!(harness.state().views.active.schematic.selection.single_symbol(), Some("symbol-2"));
    assert!(harness.state().views.active.camera.zoom() > 1.0);
    assert!(other.schematic.selection.single_symbol().is_none());
    assert_eq!(other.camera.zoom(), 1.0);
    assert_eq!(harness.state().session.inspect(), inspected);
    assert!(Arc::ptr_eq(&snapshot, &harness.state().session.snapshot().unwrap()));
    assert_eq!(encode(harness.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(harness.state().files.dirty(&harness.state().session), dirty);
}

/// Delayed canvas events and browser payloads must validate their origin before changing either
/// the current selection or a newer tool preview, even when the object IDs still exist.
#[test]
fn stale_and_foreign_view_input_cannot_replace_current_interactions() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    let app = harness.state_mut();
    let old = app.views.active.input(&app.session).unwrap();
    let definition = app.session.project().unwrap().symbols()[0].definition.clone();
    app.run("project.rename", &json!({"name": "current"}));
    let mut other = EditorView::new(ViewId(1));
    other.sync(&app.session);
    let foreign = other.input(&app.session).unwrap();
    let before = app.session.inspect();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let origin = Point { x_nm: 0, y_nm: 0 };
    let before_camera = app.views.active.camera.screen(origin, rect);
    for source in [old, foreign] {
        app.view_click(source, Click::Select(Some("symbol-2".to_owned())));
        app.view_click(source, Click::Edit(SymbolAction::Delete, "symbol-2".to_owned()));
        app.show_pick(source, project_browser::Pick { id: "symbol-2".to_owned(), wire: false, at: Point { x_nm: 25_400_000, y_nm: 0 } });
        assert!(app.views.active.schematic.selection.single_symbol().is_none());
        assert_eq!(app.views.active.camera.screen(origin, rect), before_camera);
        app.begin_wire();
        let current = app.views.active.input(&app.session).unwrap();
        app.view_click(current, Click::Drop(PartDrag { definition: definition.clone(), source }, Point { x_nm: 0, y_nm: 0 }));
        app.view_click(source, Click::CancelWire);
        assert!(matches!(app.views.active.schematic.tool, Some(Tool::Wire { .. })));
        app.views.active.cancel_gesture();
    }
    assert_eq!(app.session.inspect(), before);
}

/// A file reopening between mouse-down and mouse-up cancels the old move even when it restores
/// the identical snapshot. The new canvas also has a fresh egui interaction scope.
#[test]
fn reopening_during_a_move_does_not_move_the_reopened_symbol() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    let snapshot = harness.state().session.snapshot().unwrap();
    let from = canvas_center(&harness);
    let to = from + egui::vec2(90.0, 40.0);
    harness.drag_at(from);
    harness.run();
    harness.hover_at(to);
    harness.run();
    assert!(matches!(harness.state().views.active.schematic.tool, Some(Tool::Move { .. })));
    harness.state_mut().session.open_snapshot(snapshot);
    let reopened = harness.state().session.inspect();
    harness.run();
    harness.drop_at(to);
    harness.run();
    assert_eq!(harness.state().session.inspect(), reopened);
    assert!(harness.state().views.active.schematic.tool.is_none());
    assert!(harness.state().views.active.schematic.selection.single_symbol().is_none());
}

/// The fields dialog retains its starting revision and cannot overwrite an intervening edit or
/// a reopened snapshot with the same symbol IDs. The person can reopen it with current values.
#[test]
fn stale_field_dialogs_cannot_edit_the_current_project() {
    for reopen in [false, true] {
        let mut harness = harness();
        resistor_at_origin(&mut harness);
        click_canvas(&mut harness);
        harness.key_press(Key::E);
        harness.run();
        harness.get_by_label("Value").type_text("stale value");
        harness.run();
        let app = harness.state_mut();
        if reopen {
            app.session.open_snapshot(app.session.snapshot().unwrap());
        } else {
            app.session
                .apply(
                    SET_SYMBOL_FIELDS,
                    &json!({"symbol": "symbol-2", "reference": "R1", "value": "new value", "expected_revision": app.session.revision()}),
                )
                .unwrap();
        }
        let before = harness.state().session.inspect();
        click(&mut harness, "OK");
        assert_eq!(harness.state().session.inspect(), before);
        assert!(matches!(harness.state().dialog, Dialog::None));
        assert_eq!(harness.state().message.as_deref(), Some("View or project changed; reopen the symbol fields to continue editing."));
    }
}

/// An incorrectly addressed shortcut leaves both the project and the key event untouched so
/// the actual input-owning view can process it; routing to that view then performs one edit.
#[test]
fn only_the_addressed_view_consumes_editor_shortcuts() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    click_canvas(&mut harness);
    let app = harness.state_mut();
    let before = app.session.inspect();
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key: Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        let ctx = ui.ctx();
        app.shortcuts(ctx, false, ViewId(99));
        assert_eq!(app.session.inspect(), before);
        assert!(ctx.input(|input| input.key_pressed(Key::Delete)));
        app.shortcuts(ctx, false, app.views.active.id());
    });
    output.drop_without_applying_deltas();
    assert!(app.session.project().unwrap().symbols().is_empty());
    assert_eq!(app.session.revision(), before["revision"].as_u64().unwrap() + 1);
}
