//! Schematic-only projects can show PCB presentation without acquiring a physical board.
use super::*;
use crate::editor_view::{DocumentTarget, EditorView};
use pcbcraft_engine::persistence::encode_documents;

/// Opening the PCB workspace is a view change; only Create board mutates the document.
#[test]
fn optional_board_is_created_explicitly_and_undo_restores_schematic_only_state() {
    let mut h = harness();
    new_project(&mut h);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert!(!before.contains_key("board.json"));
    assert!(h.state().session.project().unwrap().board_target().is_none());
    let revision = h.state().session.revision();

    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.get_by_label("Create board");
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::BoardWorkspace(h.state().session.project().unwrap().id())));
    assert!(h.state().views.active.board.target.is_none());
    assert!(!h.state().session.available(pcbcraft_engine::INITIALIZE_BOARD_SETUP));

    click(&mut h, "Create board");
    let board = h.state().session.project().unwrap().board_target().unwrap();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Board(board)));
    h.get_by_label("PCB canvas");
    assert!(encode_documents(h.state().session.snapshot().unwrap()).unwrap().contains_key("board.json"));

    h.state_mut().run("edit.undo", &Value::Null);
    h.run();
    assert!(h.state().session.project().unwrap().board_target().is_none());
    assert!(h.state().views.active.unavailable(), "An explicit removed board cannot become a different target");
    h.state_mut().run("edit.redo", &Value::Null);
    h.run();
    assert_eq!(h.state().session.project().unwrap().board_target(), Some(board));
    assert!(!h.state().views.active.unavailable());
}

/// Empty PCB arrangements retain a project-scoped presentation target, never a fake board UUID.
#[test]
fn empty_board_view_roundtrips_its_presentation_and_keeps_window_controls() {
    let mut session = pcbcraft_engine::Session::default();
    session.apply("project.new", &Value::Null).unwrap();
    let mut view = EditorView::new(ViewId(4));
    view.sync(&session);
    view.switch(Workspace::Board, &session);
    view.sync(&session);
    let saved = view.saved_state(&session).unwrap();
    let restored = EditorView::from_state(ViewId(5), &saved, &session);
    assert_eq!(restored.workspace, Workspace::Board);
    assert!(restored.board.target.is_none());
    assert!(restored.control_input(&session).is_some());
    assert_eq!(restored.saved_state(&session).unwrap(), saved);

    session.apply("pcb.create_board", &json!({"expected_revision":session.revision()})).unwrap();
    let restored = EditorView::from_state(ViewId(6), &saved, &session);
    assert_eq!(restored.board.target, session.project().unwrap().board_target());
    assert!(!restored.unavailable());
}

/// The package chooser requires a real board and cannot create one as a side effect.
#[test]
fn footprint_chooser_does_not_create_a_board_for_schematic_only_parts() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().begin_footprint_assignment(Some("component-1".into()));
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().message.as_deref(), Some("Create a board before assigning footprints."));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A press cannot follow project replacement or reopen, even when the release arrives at the same location.
#[test]
fn pending_create_board_click_cannot_follow_project_replacement_or_reopen() {
    for reopen in [false, true] {
        let mut h = harness();
        new_project(&mut h);
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        let point = h.get_by_label("Create board").rect().center();
        h.event(egui::Event::PointerMoved(point));
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run();
        let snapshot = h.state().session.snapshot().unwrap();
        if reopen {
            h.state_mut().session.open_snapshot(snapshot);
        } else {
            h.state_mut().run("project.new", &Value::Null);
        }
        h.run();
        let before_release = h.state().session.snapshot().unwrap();
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run();
        assert!(h.state().session.project().unwrap().board_target().is_none());
        assert!(Arc::ptr_eq(&h.state().session.snapshot().unwrap(), &before_release));
    }
}

/// A second click at the old CTA location cannot create another board or an extra undo entry.
#[test]
fn repeated_create_board_clicks_only_create_one_board() {
    let mut h = harness();
    new_project(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let point = h.get_by_label("Create board").rect().center();
    click(&mut h, "Create board");
    let created = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    click_canvas_at(&mut h, point);
    assert_eq!(h.state().session.revision(), revision);
    assert!(Arc::ptr_eq(&h.state().session.snapshot().unwrap(), &created));
    assert_eq!(h.state().session.project().unwrap().boards().count(), 1);
}
