use super::*;
use crate::{ViewId, editor_view::Workspace, editor_windows::ViewAction, project_browser::Action};
use pcbcraft_engine::{PLACE_FOOTPRINT, persistence::encode_documents};
use std::sync::Arc;

/// The original connected schematic, with a board placement created through the engine.
fn project_harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = wired_app();
    let revision = app.session.revision();
    app.run(
        PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,
        "x_nm":127_000_000,"y_nm":25_400_000,"expected_revision":revision}),
    );
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).with_max_steps(30).build_ui_state(|ui, app| app.frame(ui), app);
    h.run();
    h
}

/// Repeated row clicks inspect a board's tree without replacing the current editor.
#[test]
fn repeated_browser_clicks_do_not_switch_workspaces() {
    let mut h = project_harness(1400.0);
    let before = h.state().session.snapshot().unwrap();
    for _ in 0..3 {
        h.get_by_label("Board Board").click();
        h.step();
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
    }
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Sheets and Boards have the same row metrics and navigation from both workspaces. Picking
/// an inactive document's object requires explicit opening before selecting its object.
#[test]
fn common_browser_navigates_both_workspaces_without_document_edits() {
    for width in [1400.0, 700.0] {
        let mut h = project_harness(width);
        let before = h.state().session.inspect();
        let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let sheet_height = h.get_by_label("Main").rect().height();
        assert_eq!(h.get_by_label("Board Board").rect().height(), sheet_height);
        h.get_by_label("Sheets");
        h.get_by_label("Boards");
        let camera = h.state().views.active.camera.saved_state();
        click(&mut h, "Board Board");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        assert_eq!(h.state().views.active.camera.saved_state(), camera);
        click(&mut h, "Footprint R1  R");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        crate::tests::open_browser_row(&mut h, "Board Board", "Open in PCB layout");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        h.get_by_label("Sheets");
        h.get_by_label("Main");
        h.get_by_label("Boards");
        assert_eq!(h.get_by_label("Footprint R1  R").rect().height(), sheet_height);
        click(&mut h, "Footprint R1  R");
        let board_selection = h.state().views.active.board.selection.clone();
        assert_eq!(board_selection.len(), 1);
        h.get_by_label("generic-through-hole-2");
        // The arrow can inspect a different document without navigating to it.
        click(&mut h, "Expand Main");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        click(&mut h, "Expand Symbols");
        click(&mut h, "R2  R");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        crate::tests::open_browser_row(&mut h, "R2  R", "Show in Schematic");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some("symbol-3"));
        assert_eq!(h.get_by_label("R1  R").rect().height(), sheet_height);
        // A footprint row opens another workspace only through an explicit action.
        click(&mut h, "Expand Board");
        click(&mut h, "Footprint R1  R");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        crate::tests::open_browser_row(&mut h, "Footprint R1  R", "Show in PCB layout");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        assert_eq!(h.state().views.active.board.selection, board_selection);
        assert_eq!(h.state().session.inspect(), before);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
}

/// A Browser tree fold, header collapse and visibility toggle are presentation changes. The
/// shared visibility flag applies to PCB too, with Layers remaining available beneath it.
#[test]
fn browser_folding_and_visibility_are_consistent_on_the_board() {
    let mut h = project_harness(1400.0);
    crate::tests::open_browser_row(&mut h, "Board Board", "Open in PCB layout");
    let before = h.state().session.inspect();
    click(&mut h, "Collapse Footprints");
    assert!(h.query_by_label("Footprint R1  R").is_none());
    click(&mut h, "Expand Footprints");
    h.get_by_label("Footprint R1  R");
    click(&mut h, "Collapse Browser");
    assert!(h.query_by_label("Main").is_none());
    click(&mut h, "Expand Browser");
    h.get_by_label("Main");
    h.state_mut().browser.open = false;
    h.run();
    assert!(h.query_by_label("Collapse Browser").is_none());
    h.get_by_label("Collapse Layers");
    h.state_mut().browser.open = true;
    h.run();
    h.get_by_label("Main");
    assert_eq!(h.state().session.inspect(), before);
}

/// The ribbon can restore a Browser hidden in the other workspace. Both directions share
/// visibility without changing project bytes, revision or history, including compact windows.
#[test]
fn browser_ribbon_toggle_can_restore_visibility_from_either_workspace() {
    for width in [1400.0, 700.0] {
        let mut h = project_harness(width);
        let before = h.state().session.inspect();
        let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        click(&mut h, "View");
        click(&mut h, "Toggle Browser");
        assert!(!h.state().browser.open);
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        assert!(h.query_by_label("Main").is_none());
        click(&mut h, "Toggle Browser");
        assert!(h.state().browser.open);
        h.get_by_label("Main");
        h.get_by_label("Footprint R1  R");
        click(&mut h, "Toggle Browser");
        assert!(!h.state().browser.open);
        h.get_by_label("Collapse Layers");
        click(&mut h, "Choose workspace");
        click(&mut h, "Schematic");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        assert!(h.query_by_label("Main").is_none());
        click(&mut h, "Toggle Browser");
        assert!(h.state().browser.open);
        h.get_by_label("Main");
        assert_eq!(h.state().session.inspect(), before);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
}

/// Selecting an inactive board's object creates its view before its panel layout exists.
/// Centering waits for that layout, so the placement stays clear of Browser and Properties.
#[test]
fn first_footprint_pick_centers_in_the_destination_uncovered_canvas() {
    for width in [1400.0, 700.0] {
        let mut h = project_harness(width);
        click(&mut h, "Expand Board");
        click(&mut h, "Footprint R1  R");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        crate::tests::open_browser_row(&mut h, "Footprint R1  R", "Show in PCB layout");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        let center = h.state().views.active.camera.center();
        assert!((center.x_nm - 127_000_000).abs() < 100);
        assert!((center.y_nm - 25_400_000).abs() < 100, "width={width}, center={center:?}");
        // A consumed request must not undo a later manual camera move.
        let away = pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 };
        h.state_mut().views.active.camera.center_on(away);
        h.run();
        assert_eq!(h.state().views.active.camera.center(), away);
    }
}

/// Deferred Browser actions cannot navigate or select after an edit/reopen or a same-frame
/// modal. They validate the originating view before applying navigation and selection together.
#[test]
fn stale_browser_actions_leave_navigation_and_selection_alone() {
    for interrupted in 0..3 {
        let mut h = project_harness(1400.0);
        let project = h.state().session.project().unwrap();
        let board = project.board_target();
        let footprint = project.board_view(board).unwrap().footprints().next().unwrap();
        let action = Action::Footprint { board, id: footprint.id, at: footprint.position };
        let ctx = h.ctx.clone();
        let source = h.state().views.active.id();
        h.state_mut().queue_view_action(&ctx, source, ViewAction::Browse(action));
        if interrupted == 0 {
            h.state_mut().run("project.rename", &json!({"name":"Changed"}));
        } else if interrupted == 1 {
            let snapshot = h.state().session.snapshot().unwrap();
            h.state_mut().session.open_snapshot(snapshot);
        } else {
            h.state_mut().select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
            h.state_mut().edit_selected(crate::SymbolAction::EditFields);
        }
        let before = h.state().session.snapshot().unwrap();
        h.state_mut().finish_view_actions(&ctx);
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        assert!(h.state().views.active.board.selection.is_empty());
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// A Browser click affects only the focused pane, retaining the opposite pane's document,
/// camera and actual edit selection even when the selected object has a counterpart there.
#[test]
fn browser_navigation_stays_in_the_focused_split_pane() {
    let mut h = project_harness(1400.0);
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    click(&mut h, "PCB canvas 3");
    let first = ViewId(0);
    let other = h.state().views.active.id();
    assert_ne!(first, other);
    let before = h.state().session.snapshot().unwrap();
    let target = h.state().views.view(first).unwrap().target();
    let rect = h.get_by_label("Schematic canvas 1").rect();
    let origin = pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 };
    let at = h.state().views.view(first).unwrap().camera.screen(origin, rect);
    click(&mut h, "Main");
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    crate::tests::open_browser_row(&mut h, "Main", "Open in Schematic");
    assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
    assert_ne!(h.state().views.active.id(), first);
    assert_eq!(h.state().views.view(first).unwrap().target(), target);
    assert_eq!(h.state().views.view(first).unwrap().camera.screen(origin, rect), at);
    assert!(h.state().views.view(first).unwrap().schematic.selection.is_empty());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
