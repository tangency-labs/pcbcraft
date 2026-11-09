//! Flat-sheet acceptance through actual Browser controls and explicitly addressed view input.
use super::*;
use crate::editor_view::DocumentTarget;
use pcbcraft_engine::{
    PLACE_RESISTOR,
    persistence::encode_documents,
    sheets::{CREATE_SHEET, DELETE_SHEET},
};

/// Start with one original resistor and the common Browser visible.
fn sheet_harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Multi sheet"}));
    app.run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Open a real Browser row's contextual actions with separate press/release input frames.
fn menu(h: &mut Harness<'_, PcbApp>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Replace the form's initial name and submit through its actual OK button.
fn name(h: &mut Harness<'_, PcbApp>, text: &str) {
    clear_name(h);
    h.get_by_label("Name").type_text(text);
    h.run();
    click(h, "OK");
}

/// Creating, naming, switching, deleting and undoing are complete GUI workflows at both sizes.
#[test]
fn browser_sheet_lifecycle_and_local_canvas_selection() {
    for width in [1400.0, 700.0] {
        let mut h = sheet_harness(width);
        let main = h.state().session.project().unwrap().sheet_target();
        menu(&mut h, "Sheets");
        click(&mut h, "New Sheet…");
        name(&mut h, "Power");
        assert!(matches!(h.state().dialog, Dialog::None));
        let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
        assert!(h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="2 / 2")));
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
        h.run();
        click(&mut h, "Schematic canvas");
        let selected = h.state().views.active.schematic.selection.single_symbol().unwrap().to_owned();
        assert_eq!(h.state().selected_reference().as_deref(), Some("R2"));
        click(&mut h, "Main");
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(main)));
        assert!(h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="1 / 2")));
        click(&mut h, "Schematic canvas");
        assert_eq!(h.state().selected_reference().as_deref(), Some("R1"));
        click(&mut h, "Power");
        assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some(selected.as_str()));
        menu(&mut h, "Power");
        click(&mut h, "Rename Sheet…");
        name(&mut h, "Supply");
        h.get_by_label("Supply");
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        menu(&mut h, "Supply");
        click(&mut h, "Delete Sheet…");
        click(&mut h, "Cancel");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        menu(&mut h, "Supply");
        click(&mut h, "Delete Sheet…");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete Sheet").click();
        h.run();
        assert_eq!(h.state().session.project().unwrap().sheets().len(), 1);
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(main)));
        click(&mut h, "Undo");
        h.get_by_label("Supply");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Two panes read separate topology/geometry and GUI commands take the focused pane's address.
/// Editing another sheet never retargets, pans or selects the first pane.
#[test]
fn different_sheets_in_split_panes_keep_edits_and_net_inspection_local() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let main = h.state().session.project().unwrap().sheet_target();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let first = h.state().views.active.id();
    click(&mut h, "Schematic canvas 2");
    click(&mut h, "Power");
    let second = h.state().views.active.id();
    assert_ne!(first, second);
    let before = h.state().views.view(first).unwrap().camera.saved_state();
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    click(&mut h, &format!("Schematic canvas {}", second.0 + 1));
    assert_eq!(h.state().selected_reference().as_deref(), Some("R2"));
    assert_eq!(h.state().views.view(first).unwrap().target(), Some(DocumentTarget::Sheet(main)));
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(h.state().views.view(first).unwrap().camera.saved_state(), before);
    assert!(h.state().views.view(first).unwrap().schematic.selection.is_empty());
    click(&mut h, "Toggle Net Inspector");
    h.get_by_label("R2.1");
    h.get_by_label("R2.2");
    assert!(h.query_by_label("R1.1").is_none());
    click(&mut h, &format!("Schematic canvas {}", first.0 + 1));
    h.get_by_label("R1.1");
    h.get_by_label("R1.2");
    assert!(h.query_by_label("R2.1").is_none());
}

/// A stale lifecycle form cannot edit a new revision, and a view whose sheet disappeared
/// cannot edit a surviving sheet. Undo revives the original address and camera.
#[test]
fn stale_sheet_forms_and_deleted_sheet_views_never_fall_back_to_editing_main() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    let id = h.state().views.active.id();
    let camera = h.state().views.active.camera.saved_state();
    menu(&mut h, "Power");
    click(&mut h, "Rename Sheet…");
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    name(&mut h, "Supply");
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let old = h.state().views.active.input(&h.state().session).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DELETE_SHEET, &json!({"sheet":power.id(),"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.view(id).unwrap().unavailable());
    assert!(!h.state().views.view(id).unwrap().accepts(old, &h.state().session));
    assert!(h.state().views.view(id).unwrap().snapshot(&h.state().session).is_none());
    h.state_mut().session.apply("edit.undo", &Value::Null).unwrap();
    h.run();
    assert_eq!(h.state().views.view(id).unwrap().target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(h.state().views.view(id).unwrap().camera.saved_state(), camera);
}

/// Deleting the active sheet externally cannot disable project-wide Save/Save As. A real
/// folder save still preserves the unavailable view, and its pending job blocks a second save.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unavailable_sheet_view_can_save_the_project_and_respects_pending_file_jobs() {
    let mut h = sheet_harness(1400.0);
    h.state_mut().files.use_stand_in();
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DELETE_SHEET, &json!({"sheet":power.id(),"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.active.unavailable());
    assert!(h.state().views.active.snapshot(&h.state().session).is_none());
    assert!(h.state().can_save_project());
    assert!(!h.get_by_label("Save Project").accesskit_node().is_disabled());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "File");
    assert!(!h.get_by_label("Save").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Save As…").accesskit_node().is_disabled());
    click(&mut h, "Save As…");
    assert!(matches!(h.state().dialog, Dialog::SaveEncoding));
    click(&mut h, "Cancel");
    click(&mut h, "File");
    h.get_by_label("Save").click();
    // The stand-in holds the real file job open until the simulated destination is chosen.
    h.run_steps(3);
    assert!(h.state().files.stand_in_waiting());
    assert!(!h.state().can_save_project());
    assert!(h.get_by_label("Save Project").accesskit_node().is_disabled());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved-project");
    h.state_mut().files.finish_stand_in(path.clone());
    h.run();
    assert!(h.state().can_save_project());
    assert!(h.state().views.active.unavailable());
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(encode_documents(pcbcraft_storage::open(&path).unwrap()).unwrap(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Live navigation validates the explicit sheet before selecting, independently of focused pane.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_sheet_navigation_and_selection_validate_the_document_address() {
    let mut h = sheet_harness(1400.0);
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let before = h.state().session.snapshot().unwrap();
    let source = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.open_sheet", &json!({"view":source.0,"sheet":power.id()})).unwrap();
    h.run();
    let target = h.state().views.active.id();
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert!(h.state_mut().run_window_command(&ctx, "view.select", &json!({"view":target.0,"items":{"symbols":["symbol-2"],"wires":[]}})).is_err());
    assert!(
        h.state_mut().run_window_command(&ctx, "view.open_sheet", &json!({"view":target.0,"sheet":"00000000-0000-0000-0000-000000000000"})).is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Saved view targets survive an offline project reopen, including the sheet associated with
/// a PCB view's parked schematic camera. Restoring presentation never changes project bytes.
#[test]
fn reopened_arrangement_keeps_sheet_targets_and_board_return_navigation() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run("pcb.create_board", &json!({"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 10_000_000, y_nm: 20_000_000 });
    crate::tests::open_browser_row(&mut h, "Board Board", "Open in PCB layout");
    let state = h.state().views.saved_state(&h.state().session).unwrap();
    let documents = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let mut app = PcbApp::default();
    app.session.open_snapshot(pcbcraft_engine::persistence::decode_documents(&documents).unwrap());
    app.views.restore_state(&state, &app.session).unwrap();
    assert_eq!(app.views.active.workspace, crate::editor_view::Workspace::Board);
    let source = app.views.active.id();
    app.apply_workspace(source, crate::editor_view::Workspace::Schematic, pcbcraft_chrome::workspace::Placement::Here);
    assert_eq!(app.views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(encode_documents(app.session.snapshot().unwrap()).unwrap(), documents);
}
