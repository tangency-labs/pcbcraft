use super::*;
use crate::editor_view::DocumentTarget;
use pcbcraft_engine::{
    hierarchy as engine,
    persistence::{decode_documents, encode_documents},
    schematic::Point,
};

/// Exercise actual placement controls at normal and compact widths.
fn hierarchy_harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Hierarchy"}));
    let mut h = Harness::builder()
        .with_step_dt(0.05)
        .with_size(egui::vec2(width, 900.))
        .with_max_steps(40)
        .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Replace the focused form field and commit using its real button.
fn name(h: &mut Harness<'_, PcbApp>, value: &str) {
    h.get_by_label("Name").click();
    h.run();
    clear_name(h);
    h.get_by_label("Name").type_text(value);
    h.run();
    click(h, "OK");
}

/// Send a complete pointer click at a model-derived position, with explicit timing frames.
fn point_click(h: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
    h.hover_at(point);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
}

/// Accepting the suggested name succeeds after deletion and skips consecutive names
/// regardless of case; creating the child still occupies exactly one undo step.
#[test]
fn default_child_sheet_name_skips_existing_names() {
    use pcbcraft_engine::sheets::{CREATE_SHEET, DELETE_SHEET};

    for (names, delete, expected) in
        [(vec!["Sheet 3", "Discard"], Some("Discard"), "Sheet 4"), (vec!["sHeEt 3"], None, "Sheet 4"), (vec!["Sheet 4", "SHEET 5"], None, "Sheet 6")]
    {
        let mut h = hierarchy_harness(1400.);
        for name in names {
            let revision = h.state().session.revision();
            h.state_mut().run(CREATE_SHEET, &json!({"name":name,"expected_revision":revision}));
        }
        if let Some(name) = delete {
            let sheet = h.state().session.project().unwrap().sheets().find(|sheet| sheet.name == name).unwrap().target;
            let revision = h.state().session.revision();
            h.state_mut().run(DELETE_SHEET, &json!({"sheet":sheet.id(),"expected_revision":revision}));
        }
        h.run();
        let before = h.state().session.snapshot().unwrap();
        h.key_press(Key::S);
        h.run();
        click(&mut h, "Schematic canvas");
        assert_eq!(h.get_by_label("Name").value().as_deref(), Some(expected));
        click(&mut h, "OK");
        assert!(matches!(h.state().dialog, Dialog::None), "{:?}", h.state().dialog);
        let project = h.state().session.project().unwrap();
        assert_eq!(project.sheets().len(), before.sheets().len() + 1);
        let child = project.child_sheet(project.sheet_symbols()[0].id).unwrap();
        assert_eq!(project.sheets().find(|sheet| sheet.target == child).unwrap().name, expected);
        h.state_mut().run("edit.undo", &Value::Null);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// S placement, Browser navigation, label placement, double-click entry and Alt+Backspace
/// use persistent document identities and survive save/reopen at both supported widths.
#[test]
fn hierarchical_sheet_and_label_workflow() {
    for width in [1400., 700.] {
        let mut h = hierarchy_harness(width);
        let parent = h.state().session.project().unwrap().sheet_target();
        h.key_press(Key::S);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Sheet, .. })));
        click(&mut h, "Schematic canvas");
        name(&mut h, "Power");
        assert!(matches!(h.state().dialog, Dialog::None), "{:?}", h.state().dialog);
        let block = h.state().session.project().unwrap().sheet_symbols()[0].clone();
        let child = h.state().session.project().unwrap().child_sheet(block.id).unwrap();
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(parent)));
        click(&mut h, "Power");
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(child)));
        h.key_press(Key::H);
        h.run();
        click(&mut h, "Schematic canvas");
        name(&mut h, "ENABLE");
        assert_eq!(h.state().views.active.snapshot(&h.state().session).unwrap().hierarchical_labels()[0].name, "ENABLE");
        h.key_press_modifiers(Modifiers::ALT, Key::Backspace);
        h.run();
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(parent)));
        let project = h.state().views.active.snapshot(&h.state().session).unwrap();
        assert_eq!(project.sheet_pins(&project.sheet_symbols()[0]).next().unwrap().label.name, "ENABLE");
        let rect = h.get_by_label("Schematic canvas").rect();
        let inside = h.state().views.active.camera.screen(
            Point { x_nm: block.at.x_nm + 4 * pcbcraft_engine::schematic::GRID_NM, y_nm: block.at.y_nm + 3 * pcbcraft_engine::schematic::GRID_NM },
            rect,
        );
        point_click(&mut h, inside);
        point_click(&mut h, inside);
        h.run();
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(child)));
        let docs = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let loaded = decode_documents(&docs).unwrap();
        assert_eq!(loaded.parent_sheet(child), Some(parent));
        assert_eq!(docs, encode_documents(loaded).unwrap());
    }
}

/// An edit prepared on an earlier revision never mutates the newly focused/reopened document.
#[test]
fn stale_hierarchy_form_is_inert_and_creation_undoes_as_one_step() {
    let mut h = hierarchy_harness(1400.);
    let before = h.state().session.snapshot().unwrap();
    h.key_press(Key::S);
    h.run();
    click(&mut h, "Schematic canvas");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Other","expected_revision":revision}));
    let changed = h.state().session.snapshot().unwrap();
    h.run();
    click(&mut h, "OK");
    assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
    assert!(h.state().message.as_deref().unwrap().contains("changed"));
    h.state_mut().run("edit.undo", &Value::Null);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let revision = h.state().session.revision();
    h.state_mut().run(engine::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    assert_eq!(h.state().session.project().unwrap().sheets().len(), 2);
    h.state_mut().run("edit.undo", &Value::Null);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A child label added from another pane updates the parent's pins without changing either
/// pane's document or camera. Delayed canvas actions cannot acquire the other pane's authority.
#[test]
fn hierarchy_edits_stay_bound_to_their_split_pane() {
    let mut h = hierarchy_harness(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    let parent = h.state().session.project().unwrap().sheet_target();
    let child = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let first = h.state().views.active.id();
    let before = h.state().views.active.camera.saved_state();
    click(&mut h, "Schematic canvas 2");
    click(&mut h, "Child");
    let second = h.state().views.active.id();
    assert_ne!(first, second);
    h.key_press(Key::H);
    h.run();
    click(&mut h, &format!("Schematic canvas {}", second.0 + 1));
    name(&mut h, "DATA");
    let p = h.state().session.project().unwrap();
    assert_eq!(p.sheet_pins(&p.sheet_symbols()[0]).next().unwrap().label.name, "DATA");
    assert_eq!(h.state().views.view(first).unwrap().target(), Some(DocumentTarget::Sheet(parent)));
    assert_eq!(h.state().views.view(first).unwrap().camera.saved_state(), before);
    assert_eq!(h.state().views.view(second).unwrap().target(), Some(DocumentTarget::Sheet(child)));
}

/// Hovered hierarchy edits take precedence over a component selected earlier, so M/E/Delete
/// cannot silently edit or remove that unrelated symbol.
#[test]
fn hierarchy_shortcuts_do_not_edit_an_old_component_selection() {
    let mut h = hierarchy_harness(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":20_000_000,"y_nm":0,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD_LABEL, &json!({"name":"DATA","direction":"input","x_nm":0,"y_nm":0,"expected_revision":revision}));
    let symbol = h.state().session.project().unwrap().symbols()[0].id();
    h.state_mut().select(Selection::one_symbol(symbol.clone()));
    h.run();
    let rect = h.get_by_label("Schematic canvas").rect();
    let pointer = h.state().views.active.camera.screen(Point { x_nm: -2 * pcbcraft_engine::schematic::GRID_NM, y_nm: 0 }, rect);
    h.hover_at(pointer);
    h.run();
    let before_move = h.state().session.inspect();
    h.key_press(Key::M);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Label, .. })));
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().session.inspect(), before_move);
    h.hover_at(pointer);
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::Hierarchy(_)));
    name(&mut h, "RENAMED");
    h.hover_at(pointer);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().hierarchical_labels().is_empty());
    assert_eq!(h.state().session.project().unwrap().symbols()[0].id(), symbol);
}

/// Build original geometry through commands and clear floating panels for coordinate gestures.
fn movable_sheet(width: f32) -> Harness<'static, PcbApp> {
    let mut h = hierarchy_harness(width);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::CREATE_CHILD, &json!({"name":"Power","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.state_mut().browser.open = false;
    h.run();
    h
}

/// Project model points using this pane's current camera and actual canvas rectangle.
fn screen(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect())
}

/// Direct mouse movement retains its grab offset. Corners resize from the opposite corner,
/// and each complete gesture is one undoable edit, at normal and compact sizes.
#[test]
fn sheet_body_and_title_drag_and_four_corner_resize() {
    let grid = pcbcraft_engine::schematic::GRID_NM;
    for width in [1400., 700.] {
        let mut h = movable_sheet(width);
        let before = h.state().session.snapshot().unwrap();
        let top = screen(&h, Point { x_nm: 0, y_nm: 0 }).y;
        let title = h
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Power" => Some(text),
                _ => None,
            })
            .unwrap();
        assert!(title.pos.y + title.galley.size().y < top, "sheet name must clear the outline");
        let from = screen(&h, Point { x_nm: 4 * grid, y_nm: 3 * grid });
        let to = from + egui::vec2(50., 30.);
        h.drag_at(from);
        h.run();
        h.hover_at(to);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::SheetGesture { .. })));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        h.drop_at(to);
        h.run();
        let moved = h.state().session.snapshot().unwrap();
        assert_eq!(moved.sheet_symbols()[0].at, Point { x_nm: 5 * grid, y_nm: 3 * grid });
        h.state_mut().run("edit.undo", &Value::Null);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        // The name is outside the outline and remains a pickup target.
        let title = screen(&h, Point { x_nm: 2 * grid, y_nm: 0 }) - egui::vec2(0., 10.);
        h.drag_at(title);
        h.run();
        h.hover_at(title + egui::vec2(-20., 20.));
        h.run();
        h.drop_at(title + egui::vec2(-20., 20.));
        h.run();
        assert_eq!(h.state().session.project().unwrap().sheet_symbols()[0].at, Point { x_nm: -2 * grid, y_nm: 2 * grid });
        h.state_mut().run("edit.undo", &Value::Null);
        h.run();
        for (x, y, dx, dy) in [(0, 0, -3, -2), (16, 0, 3, -2), (0, 10, -3, 2), (16, 10, 3, 2)] {
            let corner = screen(&h, Point { x_nm: x * grid, y_nm: y * grid });
            let target = corner + egui::vec2(dx as f32 * 10., dy as f32 * 10.);
            h.drag_at(corner);
            h.run();
            h.hover_at(target);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            h.drop_at(target);
            h.run();
            let p = h.state().session.project().unwrap();
            let block = &p.sheet_symbols()[0];
            let size = p.sheet_symbol_size(block);
            assert_eq!(size, Point { x_nm: 19 * grid, y_nm: 12 * grid });
            assert_eq!(block.at, Point { x_nm: if x == 0 { -3 * grid } else { 0 }, y_nm: if y == 0 { -2 * grid } else { 0 } });
            h.state_mut().run("edit.undo", &Value::Null);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Cancellation, leaving the canvas and stale source revisions cannot publish a resize.
#[test]
fn cancelled_and_stale_sheet_gestures_leave_the_document_unchanged() {
    let mut h = movable_sheet(1400.);
    let before = h.state().session.snapshot().unwrap();
    let corner = screen(&h, Point { x_nm: 16 * pcbcraft_engine::schematic::GRID_NM, y_nm: 10 * pcbcraft_engine::schematic::GRID_NM });
    h.drag_at(corner);
    h.run();
    h.hover_at(corner + egui::vec2(30., 40.));
    h.run();
    h.key_press(Key::Escape);
    h.run();
    h.drop_at(corner + egui::vec2(30., 40.));
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.drag_at(corner);
    h.run();
    h.hover_at(egui::pos2(2., 2.));
    h.run();
    h.drop_at(egui::pos2(2., 2.));
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.drag_at(corner);
    h.run();
    h.hover_at(corner + egui::vec2(30., 40.));
    h.run();
    let revision = h.state().session.revision();
    h.state_mut().run("project.rename", &json!({"name":"Changed"}));
    h.run();
    let changed = h.state().session.snapshot().unwrap();
    assert_ne!(h.state().session.revision(), revision);
    h.drop_at(corner + egui::vec2(30., 40.));
    h.run();
    assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
}

/// Resizing shows the connected route at the new pin before commit, then retains its exact
/// electrical identity and restores the original document with one Undo.
#[test]
fn resizing_sheet_previews_and_commits_attached_wire_geometry() {
    let grid = pcbcraft_engine::schematic::GRID_NM;
    let mut h = movable_sheet(1400.);
    let child = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let revision = h.state().session.revision();
    h.state_mut()
        .run(engine::ADD_LABEL, &json!({"sheet":child.id(),"name":"OUT","direction":"output","x_nm":0,"y_nm":0,"expected_revision":revision}));
    let p = h.state().session.project().unwrap();
    let pin = p.sheet_pins(&p.sheet_symbols()[0]).next().unwrap();
    let end = pcbcraft_engine::schematic::WireEndpoint::SheetPin(pin.label.id);
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::DRAW_WIRE,
        &json!({"from":end.inspect(),"to":{"x_nm":28*grid,"y_nm":4*grid},"bends":[],"auto_heal":false,"expected_revision":revision}),
    );
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let from = screen(&h, Point { x_nm: 16 * grid, y_nm: 10 * grid });
    let to = screen(&h, Point { x_nm: 20 * grid, y_nm: 12 * grid });
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let preview_pin = screen(&h, Point { x_nm: 20 * grid, y_nm: 4 * grid });
    assert!(
        h.output()
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.points.len()==2 && path.points[0].distance(preview_pin)<0.1)),
        "wire preview must follow the resized pin"
    );
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().session.project().unwrap().endpoint_position(end), Some(Point { x_nm: 20 * grid, y_nm: 4 * grid }));
    assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints()[0], end);
    h.state_mut().run("edit.undo", &Value::Null);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
