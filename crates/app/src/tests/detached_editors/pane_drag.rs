//! Pointer-driven regression coverage for instance-local pane transfers.

use super::*;

/// Cross the drag threshold on a real root header, without releasing the primary button.
fn root_drag(h: &mut Harness<'_, PcbApp>, index: usize) -> ViewId {
    let id = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[index];
    let path = h.state().view_title(id);
    let pane = h.state().views.hosts[&HostId::ROOT].rects.iter().find(|(view, _)| *view == id).unwrap().1;
    let at = h.get_all_by_label(&path).find(|node| pane.contains(node.rect().center())).unwrap().rect().center();
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.hover_at(at + vec2(20.0, 0.0));
    h.run();
    id
}

/// Begin a child header drag through its independent native input stream.
fn child_drag(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>) -> ViewId {
    focus(h, peer, true);
    let id = h.state().views.hosts[&HostId(1)].focused;
    let at = child_rect(peer, &h.state().view_title(id)).center();
    peer.borrow_mut().events.push(Event::PointerMoved(at));
    h.run();
    peer.borrow_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    peer.borrow_mut().events.push(Event::PointerMoved(at + vec2(20.0, 0.0)));
    h.run();
    id
}

/// Drag-out creates another viewport in this application, retaining view identity and session.
/// The detached single pane keeps a header so it can be dragged back on Wayland as well.
#[test]
fn pane_drag_out_and_back_keeps_the_same_view_and_session() {
    let (mut h, peers) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let id = root_drag(&mut h, 1);
    let camera = h.state().views.view(id).unwrap().camera.saved_state();
    h.drop_at(pos2(1450.0, 450.0));
    h.run();
    assert_eq!(h.state().views.owner(id), Some(HostId(1)));
    assert!(!h.state().views.is_split());
    let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    child_drag(&mut h, &peer);
    let root = h.state().views.hosts[&HostId::ROOT].focused;
    let target_label = format!("Dock in main window · {}", h.state().view_title(root));
    let at = child_rect(&peer, &target_label).center();
    peer.borrow_mut().events.push(Event::PointerMoved(at));
    h.run();
    peer.borrow_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(h.state().views.owner(id), Some(HostId::ROOT));
    assert!(h.state().views.is_split());
    assert_eq!(h.state().views.hosts.len(), 1);
    assert_eq!(h.state().views.view(id).unwrap().camera.saved_state(), camera);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Desktop-coordinate drops resolve the addressed pane rather than always using main focus.
#[test]
fn pane_drag_can_dock_directly_in_another_native_window() {
    let (mut h, peers) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    h.state_mut().split_editors(&ctx, true);
    h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect = Some(Rect::from_min_size(pos2(10.0, 20.0), vec2(1400.0, 900.0)));
    h.run();
    let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    peer.borrow_mut().origin = Some(pos2(1600.0, 20.0));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let moved = root_drag(&mut h, 1);
    // Direct docking outside the source must not advertise a new window instead.
    h.hover_at(pos2(1800.0, 450.0));
    h.run();
    assert_eq!(h.get_by_label("Open in new window").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert!(peer.borrow().nodes.values().any(|node| node.label() == Some("Dock here")));
    // Source receives the release outside its bounds, as with native pointer capture.
    h.drop_at(pos2(1800.0, 450.0));
    h.run();
    assert_eq!(h.state().views.owner(moved), Some(HostId(1)));
    assert!(h.state().views.hosts[&HostId(1)].split());
    assert_eq!(h.state().views.hosts.len(), 2, "dock without opening a third host");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Cancelling, dropping inside the original window and replacing the source cannot move it.
#[test]
fn pane_drag_cancellation_and_stale_input_leave_layouts_intact() {
    for cancel in ["escape", "inside", "stale", "modal"] {
        let (mut h, _) = many_windows();
        let ctx = h.ctx.clone();
        h.state_mut().split_editors(&ctx, true);
        h.run();
        let id = root_drag(&mut h, 1);
        match cancel {
            "escape" => h.key_press(Key::Escape),
            "stale" => h.state_mut().views.suspend(),
            "modal" => h.state_mut().dialog = Dialog::About,
            _ => {}
        }
        h.run();
        h.drop_at(if cancel == "inside" { pos2(800.0, 450.0) } else { pos2(1450.0, 450.0) });
        h.run();
        assert_eq!(h.state().views.owner(id), Some(HostId::ROOT), "{cancel}");
        assert_eq!(h.state().views.hosts.len(), 1, "{cancel}");
    }
}

/// A separate application may allocate identical numeric IDs, but it owns no drag gesture.
/// Pointer events alone, even over its matching panes, cannot import or remove any view.
#[test]
fn pane_drag_authority_never_crosses_application_instances() {
    let (mut first, _) = many_windows();
    let ctx = first.ctx.clone();
    first.state_mut().split_editors(&ctx, true);
    first.run();
    let moved = root_drag(&mut first, 1);
    let mut other = harness();
    new_project(&mut other);
    let snapshot = other.state().session.snapshot().unwrap();
    other.drop_at(pos2(700.0, 400.0));
    other.run();
    assert_eq!(other.state().views.hosts.len(), 1);
    assert!(Arc::ptr_eq(&snapshot, &other.state().session.snapshot().unwrap()));
    assert_eq!(first.state().views.owner(moved), Some(HostId::ROOT));
    first.key_press(Key::Escape);
    first.run();
}

/// GUI and MCP share explicit cross-window docking, rejecting unknown/same-window targets.
#[test]
fn docking_commands_keep_cached_documents_and_validate_destinations() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    h.state_mut().views.open_window(root);
    h.run();
    let source = h.state().views.hosts[&HostId(1)].focused;
    let target = h.state().views.hosts[&HostId(2)].focused;
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision})).unwrap();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let app = h.state_mut();
    let cached = app.views.open_document(source, crate::editor_view::DocumentTarget::Sheet(power), &app.session).unwrap();
    h.run();
    let source_camera = h.state().views.view(source).unwrap().camera.saved_state();
    for invalid in [source.0, 999] {
        assert!(h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":source.0,"beside":invalid})).is_err());
        assert_eq!(h.state().views.owner(source), Some(HostId(1)));
    }
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":source.0,"beside":target.0})).unwrap();
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId(2)));
    assert!(!h.state().views.hosts.contains_key(&HostId(1)));
    assert_eq!(h.state().views.hosts[&HostId(2)].layout.visible_views(), vec![target, cached]);
    assert_eq!(h.state().views.owner(cached), Some(HostId(2)));
    assert_eq!(h.state().views.hosts[&HostId(2)].layout.group(source).unwrap().views(), vec![source, cached]);
    assert_eq!(h.state().views.view(source).unwrap().camera.saved_state(), source_camera);
    assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
}

/// Moving the sole main pane by a cached document ID retains its unpainted Fit/center request
/// and its parked workspace's centering. Docking and detaching leave a replacement with only
/// the displayed transform, whether newly allocated or reused from a collapsed split.
#[test]
fn moving_the_sole_main_pane_preserves_pending_navigation_in_cached_documents() {
    for (operation, fit) in [("dock", true), ("dock", false), ("detach", true), ("detach", false), ("detach_parked", true), ("detach_parked", false)]
    {
        let (mut h, _) = many_windows();
        let ctx = h.ctx.clone();
        let source = h.state().views.active.id();
        let destination = h.state_mut().views.open_window(source).unwrap();
        h.run();
        let target = h.state().views.hosts[&destination].focused;
        let revision = h.state().session.revision();
        h.state_mut().session.apply(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision})).unwrap();
        let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
        let app = h.state_mut();
        let visible = app.views.open_document(source, crate::editor_view::DocumentTarget::Sheet(power), &app.session).unwrap();
        h.run();
        if operation == "detach_parked" {
            h.state_mut().views.open_beside(source).unwrap();
            h.state_mut().split_editors(&ctx, false);
            h.run();
            assert!(!h.state().views.hosts[&HostId::ROOT].parked.is_empty());
        }
        let far = Point { x_nm: 200_000_000, y_nm: 100_000_000 };
        let origin = Point { x_nm: 0, y_nm: 0 };
        let app = h.state_mut();
        app.views.switch_workspace(source, crate::editor_view::Workspace::Board, &app.session);
        let view = app.views.view_mut(source).unwrap();
        view.camera.center_on(far);
        view.camera.request_center(origin);
        app.views.switch_workspace(source, crate::editor_view::Workspace::Schematic, &app.session);
        let view = app.views.view_mut(source).unwrap();
        view.camera.set_zoom(0.05);
        view.camera.center_on(far);
        if fit {
            h.state_mut().run_window_command(&ctx, "view.fit", &json!({"view":source.0})).unwrap();
        } else {
            h.state_mut().views.view_mut(source).unwrap().camera.request_center(origin);
        }
        let camera = h.state().views.view(source).unwrap().camera.saved_state();
        let snapshot = h.state().session.snapshot().unwrap();
        if operation == "dock" {
            h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":source.0,"beside":target.0})).unwrap();
        } else {
            h.state_mut().run_window_command(&ctx, "view.detach", &json!({"view":source.0})).unwrap();
        }
        let destination = h.state().views.owner(source).unwrap();
        let replacement = h.state().views.hosts[&HostId::ROOT].focused;
        assert_ne!(replacement, source);
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), vec![replacement]);
        let expected = if operation == "dock" { vec![target, visible] } else { vec![visible] };
        assert_eq!(h.state().views.hosts[&destination].layout.visible_views(), expected);
        assert_eq!(h.state().views.hosts[&destination].layout.group(source).unwrap().views(), vec![source, visible]);
        assert_eq!(h.state().views.hosts[&destination].focused, visible, "{operation}: moving by a cached ID retains visible focus");
        h.run();
        assert_eq!(h.state().views.view(source).unwrap().camera.saved_state(), camera, "cached navigation waits for its own paint");
        assert_eq!(h.state().views.view(replacement).unwrap().camera.zoom(), 0.05, "the replacement must not inherit pending Fit");
        h.state_mut().run_window_command(&ctx, "view.focus", &json!({"view":source.0})).unwrap();
        h.run();
        let camera = &h.state().views.view(source).unwrap().camera;
        if fit {
            assert!(camera.zoom() > 0.05, "{operation}: deferred Fit must run when the moved document is painted");
        } else {
            assert_eq!(camera.center(), origin);
        }
        let app = h.state_mut();
        app.views.switch_workspace(source, crate::editor_view::Workspace::Board, &app.session);
        h.run();
        assert_eq!(h.state().views.view(source).unwrap().camera.center(), origin, "the parked workspace retains pending centering");
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    }
}

/// A held drag shows stationary destinations before any layout change. Captured motion
/// outside the client highlights the new-window action that release will perform.
#[test]
fn pane_drag_highlights_detach_before_release_without_a_floating_card() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = root_drag(&mut h, 1);
    let before = h.state().session.snapshot().unwrap();
    let initial = h.get_by_label("Open in new window").rect();
    h.hover_at(pos2(1000.0, 400.0));
    h.run();
    let moved = h.get_by_label("Open in new window").rect();
    assert_eq!(moved, initial);
    assert_eq!(h.get_by_label("Open in new window").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grabbing);
    assert_eq!(h.state().views.hosts.len(), 1);
    h.hover_at(pos2(1450.0, 450.0));
    h.run();
    let feedback = h.get_by_label("Open in new window").rect();
    assert_eq!(h.get_by_label("Open in new window").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert!(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0)).contains_rect(feedback));
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    h.drop_at(pos2(1450.0, 450.0));
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId(1)));
    assert!(h.query_by_label("Open in new window").is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Every backend exposes the same explicit targets, and their selected state agrees
/// with release. This avoids requiring an invisible trip outside the native window.
#[test]
fn pane_drag_can_detach_through_a_highlighted_new_window_target() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = root_drag(&mut h, 1);
    let at = h.get_by_label("Open in new window").rect().center();
    h.hover_at(at);
    h.run();
    assert_eq!(h.get_by_label("Open in new window").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    h.drop_at(at);
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId(1)));
}

/// Hovering a same-instance target highlights the future split region and states where the
/// pane will dock. Moving off it clears the preview; Escape removes all drag feedback.
#[test]
fn pane_drag_dock_target_highlights_destination_and_cancellation_clears_feedback() {
    let (mut h, peers) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = root_drag(&mut h, 1);
    let target = h.state().views.hosts[&HostId(1)].focused;
    let label = format!("Dock in window 1 · {}", h.state().view_title(target));
    let at = h.get_by_label(&label).rect().center();
    h.hover_at(at);
    h.run();
    assert_eq!(h.get_by_label(&label).accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let preview = child_rect(&peer, "Dock here");
    let target_pane = h.state().views.hosts[&HostId(1)].rects[0].1;
    let future = pcbcraft_chrome::editor::dock_region(target_pane);
    assert!(future.contains(preview.center()));
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    h.hover_at(pos2(800.0, 600.0));
    h.run();
    assert_eq!(h.get_by_label(&label).accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert!(!peer.borrow().nodes.values().any(|node| node.label() == Some("Dock here")));
    h.key_press(Key::Escape);
    h.run();
    assert!(h.query_by_label(&label).is_none());
    assert!(h.query_by_label("Move to another pane or outside the window").is_none());
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
}

/// Desktop bounds may overlap. Dragging within the source must not silently target a window
/// underneath it; the explicit row remains available for intentionally docking there.
#[test]
fn pane_drag_does_not_dock_into_a_window_underneath_the_source() {
    let (mut h, peers) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    h.state_mut().split_editors(&ctx, true);
    h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect = Some(Rect::from_min_size(pos2(10.0, 20.0), vec2(1400.0, 900.0)));
    h.run();
    peers.borrow()[&editor_windows::viewport(HostId(1))].borrow_mut().origin = Some(pos2(10.0, 20.0));
    h.run();
    let source = root_drag(&mut h, 1);
    h.hover_at(pos2(800.0, 500.0));
    h.run();
    h.drop_at(pos2(800.0, 500.0));
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    assert_eq!(h.state().views.hosts.len(), 2);
}

/// Compact windows retain a visible, selected new-window row after captured motion leaves
/// the client; there is no clipped floating card to obscure the destination.
#[test]
fn pane_drag_destinations_stay_visible_in_compact_windows() {
    for size in [vec2(760.0, 650.0), vec2(480.0, 400.0)] {
        let (mut h, _) = many_windows();
        h.set_size(size);
        let ctx = h.ctx.clone();
        h.state_mut().split_editors(&ctx, true);
        h.run();
        root_drag(&mut h, 1);
        let pane = h.state().views.hosts[&HostId::ROOT].rects[1].1;
        h.hover_at(pos2(size.x + 80.0, pane.top() + 20.0));
        h.run();
        let target = h.get_by_label("Open in new window").rect();
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(target));
        assert_eq!(h.get_by_label("Open in new window").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
        assert!(h.query_by_label("Dragged view preview").is_none());
        h.key_press(Key::Escape);
        h.run();
    }
}

/// A compositor can withdraw hover during capture (for example for a screenshot) and deliver
/// release without a fresh enter/motion. The captured position still selects the indicated row.
#[test]
fn pane_drag_docking_survives_native_hover_loss_before_release() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = root_drag(&mut h, 1);
    let target = h.state().views.hosts[&HostId(1)].focused;
    let at = h.get_by_label(&format!("Dock in window 1 · {}", h.state().view_title(target))).rect().center();
    h.hover_at(at);
    h.run();
    h.input_mut().focused = false;
    h.event(Event::PointerGone);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId(1)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// If native capture disappears without delivering release, a fresh press starts a new
/// interaction; it must never finish the abandoned pane drag when that later click releases.
#[test]
fn pane_drag_abandoned_capture_cannot_commit_on_a_later_click() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = root_drag(&mut h, 1);
    h.input_mut().focused = false;
    h.event(Event::PointerGone);
    h.run();
    h.input_mut().focused = true;
    h.drag_at(pos2(800.0, 500.0));
    h.run();
    assert!(h.query_by_label("Dragged view preview").is_none());
    h.drop_at(pos2(1450.0, 450.0));
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    assert_eq!(h.state().views.hosts.len(), 1);
}

/// Fast native motion can deliver press and movement together. Starting feedback and any
/// extra layout pass must retain that gesture until its own release.
#[test]
fn pane_drag_supports_press_and_movement_in_one_input_frame() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let source = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[1];
    let at = h.get_all_by_label(&h.state().view_title(source)).nth(1).unwrap().rect().center();
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.hover_at(at + vec2(30.0, 0.0));
    h.run();
    h.get_by_label("Open in new window");
    h.drop_at(pos2(1450.0, 450.0));
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId(1)));
}

/// A detached schematic can return beside a board through its visible header button, without
/// dragging to a distant target or changing either workspace, camera or project snapshot.
#[test]
fn detached_schematic_header_dock_button_returns_to_the_main_board_window() {
    let (mut h, peers) = many_windows();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    let app = h.state_mut();
    app.views.switch_workspace(root, crate::editor_view::Workspace::Board, &app.session);
    h.run();
    let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let source = h.state().views.hosts[&HostId(1)].focused;
    let camera = h.state().views.view(source).unwrap().camera.saved_state();
    let before = h.state().session.snapshot().unwrap();
    let dock = child_rect(&peer, "Dock in main window");
    assert!(dock.right() <= 60.0, "Dock stays at the leading edge of the header");
    child_click_at(&mut h, &peer, dock.center());
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    assert_eq!(h.state().views.hosts.len(), 1);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), vec![root, source]);
    assert_eq!(h.state().views.view(root).unwrap().workspace, crate::editor_view::Workspace::Board);
    assert_eq!(h.state().views.view(source).unwrap().workspace, crate::editor_view::Workspace::Schematic);
    assert_eq!(h.state().views.view(source).unwrap().camera.saved_state(), camera);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A Wayland host can extend beyond the monitor without exposing its desktop bounds. The
/// return target must stay beside the visible dragged header, rather than at the far edge of
/// its full client rectangle, and remain fixed while the pointer moves over the main window.
#[test]
fn pane_drag_return_target_stays_at_the_visible_header_of_a_partly_offscreen_host() {
    let (mut h, peers) = many_windows();
    let root = h.state().views.active.id();
    h.state_mut().views.open_window(root);
    let app = h.state_mut();
    app.views.switch_workspace(root, crate::editor_view::Workspace::Board, &app.session);
    h.run();
    let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let source = child_drag(&mut h, &peer);
    let label = format!("Dock in main window · {}", h.state().view_title(root));
    let target = child_rect(&peer, &label);
    assert!(target.left() < 100.0 && target.right() < 500.0, "reachable in the visible left part of the 1100-point host: {target:?}");
    peer.borrow_mut().events.push(Event::PointerMoved(pos2(-140.0, 100.0)));
    h.run();
    assert_eq!(child_rect(&peer, &label), target, "the destination must not chase the drag pointer");
    assert!(peer.borrow().nodes.values().any(|node| node.label() == Some(label.as_str()) && node.toggled() == Some(egui::accesskit::Toggled::False)));
    let before = h.state().session.snapshot().unwrap();
    peer.borrow_mut().events.push(Event::PointerMoved(target.center()));
    h.run();
    assert!(peer.borrow().nodes.values().any(|node| node.label() == Some(label.as_str()) && node.toggled() == Some(egui::accesskit::Toggled::True)));
    peer.borrow_mut().events.push(Event::PointerButton {
        pos: target.center(),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    assert_eq!(h.state().views.hosts.len(), 1);
    assert_eq!(h.state().views.view(root).unwrap().workspace, crate::editor_view::Workspace::Board);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A drop over the other pane exchanges the two positions, retaining their cameras and the
/// divider. The preview names a move and must never imply or create a native tear-off.
/// Both setup and result require a split with the specified divider ratio.
#[test]
fn pane_drag_to_the_left_exchanges_two_existing_panes() {
    for size in [vec2(1400.0, 900.0), vec2(760.0, 650.0)] {
        let (mut h, _) = many_windows();
        h.set_size(size);
        let ctx = h.ctx.clone();
        h.state_mut().split_editors(&ctx, true);
        let crate::editor_layout::Layout::Split { ratio, .. } = &mut h.state_mut().views.hosts.get_mut(&HostId::ROOT).unwrap().layout else {
            panic!("two-pane setup must have a split layout");
        };
        *ratio = 0.4;
        h.run();
        let original = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
        let source = root_drag(&mut h, 1);
        let cameras: Vec<_> = original.iter().map(|id| h.state().views.view(*id).unwrap().camera.saved_state()).collect();
        let before = h.state().session.snapshot().unwrap();
        let target = h.state().views.hosts[&HostId::ROOT].rects[0].1;
        h.hover_at(target.center());
        h.run();
        h.get_by_label("Move pane here");
        assert!(target.contains(h.get_by_label("Move pane here").rect().center()));
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), original);
        h.drop_at(target.center());
        h.run();
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), vec![source, original[0]]);
        assert_eq!(h.state().views.hosts.len(), 1);
        let crate::editor_layout::Layout::Split { ratio, .. } = h.state().views.hosts[&HostId::ROOT].layout else {
            panic!("two-pane layout must remain split after the exchange");
        };
        assert_eq!(ratio, 0.4);
        for (id, camera) in original.iter().zip(cameras) {
            assert_eq!(h.state().views.view(*id).unwrap().camera.saved_state(), camera);
        }
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Moving through a three-pane layout shifts intervening panes instead of swapping only the
/// endpoints. Every split slot/ratio survives and movement works back in the other direction.
#[test]
fn pane_drag_reorders_three_panes_without_changing_the_split_tree() {
    let (mut h, _) = many_windows();
    let root = h.state().views.active.id();
    let middle = h.state_mut().views.open_beside(root).unwrap();
    let last = h.state_mut().views.open_beside(middle).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    for (source_index, target_id, expected) in [(2, root, vec![last, root, middle]), (0, middle, vec![root, middle, last])] {
        root_drag(&mut h, source_index);
        let target = h.state().views.hosts[&HostId::ROOT].rects.iter().find(|(id, _)| *id == target_id).unwrap().1.center();
        h.hover_at(target);
        h.run();
        h.get_by_label("Move pane here");
        h.drop_at(target);
        h.run();
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), expected);
        assert_eq!(h.state().views.hosts.len(), 1);
    }
    assert!(
        matches!(h.state().views.hosts[&HostId::ROOT].layout, crate::editor_layout::Layout::Split { ref second, .. } if matches!(**second, crate::editor_layout::Layout::Split { .. }))
    );
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Reordering moves cached documents together through the shared GUI/MCP handler, validates
/// destinations and survives collapse/reopen without reverting to the earlier pane order.
#[test]
fn pane_reorder_commands_preserve_cached_documents_and_restored_order() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    let source = h.state_mut().views.open_beside(root).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision})).unwrap();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let app = h.state_mut();
    let cached = app.views.open_document(source, crate::editor_view::DocumentTarget::Sheet(power), &app.session).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    for target in [source.0, cached.0, 999] {
        assert!(h.state_mut().run_window_command(&ctx, "view.reorder", &json!({"view":source.0,"target":target})).is_err());
    }
    h.state_mut().run_window_command(&ctx, "view.reorder", &json!({"view":source.0,"target":root.0})).unwrap();
    h.run();
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), vec![cached, root]);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.group(cached).unwrap().views(), vec![source, cached]);
    h.state_mut().split_editors(&ctx, false);
    h.state_mut().split_editors(&ctx, true);
    h.run();
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), vec![cached, root]);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
