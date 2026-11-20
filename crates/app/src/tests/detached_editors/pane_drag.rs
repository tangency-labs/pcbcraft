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

/// Docking from a collapsed main host retains its replacement in the remembered split, so
/// reopening cannot orphan the focused pane and arrangements remain saveable and restorable.
#[test]
fn docking_from_a_collapsed_main_host_keeps_its_replacement_in_saved_splits() {
    for parked_count in [1, 2] {
        let (mut h, _) = many_windows();
        let ctx = h.ctx.clone();
        let source = h.state().views.active.id();
        let destination = h.state_mut().views.open_window(source).unwrap();
        let target = h.state().views.hosts[&destination].focused;
        let mut parked = Vec::new();
        let mut beside = source;
        for _ in 0..parked_count {
            beside = h.state_mut().views.open_beside(beside).unwrap();
            parked.push(beside);
        }
        h.run();
        h.state_mut().run_window_command(&ctx, "view.split", &json!({"view":source.0,"split":false})).unwrap();
        h.run();
        assert_eq!(h.state().views.hosts[&HostId::ROOT].parked, parked);
        let project = h.state().session.snapshot().unwrap();
        h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":source.0,"beside":target.0})).unwrap();
        h.run();
        let replacement = h.state().views.hosts[&HostId::ROOT].focused;
        assert_ne!(replacement, source);
        assert!(h.state().arrangement_snapshot().expect("docking must leave a saveable collapsed arrangement").valid());
        h.state_mut().run_window_command(&ctx, "view.split", &json!({"view":replacement.0,"split":true})).unwrap();
        h.run();
        let expected: Vec<_> = std::iter::once(replacement).chain(parked).collect();
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), expected);
        assert_eq!(h.state().views.owner(replacement), Some(HostId::ROOT));
        assert_eq!(h.state().views.hosts[&HostId::ROOT].focused, replacement);
        assert_eq!(h.state().views.owner(source), Some(destination));
        let saved = h.state().arrangement_snapshot().expect("reopening must leave a saveable arrangement");
        assert!(saved.valid());
        assert!(saved.hosts[0].layout.visible(saved.hosts[0].focused));
        let mut restored = crate::editor_workspace::EditorViews::default();
        restored.restore_state(&saved, &h.state().session).unwrap();
        assert_eq!(restored.hosts[&HostId::ROOT].layout.visible_views().len(), expected.len());
        assert_eq!(restored.owner(restored.hosts[&HostId::ROOT].focused), Some(HostId::ROOT));
        assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
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
    let canvas = h.state().views.hosts[&HostId::ROOT].rects.iter().find(|(id, _)| *id == source).unwrap().1;
    h.hover_at(canvas.center());
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

/// Edge drops choose the indicated half and keep that direction across window resizing.
#[test]
fn pane_edge_drops_split_in_all_four_directions() {
    for (side, caption) in [("top", "Split above"), ("bottom", "Split below"), ("left", "Split left"), ("right", "Split right")] {
        for light in [false, true] {
            let (mut h, _) = many_windows();
            let ctx = h.ctx.clone();
            h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"theme":if light {"light"} else {"dark"}})).unwrap();
            h.state_mut().split_editors(&ctx, true);
            h.run();
            let target = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[0];
            let moved = root_drag(&mut h, 1);
            let rect = h.state().views.hosts[&HostId::ROOT].rects[0].1;
            let host_area = h.state().views.hosts[&HostId::ROOT].rects.iter().map(|(_, rect)| *rect).reduce(Rect::union).unwrap();
            let at = match side {
                "top" => pos2(rect.center().x, rect.top() + 8.0),
                "bottom" => pos2(rect.center().x, rect.bottom() - 8.0),
                "left" => pos2(rect.left() + 8.0, rect.center().y),
                _ => pos2(rect.right() - 8.0, rect.center().y),
            };
            let before = h.state().session.snapshot().unwrap();
            let camera = h.state().views.view(moved).unwrap().camera.saved_state();
            h.hover_at(at);
            h.run();
            let preview = h.get_by_label(caption).rect();
            assert!(host_area.contains(preview.center()));
            if side == "top" {
                assert!(preview.bottom() < rect.center().y);
            }
            if side == "bottom" {
                assert!(preview.top() > rect.center().y);
            }
            h.drop_at(at);
            h.run();
            let placed = h.state().views.hosts[&HostId::ROOT].rects.iter().find(|(id, _)| *id == moved).unwrap().1;
            assert_eq!(preview, placed.shrink(pcbcraft_chrome::Tokens::get().space_4));
            for size in [vec2(1400.0, 900.0), vec2(760.0, 650.0)] {
                h.set_size(size);
                h.run();
                let panes = &h.state().views.hosts[&HostId::ROOT].rects;
                assert_eq!(panes.len(), 2);
                let a = panes.iter().find(|(id, _)| *id == moved).unwrap().1;
                let b = panes.iter().find(|(id, _)| *id == target).unwrap().1;
                match side {
                    "top" => assert!(a.bottom() < b.top()),
                    "bottom" => assert!(a.top() > b.bottom()),
                    "left" => assert!(a.right() < b.left()),
                    _ => assert!(a.left() > b.right()),
                }
            }
            assert_eq!(h.state().views.view(moved).unwrap().camera.saved_state(), camera);
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Directional docking shares command validation, cached-document movement and arrangement storage.
#[test]
fn directional_docking_retains_cached_documents_and_saved_axis() {
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
    for params in [
        json!({"view":source.0,"side":"top"}),
        json!({"view":source.0,"beside":cached.0,"side":"top"}),
        json!({"view":source.0,"beside":root.0,"side":"diagonal"}),
        json!({"view":source.0,"beside":999,"side":"top"}),
    ] {
        assert!(h.state_mut().run_window_command(&ctx, "view.dock", &params).is_err());
    }
    h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":source.0,"beside":root.0,"side":"bottom"})).unwrap();
    h.run();
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.group(cached).unwrap().views(), vec![source, cached]);
    h.state_mut().split_editors(&ctx, false);
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let app = h.state_mut();
    let saved = app.views.saved_state(&app.session).unwrap();
    let bytes = serde_json::to_vec(&saved).unwrap();
    let saved: crate::arrangements::Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(saved.valid());
    assert!(String::from_utf8(bytes).unwrap().contains("\"axis\":\"rows\""));
    app.views.restore_state(&saved, &app.session).unwrap();
    h.run();
    let panes = &h.state().views.hosts[&HostId::ROOT].rects;
    assert!(panes[0].1.bottom() < panes[1].1.top());
}

/// Old arrangements omit orientation, and unknown future directions fail validation.
#[test]
fn split_axis_reads_legacy_arrangements_without_rewriting_them() {
    let old = json!({"kind":"split","first":{"kind":"view","view":0},"second":{"kind":"view","view":1},"ratio":0.4});
    let saved: crate::arrangements::SavedLayout = serde_json::from_value(old.clone()).unwrap();
    let encoded = serde_json::to_string(&saved).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), old);
    let mut bad = old;
    bad["axis"] = json!("diagonal");
    assert!(serde_json::from_value::<crate::arrangements::SavedLayout>(bad).is_err());
}

/// Explicit window targets can select a stacked split without global window coordinates.
#[test]
fn cross_window_split_targets_choose_above_and_below_without_native_positions() {
    for (text, first) in [("above", true), ("below", false)] {
        let (mut h, peers) = many_windows();
        let root = h.state().views.active.id();
        h.state_mut().views.open_window(root);
        h.run();
        let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
        let moved = child_drag(&mut h, &peer);
        let above = child_rect(&peer, "Split above in window 0");
        let below = child_rect(&peer, "Split below in window 0");
        assert!(above.right() <= below.left(), "direction controls must not overlap");
        let target = child_rect(&peer, &format!("Split {text} in window 0")).center();
        peer.borrow_mut().events.push(Event::PointerMoved(target));
        h.run();
        h.get_by_label(if first { "Split above" } else { "Split below" });
        peer.borrow_mut().events.push(Event::PointerButton {
            pos: target,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert_eq!(h.state().views.hosts.len(), 1);
        let panes = &h.state().views.hosts[&HostId::ROOT].rects;
        assert_eq!(panes.len(), 2);
        assert!(panes[0].1.bottom() < panes[1].1.top());
        assert_eq!(panes[usize::from(!first)].0, moved);
    }
}

/// Splitting either half of a schematic/PCB pair subdivides that pane without moving its peer.
#[test]
fn dragging_to_a_panes_own_edge_creates_a_nested_view() {
    for index in [0, 1] {
        for side in ["top", "bottom", "left", "right"] {
            let (mut h, _) = many_windows();
            let ctx = h.ctx.clone();
            h.state_mut().split_editors(&ctx, true);
            let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
            let app = h.state_mut();
            app.views.switch_workspace(ids[1], crate::editor_view::Workspace::Board, &app.session);
            h.run();
            let source = root_drag(&mut h, index);
            let peer = ids[1 - index];
            let rects = &h.state().views.hosts[&HostId::ROOT].rects;
            let area = rects.iter().find(|(id, _)| *id == source).unwrap().1;
            let peer_area = rects.iter().find(|(id, _)| *id == peer).unwrap().1;
            let source_view = h.state().views.view(source).unwrap();
            let target = source_view.target();
            let camera = source_view.camera.saved_state();
            let peer_camera = h.state().views.view(peer).unwrap().camera.saved_state();
            let snapshot = h.state().session.snapshot().unwrap();
            let (at, caption) = match side {
                "top" => (pos2(area.center().x, area.top() + 8.0), "Split above"),
                "bottom" => (pos2(area.center().x, area.bottom() - 8.0), "Split below"),
                "left" => (pos2(area.left() + 8.0, area.center().y), "Split left"),
                _ => (pos2(area.right() - 8.0, area.center().y), "Split right"),
            };
            h.hover_at(at);
            h.run();
            let preview = h.get_by_label(caption).rect();
            assert!(area.contains_rect(preview));
            h.drop_at(at);
            h.run();
            let panes = &h.state().views.hosts[&HostId::ROOT].rects;
            assert_eq!(panes.len(), 3);
            assert_eq!(panes.iter().find(|(id, _)| *id == peer).unwrap().1, peer_area);
            let (new, placed) = panes.iter().find(|(id, _)| !ids.contains(id)).copied().unwrap();
            assert_eq!(preview, placed.shrink(pcbcraft_chrome::Tokens::get().space_4));
            assert_eq!(h.state().views.view(new).unwrap().target(), target);
            assert_eq!(h.state().views.view(new).unwrap().camera.saved_state(), camera);
            assert_eq!(h.state().views.view(source).unwrap().camera.saved_state(), camera);
            assert_eq!(h.state().views.view(peer).unwrap().camera.saved_state(), peer_camera);
            assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Own-edge splitting follows shared cancellation/stale-input policy; the centre stays a no-op.
#[test]
fn own_pane_split_drops_require_a_current_uncancelled_edge() {
    for cancel in ["escape", "stale", "centre"] {
        let (mut h, _) = many_windows();
        let ctx = h.ctx.clone();
        h.state_mut().split_editors(&ctx, true);
        h.run();
        let source = root_drag(&mut h, 1);
        let panes = &h.state().views.hosts[&HostId::ROOT].rects;
        let area = panes.iter().find(|(id, _)| *id == source).unwrap().1;
        let ids = h.state().views.hosts[&HostId::ROOT].layout.views();
        let at = if cancel == "centre" { area.center() } else { pos2(area.center().x, area.bottom() - 8.0) };
        h.hover_at(at);
        h.run();
        match cancel {
            "escape" => h.key_press(Key::Escape),
            "stale" => h.state_mut().views.suspend(),
            _ => {}
        }
        h.drop_at(at);
        h.run();
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), ids, "{cancel}");
    }
}

/// The shared command splits the addressed pane and rejects invalid directions atomically.
#[test]
fn explicit_open_beside_subdivides_only_the_addressed_pane() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let peer_area = h.state().views.hosts[&HostId::ROOT].rects[1].1;
    assert!(h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":ids[0].0,"side":"diagonal"})).is_err());
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), ids);
    h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":ids[0].0,"side":"bottom"})).unwrap();
    h.run();
    let panes = &h.state().views.hosts[&HostId::ROOT].rects;
    assert_eq!(panes.len(), 3);
    assert_eq!(panes[2], (ids[1], peer_area));
    assert_eq!(panes[0].0, ids[0]);
    assert!(panes[0].1.bottom() < panes[1].1.top());
}

/// Explicit up/down drag destinations subdivide each existing pane without finding an outer edge.
#[test]
fn split_up_down_drag_targets_subdivide_nested_panes() {
    for size in [vec2(2560.0, 1600.0), vec2(760.0, 650.0)] {
        for index in [0, 1, 2] {
            for label in ["Split up", "Split down"] {
                let (mut h, _) = many_windows();
                h.set_size(size);
                let ctx = h.ctx.clone();
                h.state_mut().split_editors(&ctx, true);
                let right = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[1];
                h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":right.0,"side":"right"})).unwrap();
                h.run();
                let rects = h.state().views.hosts[&HostId::ROOT].rects.clone();
                let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
                let source = ids[index];
                let area = rects[index].1;
                let at = h.get_all_by_label(&h.state().view_title(source)).find(|node| area.contains(node.rect().center())).unwrap().rect().center();
                h.hover_at(at);
                h.run();
                h.drag_at(at);
                h.run();
                h.hover_at(at + vec2(0.0, 20.0));
                h.run();
                let up = h.get_by_label("Split up").rect();
                let down = h.get_by_label("Split down").rect();
                assert!(up.bottom() <= down.top());
                let target = h.get_by_label(label).rect().center();
                h.hover_at(target);
                h.run();
                let preview = h.get_by_label(if label == "Split up" { "Split above" } else { "Split below" }).rect();
                assert!(area.contains_rect(preview));
                h.drop_at(target);
                h.run();
                let panes = &h.state().views.hosts[&HostId::ROOT].rects;
                assert_eq!(panes.len(), 4);
                for (peer, rect) in &rects {
                    if *peer != source {
                        assert_eq!(panes.iter().find(|(id, _)| id == peer).unwrap().1, *rect);
                    }
                }
                let placed = panes.iter().find(|(id, _)| !ids.contains(id)).unwrap().1;
                assert_eq!(preview, placed.shrink(pcbcraft_chrome::Tokens::get().space_4));
                assert!(h.query_by_label("Split up").is_none());
            }
        }
    }
}

/// Each destination pane offers both directions for moving the existing dragged view into it.
#[test]
fn peer_pane_drag_menu_splits_the_hovered_pane() {
    for (theme, size) in [(ThemeChoice::Dark, vec2(2560.0, 1600.0)), (ThemeChoice::Light, vec2(760.0, 650.0))] {
        for index in 0..3 {
            for (label, caption, above) in [("Split up", "Split above", true), ("Split down", "Split below", false)] {
                let (mut h, _) = many_windows();
                h.state_mut().preferences.theme = theme;
                h.set_size(size);
                let ctx = h.ctx.clone();
                h.state_mut().split_editors(&ctx, true);
                let right = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[1];
                h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":right.0,"side":"right"})).unwrap();
                h.run();
                let (target, area) = h.state().views.hosts[&HostId::ROOT].rects[index];
                let moved = root_drag(&mut h, usize::from(index == 0));
                h.hover_at(area.center());
                h.run();
                for choice in ["Split up", "Split down"] {
                    assert!(area.contains(h.get_by_label(choice).rect().center()), "{choice} should appear under the target pane's tab");
                }
                let row = h.get_by_label(label).rect().center();
                h.hover_at(row);
                h.run();
                let preview = h.get_by_label(caption).rect();
                h.drop_at(row);
                h.run();
                let panes = &h.state().views.hosts[&HostId::ROOT].rects;
                assert_eq!(panes.len(), 3, "the original pane is moved");
                let moved_rect = panes.iter().find(|(id, _)| *id == moved).unwrap().1;
                let target_rect = panes.iter().find(|(id, _)| *id == target).unwrap().1;
                assert_eq!(preview, moved_rect.shrink(pcbcraft_chrome::Tokens::get().space_4));
                let (first, second) = if above { (moved_rect, target_rect) } else { (target_rect, moved_rect) };
                assert!(first.bottom() < second.top());
                assert_eq!(first.x_range(), second.x_range());
            }
        }
    }
}

/// A narrow boundary pane keeps its drag menu when the window fits the rows over its neighbour.
#[test]
fn overhanging_split_row_still_addresses_its_pane() {
    let (mut h, _) = many_windows();
    h.set_size(vec2(760.0, 650.0));
    let ctx = h.ctx.clone();
    for _ in 0..4 {
        let target = h.state().views.hosts[&HostId::ROOT].layout.visible_views().last().copied().unwrap();
        h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":target.0,"side":"right"})).unwrap();
    }
    h.run();
    let (target, bounds) = *h.state().views.hosts[&HostId::ROOT].rects.last().unwrap();
    let moved = root_drag(&mut h, 0);
    h.hover_at(bounds.center());
    h.run();
    let row = h.get_by_label("Split down").rect().center();
    assert!(!bounds.contains(row), "test must exercise a row outside its pane");
    h.hover_at(row);
    h.run();
    let preview = h.get_by_label("Split below").rect();
    h.drop_at(row);
    h.run();
    let panes = &h.state().views.hosts[&HostId::ROOT].rects;
    let to = panes.iter().find(|(id, _)| *id == target).unwrap().1;
    let placed = panes.iter().find(|(id, _)| *id == moved).unwrap().1;
    assert_eq!(to.x_range(), placed.x_range());
    assert!(to.bottom() < placed.top());
    assert_eq!(preview, placed.shrink(pcbcraft_chrome::Tokens::get().space_4));
}

/// A tab's own menu can split an unfocused pane directly, with the same geometry as a drop.
#[test]
fn pane_tab_menu_splits_its_own_view_up_or_down() {
    for (theme, label, above) in [(ThemeChoice::Dark, "Split up", true), (ThemeChoice::Light, "Split down", false)] {
        let (mut h, _) = many_windows();
        h.state_mut().preferences.theme = theme;
        let ctx = h.ctx.clone();
        h.state_mut().split_editors(&ctx, true);
        h.run();
        let panes = h.state().views.hosts[&HostId::ROOT].rects.clone();
        let (source, area) = panes.iter().copied().find(|(id, _)| *id != h.state().views.active.id()).unwrap();
        let title = h.state().view_title(source);
        h.get_all_by_label(&title).find(|node| area.contains(node.rect().center())).unwrap().click_secondary();
        h.run();
        let up = h.get_by_label("Split up").rect();
        let down = h.get_by_label("Split down").rect();
        assert!(up.bottom() <= down.top());
        assert_eq!(up.x_range(), down.x_range());
        click(&mut h, label);
        let after = &h.state().views.hosts[&HostId::ROOT].rects;
        assert_eq!(after.len(), 3);
        let next = after.iter().find(|(id, _)| *id == source).unwrap().1;
        let (_, added) = *after.iter().find(|(id, _)| !panes.iter().any(|(previous, _)| previous == id)).unwrap();
        let (first, second) = if above { (added, next) } else { (next, added) };
        assert!(first.bottom() < second.top());
        assert_eq!(first.x_range(), area.x_range());
        assert_eq!(second.x_range(), area.x_range());
        assert_eq!((first.top(), second.bottom()), (area.top(), area.bottom()));
        assert!(after.contains(panes.iter().find(|(id, _)| *id != source).unwrap()));
        assert!(h.query_by_label("Split up").is_none());
    }
}

/// A drag moves the original schematic and collapses the vacated slot without leaving a copy.
#[test]
fn pane_drop_moves_original_view_and_collapses_its_old_slot() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let app = h.state_mut();
    app.views.switch_workspace(ids[0], crate::editor_view::Workspace::Board, &app.session);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let source_camera = h.state().views.view(ids[1]).unwrap().camera.saved_state();
    let area = h.state().views.hosts[&HostId::ROOT].rects.iter().map(|(_, rect)| *rect).reduce(Rect::union).unwrap();
    let destination = h.state().views.hosts[&HostId::ROOT].rects[0].1;
    root_drag(&mut h, 1);
    h.hover_at(destination.center());
    h.run();
    let at = h.get_by_label("Split down").rect().center();
    h.hover_at(at);
    h.run();
    let preview = h.get_by_label("Split below").rect();
    h.drop_at(at);
    h.run();
    let panes = &h.state().views.hosts[&HostId::ROOT].rects;
    assert_eq!(panes.len(), 2);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), ids);
    let top = panes.iter().find(|(id, _)| *id == ids[0]).unwrap().1;
    let bottom = panes.iter().find(|(id, _)| *id == ids[1]).unwrap().1;
    assert!(top.bottom() < bottom.top());
    assert_eq!(top.x_range(), area.x_range());
    assert_eq!(bottom.x_range(), area.x_range());
    assert_eq!(preview, bottom.shrink(pcbcraft_chrome::Tokens::get().space_4));
    assert_eq!(h.state().views.view(ids[1]).unwrap().camera.saved_state(), source_camera);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Full-window targets move the existing pane around the entire remaining split tree.
#[test]
fn window_split_targets_span_the_full_host_and_retain_view_identity() {
    for third in [false, true] {
        for (label, caption, above) in [("Split window up", "Split window above", true), ("Split window down", "Split window below", false)] {
            let (mut h, _) = many_windows();
            let ctx = h.ctx.clone();
            h.state_mut().split_editors(&ctx, true);
            let first = h.state().views.hosts[&HostId::ROOT].layout.visible_views()[0];
            if third {
                h.state_mut().views.open_beside(first);
            }
            h.run();
            let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
            let rects = h.state().views.hosts[&HostId::ROOT].rects.clone();
            let area = rects.iter().map(|(_, rect)| *rect).reduce(Rect::union).unwrap();
            let source = root_drag(&mut h, ids.len() - 1);
            h.hover_at(rects[0].1.center());
            h.run();
            let at = h.get_by_label(label).rect().center();
            h.hover_at(at);
            h.run();
            let preview = h.get_by_label(caption).rect();
            assert_eq!(preview.x_range(), area.shrink(pcbcraft_chrome::Tokens::get().space_4).x_range());
            h.drop_at(at);
            h.run();
            let panes = &h.state().views.hosts[&HostId::ROOT].rects;
            assert_eq!(panes.len(), ids.len());
            let moved = panes.iter().find(|(id, _)| *id == source).unwrap().1;
            assert_eq!(preview, moved.shrink(pcbcraft_chrome::Tokens::get().space_4));
            assert_eq!(moved.x_range(), area.x_range());
            for (id, rect) in panes {
                if *id != source {
                    if above {
                        assert!(moved.bottom() < rect.top());
                    } else {
                        assert!(rect.bottom() < moved.top());
                    }
                }
            }
        }
    }
}

/// Shared commands expose destination copies and full-window moves without changing project history.
#[test]
fn pane_and_window_scope_commands_validate_before_changing_layout() {
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let before = h.state().session.snapshot().unwrap();
    for params in [
        json!({"view":ids[1].0,"beside":ids[0].0,"scope":"window"}),
        json!({"view":ids[1].0,"side":"top","scope":"window"}),
        json!({"view":ids[1].0,"beside":999,"side":"top","scope":"window"}),
    ] {
        assert!(h.state_mut().run_window_command(&ctx, "view.dock", &params).is_err());
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views(), ids);
    }
    assert!(h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":ids[1].0,"beside":999,"side":"bottom"})).is_err());
    h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":ids[1].0,"beside":ids[0].0,"side":"bottom"})).unwrap();
    h.run();
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.visible_views().len(), 3);
    h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":ids[1].0,"beside":ids[0].0,"side":"top","scope":"window"})).unwrap();
    h.run();
    let panes = &h.state().views.hosts[&HostId::ROOT].rects;
    let top = panes.iter().find(|(id, _)| *id == ids[1]).unwrap().1;
    assert!(panes.iter().filter(|(id, _)| *id != ids[1]).all(|(_, rect)| top.bottom() < rect.top()));
    let app = h.state_mut();
    let saved = app.views.saved_state(&app.session).unwrap();
    assert!(saved.valid());
    app.views.restore_state(&saved, &app.session).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
}

/// A geometric drop from a detached window moves its only pane and removes the empty host.
#[test]
fn detached_pane_edge_drop_moves_instead_of_leaving_the_old_window() {
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
    let original = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let area = h.state().views.hosts[&HostId::ROOT].rects[0].1;
    let source = child_drag(&mut h, &peer);
    let target = pos2(area.center().x + 10.0 - 1600.0, area.top() + 8.0);
    peer.borrow_mut().events.push(Event::PointerMoved(target));
    h.run();
    peer.borrow_mut().events.push(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(h.state().views.hosts.len(), 1);
    assert_eq!(h.state().views.owner(source), Some(HostId::ROOT));
    let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    assert_eq!(ids.len(), original.len() + 1);
    assert_eq!(ids.iter().filter(|id| **id == source).count(), 1);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A single-pane detached host offers working local splits without no-op window targets.
#[test]
fn window_split_rows_require_multiple_visible_panes() {
    for multiple in [false, true] {
        let (mut h, peers) = many_windows();
        let root = h.state().views.active.id();
        h.state_mut().views.open_window(root);
        let child = h.state().views.hosts[&HostId(1)].focused;
        if multiple {
            h.state_mut().views.open_beside(child);
        }
        h.run();
        let peer = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
        child_drag(&mut h, &peer);
        child_rect(&peer, "Split up");
        child_rect(&peer, "Split down");
        for label in ["Split window up", "Split window down"] {
            assert_eq!(peer.borrow().nodes.values().any(|node| node.label() == Some(label)), multiple);
        }
        if !multiple {
            for label in ["Split window above", "Split window below"] {
                assert!(!peer.borrow().nodes.values().any(|node| node.label() == Some(label)));
            }
            let at = child_rect(&peer, "Split down").center();
            peer.borrow_mut().events.push(Event::PointerMoved(at));
            h.run();
            peer.borrow_mut().events.push(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
            h.run();
            assert_eq!(h.state().views.hosts[&HostId(1)].layout.visible_views().len(), 2);
        } else {
            let at = child_rect(&peer, "Split window down").center();
            peer.borrow_mut().events.push(Event::PointerMoved(at));
            h.run();
            assert!(peer.borrow().nodes.values().any(|node| node.label() == Some("Split window below")));
        }
    }
}

/// A centre drop into a very narrow column reorders the existing pane instead of splitting it.
#[test]
fn narrow_pane_retains_a_centre_reorder_drop() {
    let (mut h, _) = many_windows();
    h.set_size(vec2(760.0, 650.0));
    let ctx = h.ctx.clone();
    for _ in 0..5 {
        let target = *h.state().views.hosts[&HostId::ROOT].layout.visible_views().last().unwrap();
        h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":target.0,"side":"right"})).unwrap();
    }
    h.run();
    let ids = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let area = h.state().views.hosts[&HostId::ROOT].rects.last().unwrap().1;
    assert!(area.width() < 40.0);
    let before = h.state().session.snapshot().unwrap();
    let moved = root_drag(&mut h, 0);
    let at = pos2(area.center().x, area.bottom() - 40.0);
    h.hover_at(at);
    h.run();
    assert!(area.contains(h.get_by_label("Move pane here").rect().center()));
    h.drop_at(at);
    h.run();
    let after = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    assert_eq!(after.len(), ids.len());
    assert_eq!(after.last(), Some(&moved));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
