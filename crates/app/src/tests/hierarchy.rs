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
        h.key_press_modifiers(Modifiers::SHIFT, Key::H);
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
    h.key_press_modifiers(Modifiers::SHIFT, Key::H);
    h.run();
    click(&mut h, &format!("Schematic canvas {}", second.0 + 1));
    name(&mut h, "DATA");
    let p = h.state().session.project().unwrap();
    assert_eq!(p.sheet_pins(&p.sheet_symbols()[0]).next().unwrap().label.name, "DATA");
    assert_eq!(h.state().views.view(first).unwrap().target(), Some(DocumentTarget::Sheet(parent)));
    assert_eq!(h.state().views.view(first).unwrap().camera.saved_state(), before);
    assert_eq!(h.state().views.view(second).unwrap().target(), Some(DocumentTarget::Sheet(child)));
}

/// Hovered hierarchy edits take precedence over a component selected earlier, so E/Delete
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
