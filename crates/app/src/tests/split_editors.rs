use pcbcraft_engine::{persistence::encode, schematic::Point};

use super::*;

/// Open two visible views through the ribbon, keeping the pointer fixtures clear of floating cards.
fn split_harness<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    resistor_at_origin(&mut h);
    h.state_mut().browser.open = false;
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    h.get_by_label("Schematic canvas 1");
    h.get_by_label("Schematic canvas 2");
    h
}

/// Resolve a view by identity rather than by the slot currently holding keyboard focus.
fn view<'a>(h: &'a Harness<'_, PcbApp>, id: u64) -> &'a SchematicView {
    let views = &h.state().views;
    if views.active.id() == ViewId(id) { &views.active } else { &views.other }
}

/// The canvas rectangle keeps its identity when focus moves between panes.
fn canvas(h: &Harness<'_, PcbApp>, id: u64) -> egui::Rect {
    h.get_by_label(&format!("Schematic canvas {}", id + 1)).rect()
}

/// Two views of one sheet have the same path; their stable pane order distinguishes the tabs.
fn focus_tab(h: &mut Harness<'_, PcbApp>, id: usize) {
    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    h.get_all_by_label(&path).nth(id).unwrap().click();
    h.run();
}

/// Click a model point in the specified pane, allowing its first press to activate the view.
fn click_point(h: &mut Harness<'_, PcbApp>, id: u64, point: Point) {
    let at = view(h, id).camera.screen(point, canvas(h, id));
    click_canvas_at(h, at);
}

/// Inspect the rendered symbol ink, so highlight tests cover paint and repaint, not just state.
fn reference_color(h: &Harness<'_, PcbApp>, id: u64, reference: &str) -> egui::Color32 {
    let rect = canvas(h, id);
    h.output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == reference && rect.contains(text.pos) => Some(text.fallback_color),
            _ => None,
        })
        .unwrap()
}

/// A selection projects in either direction without taking focus, moving cameras, dirtying the
/// document or acquiring an edit selection in the receiver. Tab focus cannot turn it into Delete.
#[test]
fn linked_highlights_are_visible_without_becoming_edit_selections() {
    for source in [0, 1] {
        let mut h = split_harness();
        let target = 1 - source;
        let snapshot = h.state().session.snapshot().unwrap();
        let bytes = encode(snapshot.clone()).unwrap();
        let revision = h.state().session.revision();
        let origin = Point { x_nm: 0, y_nm: 0 };
        let centers = [view(&h, 0).camera.screen(origin, canvas(&h, 0)), view(&h, 1).camera.screen(origin, canvas(&h, 1))];
        click_point(&mut h, source, origin);
        assert_eq!(h.state().views.active.id(), ViewId(source));
        assert!(view(&h, target).selection.is_empty());
        assert_eq!(h.state().views.highlight_for(ViewId(target), &h.state().session), view(&h, source).selection);
        assert_eq!(reference_color(&h, source, "R1"), Tokens::get().accent);
        assert_eq!(reference_color(&h, target, "R1"), Tokens::get().schematic_highlight);
        for id in [0, 1] {
            assert_eq!(view(&h, id).camera.screen(origin, canvas(&h, id)), centers[id as usize]);
        }
        focus_tab(&mut h, target as usize);
        assert_eq!(reference_color(&h, target, "R1"), Tokens::get().schematic_highlight);
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        assert_eq!(encode(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        // Selecting the counterpart deliberately becomes the new source, with no feedback loop.
        click_point(&mut h, target, origin);
        assert!(h.state().views.highlight_for(ViewId(target), &h.state().session).is_empty());
        assert_eq!(h.state().views.highlight_for(ViewId(source), &h.state().session).single_symbol(), Some("symbol-2"));
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.highlight_for(ViewId(source), &h.state().session).is_empty());
        assert_eq!(reference_color(&h, target, "R1"), Tokens::get().text);
    }
}

/// Browser picks use the same projection, including wires. Shared edits prune it, undo does
/// not revive a removed selection, and reopening rejects both the highlight and delayed input.
#[test]
fn browser_highlights_prune_deleted_targets_and_reset_on_reopen() {
    let mut h = split_harness();
    h.state_mut().run(
        pcbcraft_engine::DRAW_WIRE,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"x_nm":12_700_000,"y_nm":0},"bends":[],"expected_revision":2}),
    );
    let wire = h.state().session.project().unwrap().wires()[0].id();
    h.run();
    focus_tab(&mut h, 1);
    click(&mut h, "Toggle Browser");
    click(&mut h, "Expand Symbols");
    click(&mut h, "R1  1k");
    assert_eq!(reference_color(&h, 0, "R1"), Tokens::get().schematic_highlight);
    click(&mut h, "Expand Nets");
    click(&mut h, "Net 1");
    assert_eq!(h.state().views.highlight_for(ViewId(0), &h.state().session).single_wire(), Some(wire.as_str()));
    let rect = canvas(&h, 0);
    assert!(h.output().shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Path(path) =>
            path.stroke.color == egui::epaint::ColorMode::Solid(Tokens::get().schematic_highlight)
                && path.points.iter().all(|point| rect.contains(*point)),
        _ => false,
    }));
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::DELETE_WIRE, &json!({"wire":wire,"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
    click(&mut h, "Undo");
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
    click(&mut h, "R1  1k");
    let source = h.state().views.active.input(&h.state().session).unwrap();
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    let app = h.state_mut();
    app.view_click(source, Click::SelectItems(Selection::one_symbol("symbol-2".to_owned())));
    assert!(app.views.active.selection.is_empty());
    assert!(app.views.other.selection.is_empty());
    assert!(app.views.highlight_for(ViewId(0), &app.session).is_empty());
}

/// Each pane selects and navigates independently. Both issue edits through one history, and
/// switching panes must never send Delete to the other pane's retained selection.
#[test]
fn split_views_keep_navigation_and_selection_but_share_undo() {
    let mut h = split_harness();
    let snapshot = h.state().session.snapshot().unwrap();
    let bytes = encode(snapshot.clone()).unwrap();
    let before = h.state().session.inspect();
    let dirty = h.state().files.dirty(&h.state().session);
    assert!(canvas(&h, 0).right() < canvas(&h, 1).left());
    click_point(&mut h, 1, Point { x_nm: 0, y_nm: 0 });
    assert_eq!(h.state().views.active.id(), ViewId(1));
    assert_eq!(view(&h, 1).selection.single_symbol(), Some("symbol-2"));
    assert!(view(&h, 0).selection.single_symbol().is_none());
    h.hover_at(canvas(&h, 1).center());
    h.run();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 2.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let second_zoom = view(&h, 1).camera.zoom();
    assert!(second_zoom > 1.0);
    assert_eq!(view(&h, 0).camera.zoom(), 1.0);
    focus_tab(&mut h, 0);
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().session.inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    assert_eq!(encode(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(h.state().files.dirty(&h.state().session), dirty);
    focus_tab(&mut h, 1);
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().symbols().is_empty());
    assert!(view(&h, 0).selection.single_symbol().is_none());
    assert!(view(&h, 1).selection.single_symbol().is_none());
    focus_tab(&mut h, 0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
    assert_eq!(view(&h, 1).camera.zoom(), second_zoom);
    focus_tab(&mut h, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run();
    assert!(h.state().session.project().unwrap().symbols().is_empty());
    assert_eq!(h.state().session.revision(), before["revision"].as_u64().unwrap() + 3);
}

/// Leaving and returning to a pane invalidates its unfinished gesture without a document edit;
/// old events cannot commit merely because their view ID and document revision match again.
#[test]
fn returning_to_a_pane_does_not_revive_its_wire_gesture() {
    let mut h = split_harness();
    h.key_press(Key::W);
    h.run();
    click_point(&mut h, 0, Point { x_nm: 12_700_000, y_nm: 0 });
    let source = h.state().views.active.input(&h.state().session).unwrap();
    assert!(matches!(view(&h, 0).tool, Some(Tool::Wire { start: Some(_), .. })));
    let before = h.state().session.inspect();
    focus_tab(&mut h, 1);
    assert!(view(&h, 0).tool.is_none());
    focus_tab(&mut h, 0);
    let app = h.state_mut();
    assert!(!app.views.active.accepts(source, &app.session));
    app.view_click(source, Click::Select(Some("symbol-2".to_owned())));
    assert!(app.views.active.selection.single_symbol().is_none());
    assert_eq!(app.session.inspect(), before);
}

/// Collapsing a split retains the active pane and the hidden pane's navigation/selection. A
/// narrow viewport stacks panes and dragging the divider cannot make them overlap or escape.
#[test]
fn split_layout_preserves_views_and_fits_narrow_windows() {
    let mut h = split_harness();
    click_point(&mut h, 1, Point { x_nm: 0, y_nm: 0 });
    let before = h.state().session.inspect();
    click(&mut h, "Split editor views");
    h.get_by_label("Schematic canvas");
    assert_eq!(h.state().views.active.id(), ViewId(1));
    click(&mut h, "Split editor views");
    assert_eq!(view(&h, 1).selection.single_symbol(), Some("symbol-2"));
    h.set_size(egui::vec2(640.0, 800.0));
    h.run();
    assert!(canvas(&h, 0).bottom() < canvas(&h, 1).top());
    let divider = h.get_by_label("Resize editor panes").rect().center();
    h.drag_at(divider);
    h.run();
    h.hover_at(divider + egui::vec2(0.0, 300.0));
    h.run();
    h.drop_at(divider + egui::vec2(0.0, 300.0));
    h.run();
    assert!(canvas(&h, 0).bottom() < canvas(&h, 1).top());
    for id in [0, 1] {
        let rect = canvas(&h, id);
        assert!(rect.width() > 0.0 && rect.height() > 0.0);
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 800.0)).contains_rect(rect));
    }
    assert_eq!(h.state().session.inspect(), before);
}

/// A modal retains input even when the other pane is clicked, and project replacement clears
/// selection and drafts in both views before either can operate on reused local object IDs.
#[test]
fn modal_and_replacement_keep_both_views_on_the_current_project() {
    let mut h = split_harness();
    click_point(&mut h, 0, Point { x_nm: 0, y_nm: 0 });
    h.key_press(Key::E);
    h.run();
    let other = canvas(&h, 1).center();
    h.drag_at(other);
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(0));
    h.drop_at(other);
    h.run();
    if !matches!(h.state().dialog, Dialog::None) {
        click(&mut h, "Cancel");
    }
    click_point(&mut h, 1, Point { x_nm: 0, y_nm: 0 });
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    let reopened = h.state().session.inspect();
    h.run();
    for id in [0, 1] {
        assert!(view(&h, id).selection.single_symbol().is_none());
    }
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().session.inspect(), reopened);
}

/// Parts belong to the active editor: crossing the divider while dragging does not transfer
/// the gesture. A fresh placement in the second pane still participates in the shared undo.
#[test]
fn parts_drag_cannot_place_into_an_inactive_pane() {
    let mut h = split_harness();
    h.key_press(Key::A);
    h.run();
    let from = h.get_by_label("Capacitor").rect().center();
    let to = canvas(&h, 1).center();
    let before = h.state().session.inspect();
    h.drag_at(from);
    h.run();
    h.hover_at(from + egui::vec2(15.0, 0.0));
    h.run();
    h.hover_at(to);
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(0));
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().session.inspect(), before);
    focus_tab(&mut h, 1);
    h.key_press(Key::A);
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("cap");
    h.run();
    assert!(view(&h, 1).tool.is_none(), "Typing in search must not start a canvas tool");
    click(&mut h, "Capacitor");
    click(&mut h, "Close Parts library");
    click_point(&mut h, 1, Point { x_nm: 12_700_000, y_nm: 0 });
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    focus_tab(&mut h, 0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
}

/// The stationary Browser targets the focused view, and its reveal action centers only that pane.
/// Home suspends both panes without altering their project, local selection or view layout.
#[test]
fn browser_and_home_preserve_view_identity_and_project_state() {
    let mut h = split_harness();
    focus_tab(&mut h, 1);
    let first_rect = canvas(&h, 0);
    let origin = Point { x_nm: 0, y_nm: 0 };
    let first_origin = view(&h, 0).camera.screen(origin, first_rect);
    let before = h.state().session.inspect();
    click(&mut h, "Toggle Browser");
    click(&mut h, "Expand Symbols");
    click(&mut h, "R1  1k");
    assert_eq!(view(&h, 1).selection.single_symbol(), Some("symbol-2"));
    assert!(view(&h, 0).selection.single_symbol().is_none());
    assert_eq!(view(&h, 0).camera.screen(origin, canvas(&h, 0)), first_origin);
    let source = h.state().views.active.input(&h.state().session).unwrap();
    open_home(&mut h);
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.split);
    assert_eq!(h.state().views.active.id(), ViewId(1));
    assert_eq!(view(&h, 1).selection.single_symbol(), Some("symbol-2"));
    assert!(!h.state().views.active.accepts(source, &h.state().session));
    assert_eq!(h.state().session.inspect(), before);
}

/// A floating text editor over the inactive pane owns its pointer and keys. The focus prepass
/// must consult the topmost egui layer rather than activating the canvas hidden underneath it.
#[test]
fn an_overlay_does_not_activate_the_pane_under_its_text_field() {
    let mut text = String::new();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        move |ui, app: &mut PcbApp| {
            app.frame(ui);
            if app.views.split {
                egui::Window::new("Overlay").fixed_pos(egui::pos2(950.0, 380.0)).show(ui.ctx(), |ui| {
                    let label = ui.label("Overlay input");
                    ui.text_edit_singleline(&mut text).labelled_by(label.id);
                });
            }
        },
        PcbApp::default(),
    );
    h.run();
    resistor_at_origin(&mut h);
    h.state_mut().browser.open = false;
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let before = h.state().session.inspect();
    let field = h.get_by_label("Overlay input").rect().center();
    assert!(canvas(&h, 1).contains(field));
    click_canvas_at(&mut h, field);
    h.get_by_label("Overlay input").type_text("WAR");
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(0));
    assert!(h.state().views.active.tool.is_none());
    assert_eq!(h.state().session.inspect(), before);
}

/// A drag beginning in the inactive pane activates it and commits one shared move. The earlier
/// painted pane must repaint too, so both reference labels follow the resulting model position.
#[test]
fn moving_in_the_second_pane_repaints_both_canvases() {
    let mut h = split_harness();
    let revision = h.state().session.revision();
    let from = view(&h, 1).camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas(&h, 1));
    let to = from + egui::vec2(60.0, 20.0);
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(1));
    assert_eq!(h.state().session.revision(), revision + 1);
    let position = h.state().session.project().unwrap().symbols()[0].position;
    assert_ne!(position.x_nm, 0);
    for id in [0, 1] {
        let rect = canvas(&h, id);
        let expected = view(&h, id).camera.screen(position, rect);
        let text = h
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "R1" && rect.contains(text.pos) => Some(text),
                _ => None,
            })
            .unwrap();
        assert!((text.pos.x + text.galley.size().x * 0.5 - expected.x).abs() < 1.0);
    }
    focus_tab(&mut h, 0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().session.project().unwrap().symbols()[0].position, Point { x_nm: 0, y_nm: 0 });
}

/// Changing focus must not move panels, resize canvases, or pan either view. The same sheet
/// has the same path on both tabs, and a project rename updates both without rebinding them.
#[test]
fn panels_stay_put_and_tabs_name_the_sheet_in_both_views() {
    let mut h = split_harness();
    click(&mut h, "Toggle Browser");
    click(&mut h, "Toggle Parts");
    click(&mut h, "Toggle Net Inspector");
    let panel_rects = (h.state().browser.rect(), h.state().parts.rect(), h.get_by_label("Net Inspector").rect());
    let canvases = [canvas(&h, 0), canvas(&h, 1)];
    let origin = Point { x_nm: 0, y_nm: 0 };
    let centers = [view(&h, 0).camera.screen(origin, canvases[0]), view(&h, 1).camera.screen(origin, canvases[1])];
    let before = h.state().session.inspect();
    for id in [1, 0, 1] {
        focus_tab(&mut h, id);
        assert_eq!(h.state().views.active.id(), ViewId(id as u64));
        assert_eq!(panel_rects, (h.state().browser.rect(), h.state().parts.rect(), h.get_by_label("Net Inspector").rect()));
        for id in [0, 1] {
            assert_eq!(canvas(&h, id), canvases[id as usize]);
            assert_eq!(view(&h, id).camera.screen(origin, canvas(&h, id)), centers[id as usize]);
        }
    }
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(h.get_all_by_label("edits › Main").count(), 2);
    assert!(h.query_by_label("Schematic 1").is_none());
    assert!(h.query_by_label("Schematic 2").is_none());
    let targets = [view(&h, 0).input(&h.state().session).unwrap(), view(&h, 1).input(&h.state().session).unwrap()];
    h.state_mut().run("project.rename", &json!({"name":"Power controller"}));
    h.run();
    assert_eq!(h.get_all_by_label("Power controller › Main").count(), 2);
    assert!(h.query_by_label("edits › Main").is_none());
    for id in [0, 1] {
        assert_eq!(view(&h, id).id(), ViewId(id));
        assert!(!view(&h, id).accepts(targets[id as usize], &h.state().session));
    }
    focus_tab(&mut h, 0);
    click(&mut h, "Undo");
    assert_eq!(h.get_all_by_label("edits › Main").count(), 2);
}

/// PR #71's selection modifiers, both box directions/buttons and group edits remain available
/// in either pane. Its multi-selection belongs to that view, while move/delete undo is shared.
#[test]
fn box_selection_and_atomic_group_edits_work_in_each_pane() {
    use egui::PointerButton;
    use pcbcraft_engine::schematic::GRID_NM;
    for id in [0, 1] {
        for button in [PointerButton::Primary, PointerButton::Secondary] {
            let mut h = split_harness();
            h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":20*GRID_NM,"y_nm":0,"expected_revision":2}));
            h.state_mut().run(
                pcbcraft_engine::DRAW_WIRE,
                &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"bends":[],"expected_revision":3}),
            );
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let revision = h.state().session.revision();
            // The first drag also activates an initially inactive pane.
            for (a, b, modifiers, count) in [
                ((-7, -7), (7, 7), Modifiers::NONE, 1),
                ((13, -7), (27, 7), Modifiers::SHIFT, 2),
                ((-7, -7), (7, 7), Modifiers::CTRL, 1),
                ((-1, -5), (1, 5), Modifiers::NONE, 0),
                ((1, -5), (-1, 5), Modifiers::NONE, 1),
                ((-7, -7), (27, 7), Modifiers::NONE, 3),
            ] {
                let from = view(&h, id).camera.screen(Point { x_nm: a.0 * GRID_NM, y_nm: a.1 * GRID_NM }, canvas(&h, id));
                let to = view(&h, id).camera.screen(Point { x_nm: b.0 * GRID_NM, y_nm: b.1 * GRID_NM }, canvas(&h, id));
                h.event(egui::Event::ModifiersChanged(modifiers));
                h.event(egui::Event::PointerMoved(from));
                h.event(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers });
                h.run();
                h.hover_at(to);
                h.run();
                h.event(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers });
                h.run();
                h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
                h.run();
                assert_eq!(view(&h, id).selection.len(), count);
                assert!(view(&h, 1 - id).selection.is_empty());
                assert_eq!(h.state().views.highlight_for(ViewId(1 - id), &h.state().session), view(&h, id).selection);
                assert_eq!(h.state().session.revision(), revision);
            }
            let from = view(&h, id).camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas(&h, id));
            let to = from + egui::vec2(20.0, 30.0);
            h.drag_at(from);
            h.run();
            h.hover_at(to);
            h.run();
            assert!(matches!(view(&h, id).tool, Some(Tool::MoveItems { .. })));
            h.drop_at(to);
            h.run();
            assert_eq!(h.state().session.revision(), revision + 1);
            for (old, new) in before.symbols().iter().zip(h.state().session.project().unwrap().symbols()) {
                assert_eq!(new.position, Point { x_nm: old.position.x_nm + 2 * GRID_NM, y_nm: old.position.y_nm + 3 * GRID_NM });
            }
            focus_tab(&mut h, (1 - id) as usize);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            focus_tab(&mut h, id as usize);
            h.key_press(Key::Delete);
            h.run();
            assert!(h.state().session.project().unwrap().symbols().is_empty());
            assert!(h.state().session.project().unwrap().wires().is_empty());
            focus_tab(&mut h, (1 - id) as usize);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}
