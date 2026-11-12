use pcbcraft_engine::{MOVE_SYMBOL, PLACE_SYMBOL, persistence::encode_documents, schematic::Point};
use serde_json::{Value, json};

use super::*;

/// Build through commands so both projects can deliberately contain the same local symbol ID.
fn session() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "view test", "create_board":true})).unwrap();
    session.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
    session
}

/// A second view reads the same authority but cannot reuse the first view's input address.
/// Edits and undo invalidate both views' drafts without changing their binding or selection.
#[test]
fn views_share_history_but_keep_local_interactions() {
    let mut session = session();
    let mut first = EditorView::new(ViewId(1));
    let mut second = EditorView::new(ViewId(2));
    first.sync(&session);
    second.sync(&session);
    let source = first.input(&session).unwrap();
    let widget = first.widget_id();
    assert!(!second.accepts(source, &session));
    assert_ne!(widget, second.widget_id());
    first.schematic.selection = pcbcraft_engine::selection::Selection::one_symbol("symbol-2".to_owned());
    first.schematic.tool = Some(Tool::Wire { revision: session.revision(), start: None });
    first.schematic.wire_draft.horizontal_first = false;
    assert!(second.schematic.selection.single_symbol().is_none());
    assert!(second.schematic.tool.is_none());
    assert!(second.schematic.wire_draft.horizontal_first);

    session.apply(MOVE_SYMBOL, &json!({"symbol": "symbol-2", "x_nm": 1_270_000, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
    assert!(!first.accepts(source, &session));
    assert!(!first.sync(&session));
    assert!(!second.sync(&session));
    assert!(first.schematic.tool.is_none());
    assert!(first.schematic.wire_draft.horizontal_first);
    assert_eq!(first.schematic.selection.single_symbol(), Some("symbol-2"));
    assert_eq!(first.widget_id(), widget);
    let after_edit = second.input(&session).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    first.sync(&session);
    second.sync(&session);
    assert_eq!(session.project().unwrap().symbols()[0].position, Point { x_nm: 0, y_nm: 0 });
    assert!(!second.accepts(after_edit, &session));
    session.apply("edit.redo", &Value::Null).unwrap();
    assert!(!second.accepts(after_edit, &session), "Returning to the same snapshot must not revive old input");
}

/// Reopening the same file and replacing it with a project that reuses local IDs both reset view
/// state. Neither navigation state nor the runtime opening identity changes serialized bytes.
#[test]
fn replacement_and_reopening_reset_the_view_without_editing_the_project() {
    let mut session = session();
    let snapshot = session.snapshot().unwrap();
    let bytes = encode_documents(snapshot.clone()).unwrap();
    let target = snapshot.sheet_target();
    let mut view = EditorView::new(ViewId(7));
    view.sync(&session);
    let source = view.input(&session).unwrap();
    let widget = view.widget_id();
    view.schematic.selection = pcbcraft_engine::selection::Selection::one_symbol("symbol-2".to_owned());
    view.schematic.tool = Some(Tool::Wire { revision: session.revision(), start: None });
    session.open_snapshot(snapshot);
    assert!(view.input(&session).is_none(), "An unrefreshed binding must reject input");
    assert!(view.sync(&session));
    assert_eq!(view.id(), ViewId(7));
    assert!(view.schematic.selection.single_symbol().is_none());
    assert!(view.schematic.tool.is_none());
    assert!(!view.accepts(source, &session));
    assert_ne!(view.widget_id(), widget);
    assert_eq!(session.project().unwrap().sheet_target(), target);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    let root = session.history_root_revision();
    session.apply("project.rename", &json!({"name": "renamed"})).unwrap();
    assert_eq!(session.project().unwrap().sheet_target(), target);
    assert_eq!(session.history_root_revision(), root);
    assert!(!view.sync(&session));

    let mut replacement = Session::default();
    replacement.apply("project.new", &json!({"name": "view test", "create_board":true})).unwrap();
    replacement.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": replacement.revision()})).unwrap();
    assert_eq!(replacement.project().unwrap().symbols()[0].id(), "symbol-2");
    assert_ne!(replacement.project().unwrap().sheet_target(), target);
    let before_open = view.input(&session).unwrap();
    session.open_snapshot(replacement.snapshot().unwrap());
    assert!(view.sync(&session));
    assert!(!view.accepts(before_open, &session));
    assert!(session.history_root_revision() > root);
}

/// Project replacement cannot silently retarget an explicit board; the placeholder keeps its
/// address and navigation, rejects stale input, and permits an explicit workspace replacement.
#[test]
fn replacement_keeps_an_explicit_foreign_board_unavailable() {
    let replacement = session();
    let mut session = session();
    let mut view = EditorView::new(ViewId(7));
    view.sync(&session);
    view.switch(Workspace::Board, &session);
    let old_target = view.board.target.unwrap();
    view.camera.center_on(Point { x_nm: 15_000_000, y_nm: 4_000_000 });
    let camera = view.camera.saved_state();
    let old_input = view.input(&session).unwrap();
    let bytes = encode_documents(replacement.snapshot().unwrap()).unwrap();
    session.open_snapshot(replacement.snapshot().unwrap());
    assert!(view.sync(&session));
    assert!(view.unavailable());
    assert!(view.input(&session).is_none());
    assert!(!view.accepts(old_input, &session));
    assert_eq!(view.board.target, Some(old_target));
    assert_eq!(view.control_input(&session).unwrap().target, Some(DocumentTarget::Board(old_target)));
    let saved = view.saved_state(&session).unwrap();
    assert_eq!(saved.target, DocumentTarget::Board(old_target));
    assert_eq!(saved.camera, camera);
    assert!(!view.sync(&session));
    assert_eq!(view.saved_state(&session).unwrap(), saved);
    view.switch(Workspace::Board, &session);
    assert!(!view.unavailable());
    assert_eq!(view.target(), Some(DocumentTarget::Board(session.project().unwrap().board_target().unwrap())));
    assert!(!view.accepts(old_input, &session));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// Only an omitted board address may use the legacy default; an unknown ID in this project
/// remains an addressable placeholder rather than granting input to the default board.
#[test]
fn board_sync_defaults_only_an_omitted_target() {
    let session = session();
    let target = session.project().unwrap().board_target().unwrap();
    let mut view = EditorView::new(ViewId(7));
    view.workspace = Workspace::Board;
    assert!(view.sync(&session));
    assert_eq!(view.target(), Some(DocumentTarget::Board(target)));
    let unknown = BoardTarget { board: Default::default(), ..target };
    view.board.target = Some(unknown);
    assert!(view.sync(&session));
    assert!(view.unavailable());
    assert!(view.input(&session).is_none());
    assert_eq!(view.saved_state(&session).unwrap().target, DocumentTarget::Board(unknown));
    assert_eq!(view.board.target, Some(unknown));
}

/// Undoing creation removes the view's target and transient state; redo binds the restored sheet
/// with fresh input, while deleting or undoing a symbol clears only the missing selection.
#[test]
fn undo_prunes_selection_and_unbinds_an_absent_project() {
    let mut session = session();
    let mut view = EditorView::default();
    view.sync(&session);
    let target = session.project().unwrap().sheet_target();
    view.schematic.selection = pcbcraft_engine::selection::Selection::one_symbol("symbol-2".to_owned());
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(!view.sync(&session));
    assert!(view.schematic.selection.single_symbol().is_none());
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(view.sync(&session));
    assert!(view.input(&session).is_none());
    session.apply("edit.redo", &Value::Null).unwrap();
    assert!(view.sync(&session));
    assert_eq!(session.project().unwrap().sheet_target(), target);
    assert!(view.input(&session).is_some());
}
