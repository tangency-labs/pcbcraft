//! Mixed workspace interactions use the real picker, canvas and shared session.
use super::*;
use crate::{editor_view::Workspace, preferences::SelectionFollow};
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};

/// Create a resolved starter placement through the same engine commands as MCP. Panels are
/// closed so point clicks exercise the canvas instead of an overlaid card.
fn board_harness<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    resistor_with_board_at_origin(&mut h);
    let app = h.state_mut();
    app.browser.open = false;
    app.run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":2}),
    );
    h.run();
    h
}

/// Workspace switching restores independent cameras, keeps input invalid after switching back,
/// and uses the parked source selection for highlighting without touching authored data.
#[test]
fn switching_workspaces_restores_navigation_and_rejects_stale_schematic_input() {
    let mut h = board_harness();
    click_canvas(&mut h);
    let old = h.state().views.active.input(&h.state().session).unwrap();
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let origin = Point { x_nm: 0, y_nm: 0 };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 400.));
    let initial = h.state().views.active.camera.screen(origin, rect);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    assert_eq!(
        h.state().views.board_highlight_for(h.state().views.active.id(), &h.state().session),
        std::collections::BTreeSet::from(["footprint-3".into()])
    );
    assert!(h.state().views.active.board.selection.is_empty());
    h.state_mut().views.active.camera.center_on(Point { x_nm: 50_000_000, y_nm: 10_000_000 });
    let board_camera = h.state().views.active.camera.screen(origin, rect);
    for key in [Key::Delete, Key::A, Key::W, Key::R, Key::E] {
        h.key_press(key);
        h.run();
    }
    assert_eq!(h.state().session.inspect(), before);
    click(&mut h, "Choose workspace");
    click(&mut h, "Schematic");
    assert_eq!(h.state().views.active.camera.screen(origin, rect), initial);
    assert!(!h.state().views.active.accepts(old, &h.state().session));
    h.state_mut().view_click(old, Click::Edit(SymbolAction::Delete, "symbol-2".into()));
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    assert_eq!(h.state().views.active.camera.screen(origin, rect), board_camera);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(h.state().session.inspect(), before);
}

/// Close PCB zoom holds the point under the mouse, survives arrangement restoration in either
/// workspace, and works through the addressed window command. Fit returns to a board overview.
#[test]
fn pcb_zoom_reaches_copper_detail_and_round_trips_in_arrangements() {
    let mut h = board_harness();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let rect = h.get_by_label("PCB canvas").rect();
    let pointer = rect.center() + egui::vec2(31.0, -27.0);
    let anchor = h.state().views.active.camera.unsnapped(pointer, rect);
    h.hover_at(pointer);
    h.run();
    for _ in 0..8 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 8.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
    assert_eq!(h.state().views.active.camera.zoom(), 64.0);
    assert!(h.state().views.active.camera.screen(anchor, rect).distance(pointer) < 0.01);

    let restore = |view: &crate::editor_view::EditorView, session: &pcbcraft_engine::Session| {
        let saved = view.saved_state(session).unwrap();
        let decoded: crate::editor_view::ViewState = serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert!(decoded.valid());
        crate::editor_view::EditorView::from_state(ViewId(900), &decoded, session)
    };
    let restored = restore(&h.state().views.active, &h.state().session);
    assert_eq!(restored.camera.zoom(), 64.0);
    assert!(restored.camera.screen(anchor, rect).distance(pointer) < 0.01);

    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let zoom = json!({"view":view,"zoom_percent":3200});
    h.state_mut().run_window_command(&ctx, "view.zoom", &zoom).unwrap();
    assert_eq!(h.state().views.active.camera.zoom(), 32.0);
    let mut parked = restore(&h.state().views.active, &h.state().session);
    parked.switch(Workspace::Schematic, &h.state().session);
    let mut restored = restore(&parked, &h.state().session);
    restored.switch(Workspace::Board, &h.state().session);
    assert_eq!(restored.camera.zoom(), 32.0);

    click(&mut h, "Choose workspace");
    click(&mut h, "Schematic");
    let view = h.state().views.active.id().0;
    assert!(h.state_mut().run_window_command(&ctx, "view.zoom", &json!({"view":view,"zoom_percent":3200})).is_err());

    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    assert!(h.state().views.active.camera.zoom() <= 8.0);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Canvas grabs move only the rendered reference and commit once, with cancellation and undo.
#[test]
fn pcb_reference_can_move_with_m_or_primary_drag() {
    for rotation in [0, 180] {
        fn read<'a>(h: &'a Harness<'_, PcbApp>) -> pcbcraft_engine::board::view::FootprintView<'a> {
            h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap()
        }
        let mut h = harness();
        h.set_size(egui::vec2(1400.0, 900.0));
        resistor_with_board_at_origin(&mut h);
        h.state_mut().browser.open = false;
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::PLACE_FOOTPRINT,
            &json!({"symbol":"symbol-2","definition":"resistor-0402","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":revision}),
        );
        h.run();
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        let revision = h.state().session.revision();
        let id = read(&h).id.clone();
        h.state_mut().run(
            pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":0,"y_nm":0,"rotation":rotation,"expected_revision":revision}),
        );
        h.run();
        let snapshot = h.state().session.snapshot().unwrap();
        let footprint = read(&h);
        let origin = footprint.position;
        let pads = footprint.pads.clone();
        let point = footprint.graphics.iter().find(|g| g.reference).unwrap().points[0];
        let from = board_point(&h, point);
        let to = from + egui::vec2(70.0, -55.0);
        h.hover_at(from);
        h.run();
        h.key_press(Key::M);
        h.run();
        assert!(h.state().views.active.board.reference_drag.is_some());
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.active.board.reference_drag.is_none());
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));

        h.hover_at(from);
        h.run();
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        assert!(h.state().views.active.board.reference_drag.is_some(), "mouse-down must capture the reference before the drag threshold");
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run();
        assert!(h.state().views.active.board.reference_drag.is_none());
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()), "a plain click makes no edit");

        for (dragging, quick) in [(false, false), (true, false), (true, true)] {
            let revision = h.state().session.revision();
            h.hover_at(from);
            h.run();
            if dragging {
                h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                h.step();
                assert!(h.state().views.active.board.reference_drag.is_some());
                if !quick {
                    h.event(egui::Event::PointerMoved(from + egui::vec2(1.0, 0.0)));
                    h.run();
                    h.event(egui::Event::PointerMoved(from + egui::vec2(10.0, 0.0)));
                    h.run();
                }
                h.event(egui::Event::PointerMoved(to));
                h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                h.run();
            } else {
                h.key_press(Key::M);
                h.run();
                click_canvas_at(&mut h, to);
            }
            let moved = read(&h);
            assert_eq!(h.state().session.revision(), revision + 1);
            assert_eq!(moved.position, origin);
            assert_eq!(moved.pads, pads);
            let moved_point = moved.graphics.iter().find(|g| g.reference).unwrap().points[0];
            assert!(board_point(&h, moved_point).distance(to) < 0.01);
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.run();
            assert_eq!(read(&h).reference_offset, pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET);
        }
    }
}

/// Opening the board beside a schematic retains both targets. Selection paints actual counterpart
/// ink in each direction; receiver edit selections and camera transforms remain independent.
#[test]
fn mixed_panes_highlight_both_ways_without_granting_schematic_edit_authority() {
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    h.get_by_label("Schematic canvas 1");
    h.get_by_label("PCB canvas 3");
    assert_eq!(h.state().views.view(ViewId(0)).unwrap().workspace, Workspace::Schematic);
    let before = h.state().session.inspect();
    let at = h.get_by_label("Schematic canvas 1").rect().center();
    click_canvas_at(&mut h, at);
    assert!(h.state().views.board_highlight_for(ViewId(2), &h.state().session).contains("footprint-3"));
    let board_rect = h.get_by_label("PCB canvas 3").rect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && board_rect.contains(text.pos) && text.fallback_color==pcbcraft_chrome::board::copper_emphasis(Tokens::get().board_ink, Some(true)))));
    h.key_press(Key::Escape);
    h.run();
    click_canvas_at(&mut h, board_rect.center());
    assert_eq!(h.state().views.active.id(), ViewId(2));
    assert!(h.state().views.active.board.selection.contains("footprint-3"));
    assert_eq!(h.state().views.highlight_for(ViewId(0), &h.state().session).single_symbol(), Some("symbol-2"));
    assert!(h.state().views.view(ViewId(0)).unwrap().schematic.selection.is_empty());
    let schematic_rect = h.get_by_label("Schematic canvas 1").rect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && schematic_rect.contains(text.pos) && text.fallback_color==Tokens::get().schematic_highlight)));
    h.key_press(Key::Delete);
    h.run();
    let deleted = h.state().session.inspect();
    assert_eq!(deleted["project"]["schematic"], before["project"]["schematic"]);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().session.inspect()["project"], before["project"]);
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert!(h.state().views.active.board.selection.is_empty());
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
}

/// Following a schematic selection pans using board coordinates and preserves zoom. The default
/// leaves the board camera alone, and a consumed reveal does not undo subsequent manual navigation.
#[test]
fn mixed_follow_uses_destination_geometry_and_keeps_zoom() {
    for policy in [SelectionFollow::Off, SelectionFollow::Both, SelectionFollow::SecondaryFollowsPrimary, SelectionFollow::PrimaryFollowsSecondary] {
        let mut h = board_harness();
        click(&mut h, "Choose workspace");
        click(&mut h, "Open PCB layout beside current view");
        h.state_mut().preferences.selection_follow = policy;
        h.state_mut().views.view_mut(ViewId(2)).unwrap().camera.center_on(Point { x_nm: 1_000_000_000, y_nm: 0 });
        h.run();
        let rect = h.get_by_label("PCB canvas 3").rect();
        let origin = Point { x_nm: 0, y_nm: 0 };
        let before = h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect);
        let zoom = h.state().views.view(ViewId(2)).unwrap().camera.zoom();
        let at = h.get_by_label("Schematic canvas 1").rect().center();
        click_canvas_at(&mut h, at);
        let after = h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect);
        if matches!(policy, SelectionFollow::Both | SelectionFollow::SecondaryFollowsPrimary) {
            assert_ne!(after, before);
        } else {
            assert_eq!(after, before);
        }
        assert_eq!(h.state().views.view(ViewId(2)).unwrap().camera.zoom(), zoom);
        h.state_mut().views.view_mut(ViewId(2)).unwrap().camera.center_on(Point { x_nm: 1_000_000_000, y_nm: 0 });
        h.run();
        assert_eq!(h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect), before);
    }
}

/// The FigCraft panel arrangement exposes real placement data. Visibility survives a layer
/// rename by ID, duplicates inherit camera/layers, and subsequent changes stay view-local.
#[test]
fn board_browser_properties_and_layers_share_stable_addresses() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_with_board_at_origin(&mut h);
    let app = h.state_mut();
    app.run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":2}));
    app.run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":20_000_000,"y_nm":0,"expected_revision":3}),
    );
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.inspect();
    h.state_mut().browser.open = true;
    h.run();
    click(&mut h, "Footprint R1  R");
    h.get_by_label("Reference");
    h.get_by_label("generic-through-hole-2");
    let target = h.state().views.active.board.target.unwrap();
    let top = h
        .state()
        .session
        .project()
        .unwrap()
        .board_view(target)
        .unwrap()
        .setup()
        .unwrap()
        .layers()
        .iter()
        .find(|layer| layer.name == "Top")
        .unwrap()
        .id
        .to_string();
    click(&mut h, "Show layer Top");
    assert!(h.state().views.active.board.hidden_layers.contains(&top));
    assert_eq!(h.state().session.inspect(), before);
    let mut setup = before["project"]["board"]["setup"].clone();
    for layer in setup["layers"].as_array_mut().unwrap() {
        if layer["id"] == top {
            layer["name"] = json!("Signal top");
        }
    }
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"board":target.board,"expected_revision":revision}));
    h.run();
    assert_eq!(h.get_by_label("Show layer Signal top").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    let original_id = h.state().views.active.id();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let duplicate_id = h.state().views.hosts[&h.state().views.main].layout.visible_views()[1];
    let original = h.state().views.view(original_id).unwrap();
    let duplicate = h.state().views.view(duplicate_id).unwrap();
    let point = Point { x_nm: 0, y_nm: 0 };
    assert_eq!(original.camera.screen(point, egui::Rect::ZERO), duplicate.camera.screen(point, egui::Rect::ZERO));
    assert_eq!(original.board.target, duplicate.board.target);
    assert_eq!(original.board.hidden_layers, duplicate.board.hidden_layers);
    assert!(duplicate.board.selection.is_empty());
    click(&mut h, "Show layer Signal top");
    assert!(!h.state().views.active.board.hidden_layers.contains(&top));
    assert!(h.state().views.view(duplicate_id).unwrap().board.hidden_layers.contains(&top));
    click(&mut h, "Collapse Layers");
    assert!(h.query_by_label("Show layer Signal top").is_none());
    click(&mut h, "Expand Layers");
    h.get_by_label("Show layer Signal top");
}

/// PCB uses its own near-black canvas and readable ink with either chrome theme; switching
/// back restores schematic controls. Neither palette nor grid opacity changes document state.
#[test]
fn board_canvas_and_ribbon_remain_distinct_from_schematic() {
    for theme in [crate::preferences::ThemeChoice::Dark, crate::preferences::ThemeChoice::Light] {
        let mut h = board_harness();
        h.state_mut().preferences.theme = theme;
        h.run();
        let before = h.state().session.inspect();
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        h.get_by_label("Layout");
        assert!(h.query_by_label("Capture").is_none());
        let rect = h.get_by_label("PCB canvas").rect();
        assert!(
            h.output()
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape,egui::Shape::Rect(fill) if fill.rect==rect && fill.fill==Tokens::get().board_canvas))
        );
        assert!(h.output().shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && rect.contains(text.pos) && text.fallback_color==Tokens::get().board_ink)));
        h.state_mut().preferences.grid_opacity_percent = 0;
        h.run();
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Choose workspace");
        click(&mut h, "Schematic");
        h.get_by_label("Capture");
        assert!(h.query_by_label("Layout").is_none());
    }
}

/// Keyboard navigation reaches available workspaces and placement actions without leaking keys
/// into the canvas. Planned workspaces are skipped when moving between rows.
#[test]
fn workspace_keyboard_navigation_reaches_board_and_placement_actions() {
    let mut h = board_harness();
    let before = h.state().session.inspect();
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert_eq!(h.state().library_editors.active_kind(), Some(pcbcraft_engine::authoring::Kind::Symbol));
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().library_editors.active_kind(), Some(pcbcraft_engine::authoring::Kind::Footprint));
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().library_editors.active.is_none());
    assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
    click(&mut h, "Choose workspace");
    let card = h.get_by_label("PCB layout").rect();
    for label in ["Open PCB layout beside current view", "Open PCB layout in new window"] {
        let icon = h.get_by_label(label).rect();
        assert!(icon.top() >= card.top() && icon.bottom() <= card.bottom());
        assert!(icon.left() >= card.right());
    }
    assert!(h.query_by_label("Open beside").is_none());
    assert!(h.query_by_label("New window").is_none());
    h.get_by_label("Open PCB layout beside current view").focus();
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.get_by_label("Schematic canvas 1");
    let board = h.state().views.hosts[&h.state().views.main].layout.visible_views()[1];
    h.get_by_label(&format!("PCB canvas {}", board.0 + 1));
    assert_eq!(h.state().session.inspect(), before);
}

/// Either mixed pane closes through its own tab button with either workspace's panels visible,
/// including stacked panes in narrow windows. The survivor retains its camera and session,
/// and returning to one view removes the extra pane header.
#[test]
fn mixed_pane_close_buttons_preserve_the_surviving_view() {
    for size in [egui::vec2(1400.0, 900.0), egui::vec2(760.0, 600.0)] {
        for focused in [ViewId(0), ViewId(2)] {
            for closed in [ViewId(0), ViewId(2)] {
                let mut h = board_harness();
                h.state_mut().browser.open = true;
                h.set_size(size);
                click(&mut h, "Choose workspace");
                click(&mut h, "Open PCB layout beside current view");
                let path = h.state().views.view(focused).unwrap().sheet_path(&h.state().session).unwrap();
                click(&mut h, &path);
                assert_eq!(h.state().views.active.id(), focused);
                let remaining = if closed == ViewId(0) { ViewId(2) } else { ViewId(0) };
                let origin = Point { x_nm: 0, y_nm: 0 };
                h.state_mut().views.view_mut(remaining).unwrap().camera.center_on(Point { x_nm: 10_000_000, y_nm: 5_000_000 });
                h.run();
                let camera = h.state().views.view(remaining).unwrap().camera.screen(origin, egui::Rect::ZERO);
                let before = h.state().session.inspect();
                let path = h.state().views.view(closed).unwrap().sheet_path(&h.state().session).unwrap();
                click(&mut h, &format!("Close view {path}"));
                assert!(h.state().views.view(closed).is_none(), "size={size:?} focused={focused:?} closed={closed:?}");
                assert_eq!(h.state().views.active.id(), remaining);
                assert!(!h.state().views.is_split());
                assert_eq!(h.state().views.active.camera.screen(origin, egui::Rect::ZERO), camera);
                assert_eq!(h.state().session.inspect(), before);
                let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
                assert!(h.query_by_label(&format!("Close view {path}")).is_none());
            }
        }
    }
}

/// PCB grid preferences change only board drawing, survive serialization, and preserve snapping
/// and project state. Off retains the previous style, Cancel restores visibility as well,
/// and page defaults restore the legacy Off style.
#[test]
fn pcb_grid_preferences_are_shared_display_settings() {
    use pcbcraft_chrome::grid::Style;
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let board_rect = h.get_by_label("PCB canvas 3").rect();
    click_canvas_at(&mut h, board_rect.center());
    let id = h.state().views.active.id();
    h.state_mut().views.view_mut(id).unwrap().camera.set_snap_nm(10 * pcbcraft_chrome::grid::NM_PER_MIL);
    h.run();
    let camera = h.state().views.view(id).unwrap().camera.saved_state();
    let snap = h.state().views.view(id).unwrap().camera.snap(board_rect.center() + egui::vec2(13.0, 9.0), board_rect);
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let marks = |h: &Harness<'_, PcbApp>, color| {
        let dots = h.output().shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Circle(dot) if dot.fill == color)).count();
        let lines = h.output().shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::LineSegment {stroke, ..} if stroke.color == color)).count();
        (dots, lines)
    };
    let schematic_marks = marks(&h, Tokens::get().canvas_grid);
    assert!(schematic_marks.0 > 0);
    assert_eq!(h.state().preferences.board_grid_style, Style::Off);
    assert_eq!(marks(&h, Tokens::get().board_grid), (0, 0));
    h.get_by_label("Grid 10 mil");

    for (label, style) in [("Dots", Style::Dots), ("Lines", Style::Lines), ("Off", Style::Off)] {
        click(&mut h, "Preferences");
        h.get_by_label("PCB grid");
        click(&mut h, label);
        click(&mut h, "OK");
        let remembered = if style == Style::Off { Style::Lines } else { style };
        assert_eq!(h.state().preferences.board_grid_style, remembered);
        assert_eq!(h.state().preferences.grid_visible(true), style != Style::Off);
        assert_eq!(crate::preferences::Preferences::parse(&h.state().preferences.to_text()), h.state().preferences);
        let (dots, lines) = marks(&h, Tokens::get().board_grid);
        match style {
            Style::Dots => {
                assert!(dots > 0);
                assert_eq!(lines, 0);
                h.get_by_label("Grid 10 mil · dots 50 mil");
            }
            Style::Lines => {
                assert_eq!(dots, 0);
                assert!(lines > 0);
                h.get_by_label("Grid 10 mil · lines 50 mil");
            }
            Style::Off => {
                assert_eq!((dots, lines), (0, 0));
                h.get_by_label("Grid 10 mil");
            }
        }
        assert_eq!(marks(&h, Tokens::get().canvas_grid), schematic_marks);
        assert_eq!(h.state().views.view(id).unwrap().camera.saved_state(), camera);
        assert_eq!(h.state().views.view(id).unwrap().camera.snap(board_rect.center() + egui::vec2(13.0, 9.0), board_rect), snap);
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
    click(&mut h, "Preferences");
    click(&mut h, "Dots");
    click(&mut h, "Cancel");
    assert_eq!(h.state().preferences.board_grid_style, Style::Lines);
    assert!(!h.state().preferences.grid_visible(true));
    click(&mut h, "Preferences");
    click(&mut h, "Lines");
    click(&mut h, "Restore page defaults");
    assert_eq!(h.state().preferences.board_grid_style, Style::Off);
    click(&mut h, "OK");
}

/// Choosing a visible PCB style in Preferences restores a ribbon-hidden grid, including
/// legacy zero opacity. Cancel restores the hidden state and OK preserves the chosen style.
#[test]
fn pcb_grid_preferences_restore_hidden_grid() {
    use pcbcraft_chrome::grid::Style;
    for opacity in [0, 100] {
        let mut h = board_harness();
        choose(&mut h, "Choose workspace", "PCB layout");
        h.state_mut().preferences.board_grid_style = Style::Dots;
        h.state_mut().preferences.board_grid_visible = false;
        h.state_mut().preferences.grid_opacity_percent = opacity;
        h.run();
        let hidden = h.state().preferences;
        let before = h.state().session.snapshot().unwrap();
        let canvas = h.get_by_label("PCB canvas").rect();
        assert!(super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        click(&mut h, "Preferences");
        click(&mut h, "Dots");
        assert!(h.state().preferences.grid_visible(true));
        click(&mut h, "Cancel");
        assert_eq!(h.state().preferences, hidden);
        assert!(super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        click(&mut h, "Preferences");
        click(&mut h, "Dots");
        click(&mut h, "OK");
        assert!(h.state().preferences.grid_visible(true));
        assert!(!super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        click(&mut h, "View");
        click(&mut h, "Show grid");
        assert!(!h.state().preferences.grid_visible(true));
        click(&mut h, "Show grid");
        assert!(h.state().preferences.grid_visible(true));
        assert!(!super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Open a board with its origin unobstructed, retaining the real layers/properties panels.
fn editable_board() -> Harness<'static, PcbApp> {
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.state_mut().views.active.camera.center_on(Point { x_nm: 0, y_nm: 0 });
    h.run();
    h
}

/// Map board coordinates through the actual view camera and canvas rectangle.
fn board_point(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("PCB canvas").rect())
}

/// Inspect the authored placement independently of its temporary canvas preview.
fn placement(h: &Harness<'_, PcbApp>) -> Value {
    h.state().session.inspect()["project"]["board"]["footprints"][0].clone()
}

/// Keyboard and ribbon rotation keep their screen direction on either board side and in either view.
/// Move previews
/// use the same direction, remain unauthored until commit, and cancel without changing the board.
#[test]
fn footprint_rotation_follows_the_screen_in_both_board_views() {
    for bottom in [false, true] {
        for flipped in [false, true] {
            let mut h = editable_board();
            h.state_mut().views.active.camera.set_mirrored(flipped);
            h.run();
            if bottom {
                let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
                h.hover_at(origin);
                h.key_press(Key::F);
                h.run();
                assert_eq!(placement(&h)["side"], "back");
            }
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
            click_canvas_at(&mut h, origin);
            let pad_direction = |h: &Harness<'_, PcbApp>| {
                let project = h.state().session.project().unwrap();
                let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
                let footprint = board.footprints().next().unwrap();
                board_point(h, footprint.pads[0].center) - board_point(h, footprint.position)
            };
            let initial = pad_direction(&h);
            h.key_press(Key::R);
            h.run();
            let rotated = pad_direction(&h);
            assert!(initial.x * rotated.y - initial.y * rotated.x < 0., "R is counterclockwise on screen, flipped={flipped}");
            assert_eq!(placement(&h)["rotation"], if flipped { 270 } else { 90 });
            h.key_press_modifiers(Modifiers::SHIFT, Key::R);
            h.run();
            assert_eq!(placement(&h)["rotation"], 0, "Shift+R reverses R");
            assert!((pad_direction(&h) - initial).length() < 0.01);
            click(&mut h, "Rotate selected footprint or text counterclockwise");
            assert_eq!(placement(&h)["rotation"], if flipped { 270 } else { 90 });
            let before_move = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            h.hover_at(origin);
            h.key_press(Key::M);
            h.run();
            h.key_press_modifiers(Modifiers::SHIFT, Key::R);
            h.run();
            assert_eq!(h.state().views.active.board.draft.as_ref().unwrap().rotation, 0);
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before_move);
            h.key_press(Key::Escape);
            h.run();
            assert!(h.state().views.active.board.draft.is_none());
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before_move);
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.run();
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// Dragging and rotation stay local until release; both changes commit together and undo once.
#[test]
fn footprint_drag_rotation_and_undo_share_one_command() {
    let mut h = editable_board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let start = board_point(&h, Point { x_nm: 0, y_nm: 0 });
    let destination = board_point(&h, Point { x_nm: 4_000_000, y_nm: 2_000_000 });
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.run();
    h.hover_at(destination);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    h.key_press(Key::R);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.drop_at(destination);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 1);
    let changed = placement(&h);
    assert_eq!((changed["x_nm"].as_i64(), changed["y_nm"].as_i64(), changed["rotation"].as_u64()), (Some(3_810_000), Some(2_540_000), Some(90)));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run();
    assert_eq!(placement(&h), changed);
}

/// Screen positions of a painted text label in the most recent frame only.
fn painted_text(h: &Harness<'_, PcbApp>, text: &str) -> Vec<egui::Pos2> {
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => Some(shape.pos),
            _ => None,
        })
        .collect()
}

/// The frame that commits a drop, a click-to-place move or a hover rotation already paints the
/// footprint at its new pose. Painting the old pose first makes the part spring back for a frame.
#[test]
fn committed_footprint_transforms_never_paint_the_previous_pose() {
    let mut h = editable_board();
    let reference = h.state().session.inspect()["project"]["schematic"]["symbols"][0]["reference"].as_str().unwrap().to_owned();
    let before = painted_text(&h, &reference);
    assert_eq!(before.len(), 1, "{reference} is painted once");
    let start = board_point(&h, Point { x_nm: 0, y_nm: 0 });
    let destination = board_point(&h, Point { x_nm: 7_620_000, y_nm: 5_080_000 });
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.run();
    h.hover_at(destination);
    h.run();
    let dragging = painted_text(&h, &reference);
    // Release alone: `drop_at` also queues PointerGone, and `step` would paint that later frame.
    h.event(egui::Event::PointerButton { pos: destination, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(placement(&h)["x_nm"], 7_620_000, "the release frame commits the move");
    assert_eq!(painted_text(&h, &reference), dragging, "the release frame paints the dropped pose");
    h.run();
    assert_eq!(painted_text(&h, &reference), dragging);

    let moved = painted_text(&h, &reference);
    h.key_press(Key::M);
    h.run();
    let second = board_point(&h, Point { x_nm: -2_540_000, y_nm: 1_270_000 });
    h.hover_at(second);
    h.run();
    let previewed = painted_text(&h, &reference);
    assert_ne!(previewed, moved);
    h.event(egui::Event::PointerButton { pos: second, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: second, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(placement(&h)["x_nm"], -2_540_000, "the click frame commits the move");
    assert_eq!(painted_text(&h, &reference), previewed, "the click frame paints the placed pose");

    h.run();
    let placed = painted_text(&h, &reference);
    h.key_press(Key::R);
    h.step();
    assert_eq!(placement(&h)["rotation"], 90, "the key frame commits the rotation");
    let rotated = painted_text(&h, &reference);
    h.run();
    assert_ne!(placed, painted_text(&h, &reference), "rotation moves the label");
    assert_eq!(rotated, painted_text(&h, &reference), "the key frame paints the rotated pose");
}

/// Hover M and R use the same transform path as the ribbon, and four quarter turns close exactly.
#[test]
fn footprint_hotkeys_and_ribbon_preserve_exact_quarter_turns() {
    let mut h = editable_board();
    let start = board_point(&h, Point { x_nm: 0, y_nm: 0 });
    h.hover_at(start);
    h.run();
    h.key_press(Key::M);
    h.run();
    let destination = board_point(&h, Point { x_nm: -2_540_000, y_nm: 1_270_000 });
    h.hover_at(destination);
    h.run();
    h.key_press(Key::R);
    h.run();
    click_canvas_at(&mut h, destination);
    let changed = placement(&h);
    assert_eq!(changed["x_nm"], -2_540_000);
    assert_eq!(changed["y_nm"], 1_270_000);
    assert_eq!(changed["rotation"], 90);
    for _ in 0..3 {
        click(&mut h, "Rotate selected footprint or text counterclockwise");
    }
    assert_eq!(placement(&h)["rotation"], 0);
    let center = board_point(&h, Point { x_nm: -2_540_000, y_nm: 1_270_000 });
    h.hover_at(center);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::R);
    h.run();
    assert_eq!(placement(&h)["rotation"], 270);
    h.key_press(Key::R);
    h.run();
    assert_eq!(placement(&h)["rotation"], 0);
    click(&mut h, "Move selected footprint, text or copper");
    assert!(h.state().views.active.board.draft.is_some());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.board.draft.is_none());
    assert_eq!(placement(&h)["x_nm"], -2_540_000);
}

/// Cancelling, dropping over a panel, or a concurrent revision never commits a captured draft.
#[test]
fn footprint_moves_cancel_on_escape_panel_release_and_stale_revision() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Cancel {
        Escape,
        Panel,
        Revision,
        Reopen,
        Workspace,
        Home,
        Modal,
        Focus,
        Secondary,
    }
    for mode in [
        Cancel::Escape,
        Cancel::Panel,
        Cancel::Revision,
        Cancel::Reopen,
        Cancel::Workspace,
        Cancel::Home,
        Cancel::Modal,
        Cancel::Focus,
        Cancel::Secondary,
    ] {
        let mut h = editable_board();
        let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let start = board_point(&h, Point { x_nm: 0, y_nm: 0 });
        h.hover_at(start);
        h.run();
        h.drag_at(start);
        h.run();
        let destination = start + egui::vec2(100., 60.);
        h.hover_at(destination);
        h.run();
        assert!(h.state().views.active.board.draft.is_some(), "{mode:?}");
        match mode {
            Cancel::Escape => h.key_press(Key::Escape),
            Cancel::Panel => {}
            Cancel::Revision => {
                h.state_mut().session.apply("project.rename", &json!({"name":"Concurrent"})).unwrap();
            }
            Cancel::Reopen => {
                let snapshot = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(snapshot);
            }
            Cancel::Workspace => {
                let app = h.state_mut();
                let id = app.views.active.id();
                app.views.switch_workspace(id, Workspace::Schematic, &app.session);
            }
            Cancel::Home => h.state_mut().show_home(),
            Cancel::Modal => h.state_mut().open_preferences(),
            Cancel::Focus => h.input_mut().focused = false,
            Cancel::Secondary => h.event(egui::Event::PointerButton {
                pos: destination,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }),
        }
        h.run();
        let drop = if mode == Cancel::Panel { h.get_by_label("Reference").rect().center() } else { destination };
        h.drop_at(drop);
        h.run();
        assert!(h.state().views.active.board.draft.is_none(), "{mode:?}");
        let mut after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        if mode == Cancel::Revision {
            after.insert("project.json".into(), bytes["project.json"].clone());
        }
        assert_eq!(after, bytes, "{mode:?}");
    }
}

/// Live edits cancel a local preview, update resolved pads in every board view, and can be undone
/// from the schematic. The live adapter also rejects a stale request without changing history.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_footprint_transform_shares_editor_history_and_invalidates_drafts() {
    let mut h = editable_board();
    let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
    h.hover_at(origin);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let board = h.state().session.project().unwrap().board_target().unwrap();
    let ctx = h.ctx.clone();
    let request = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,"params":{
        "board":board.board,"footprint":"footprint-3","x_nm":2_540_000,"y_nm":1_270_000,"rotation":270,"expected_revision":revision
    }}});
    let answer = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap();
    assert_eq!(answer["project"]["board"]["footprints"][0]["rotation"], 270);
    h.run();
    assert!(h.state().views.active.board.draft.is_none());
    assert!(pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    click(&mut h, "Choose workspace");
    click(&mut h, "Schematic");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A board tool belongs to its focused pane: switching to the schematic cancels the draft,
/// while a completed board edit can be undone there without changing either view's camera.
#[test]
fn footprint_tools_in_split_views_share_history_without_sharing_input() {
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let board_id = ViewId(2);
    h.state_mut().views.view_mut(board_id).unwrap().camera.center_on(Point { x_nm: 0, y_nm: 0 });
    h.run();
    let rect = h.get_by_label("PCB canvas 3").rect();
    let origin = h.state().views.view(board_id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
    click_canvas_at(&mut h, origin);
    let schematic = h.get_by_label("Schematic canvas 1").rect();
    let schematic_origin = h.state().views.view(ViewId(0)).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, schematic);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.hover_at(origin);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    click_canvas_at(&mut h, schematic.center());
    assert_eq!(h.state().views.active.id(), ViewId(0));
    assert!(h.state().views.view(board_id).unwrap().board.draft.is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click_canvas_at(&mut h, origin);
    h.key_press(Key::M);
    h.run();
    h.hover_at(origin + egui::vec2(50., 50.));
    h.run();
    h.key_press(Key::R);
    h.run();
    click_canvas_at(&mut h, origin + egui::vec2(50., 50.));
    assert_eq!(placement(&h)["rotation"], 90);
    assert_eq!(h.state().views.view(board_id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, rect), origin);
    assert_eq!(h.state().views.view(ViewId(0)).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, schematic), schematic_origin);
    click_canvas_at(&mut h, schematic.center());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A successful folder/archive Open from PCB layout binds the incoming project's board before
/// constructing its fallback arrangement, including cached views of the outgoing project.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn opening_another_project_from_pcb_layout_binds_its_board() {
    for (archive, split) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut h = board_harness();
        if split {
            click(&mut h, "Choose workspace");
            click(&mut h, "Open PCB layout beside current view");
            let at = h.get_by_label("PCB canvas 3").rect().center();
            click_canvas_at(&mut h, at);
        } else {
            click(&mut h, "Choose workspace");
            click(&mut h, "PCB layout");
        }
        let before = h.state().views.active.input(&h.state().session).unwrap();
        // The folder case opens a private copy; see `dual_led_copy`.
        let (_copy, path) = if archive {
            (None, std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/dual-led.pc"))
        } else {
            let (copy, path) = dual_led_copy();
            (Some(copy), path)
        };
        let revision = h.state().session.revision();
        h.state_mut().files.open_path_for_test(&path, revision);
        h.run();
        assert_eq!(h.state().session.project().unwrap().name, "Dual LED · editing demo");
        assert_eq!(h.state().views.active.workspace, Workspace::Board);
        assert!(!h.state().views.active.unavailable());
        assert!(h.query_by_label("This document is unavailable. Choose a workspace to replace this view.").is_none());
        let target = h.state().session.project().unwrap().board_target().unwrap();
        assert_eq!(h.state().views.active.board.target, Some(target));
        assert!(!h.state().views.active.accepts(before, &h.state().session));
        assert!(!h.state().files.dirty(&h.state().session));
        let snapshot = h.state().arrangement_snapshot().unwrap();
        assert!(snapshot.valid());
        assert!(h.query_by_label("This arrangement exceeds the supported layout limits; its last saved layout has been kept.").is_none());
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("unavailable"));
        click(&mut h, "Choose workspace");
        click(&mut h, "Schematic");
        assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        assert!(!h.state().views.active.unavailable());
        assert_eq!(h.state().views.active.board.target, Some(target));
    }
}

/// Refused file loads do not run project rebinding or replace the current PCB document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn failed_project_open_preserves_current_pcb_binding() {
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.snapshot().unwrap();
    let target = h.state().views.active.board.target;
    let revision = h.state().session.revision();
    let dir = tempfile::tempdir().unwrap();
    h.state_mut().files.open_path_for_test(&dir.path().join("missing.pc"), revision);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(h.state().views.active.board.target, target);
    assert!(!h.state().views.active.unavailable());
}

/// Physical pad polygons retain real dimensions under rotation and bind to front copper,
/// independently of the active layer. Hiding copper leaves technical graphics available.
#[test]
fn smd_pads_render_physical_geometry_and_respect_layer_visibility() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_with_board_at_origin(&mut h);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.state_mut().views.active.camera.request_fit();
    h.run();
    let target = h.state().views.active.board.target.unwrap();
    let setup = h.state().session.project().unwrap().board_view(target).unwrap().setup().unwrap();
    let top = setup.layers().iter().find(|l| l.name == "Top").unwrap().id.to_string();
    let bottom = setup.layers().iter().find(|l| l.name == "Bottom").unwrap().id.to_string();
    let count = |h: &Harness<'_, PcbApp>, color| {
        h.output().shapes.iter().filter(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.fill==color && path.points.len()>=4)).count()
    };
    assert_eq!(count(&h, Tokens::get().board_front), 2);
    h.state_mut().views.active.board.active_layer = Some(bottom);
    h.run();
    assert_eq!(count(&h, Tokens::get().board_front), 2);
    h.state_mut().views.active.board.hidden_layers.insert(top.clone());
    h.run();
    assert_eq!(count(&h, Tokens::get().board_front), 0, "inner/back copper cannot expose component-side SMD pads");
    h.state_mut().views.active.board.hidden_layers.remove(&top);
    h.run();
    let rect = h.get_by_label("PCB canvas").rect();
    let center = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
    click_canvas_at(&mut h, center);
    h.key_press(Key::R);
    h.run();
    let project = h.state().session.project().unwrap();
    let footprint = project.board_view(target).unwrap().footprints().next().unwrap();
    assert_eq!(footprint.rotation, 90);
    for pad in footprint.pads {
        let expected = pad.copper.unwrap().map(|point| h.state().views.active.camera.screen(point, rect));
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.fill==pcbcraft_chrome::board::copper_emphasis(Tokens::get().board_front, Some(true)) && {
            let bounds = path.points.iter().fold(egui::Rect::NOTHING, |rect, p| rect.union(egui::Rect::from_pos(*p)));
            let expected = expected.into_iter().fold(egui::Rect::NOTHING, |rect, p| rect.union(egui::Rect::from_pos(p)));
            bounds.min.distance(expected.min) < 0.01 && bounds.max.distance(expected.max) < 0.01
        })));
    }
}

/// The real demo uses bounded white labels, rounded SMD lands and actual drilled header lands.
/// Copper visibility respects through-hole ownership; detail-layer choices survive view restore.
#[test]
fn physical_demo_labels_drills_and_detail_visibility() {
    use pcbcraft_engine::board::setup::{LayerPurpose, Side};
    let mut h = harness();
    h.set_size(egui::vec2(1440., 950.));
    let (_copy, path) = dual_led_copy();
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&path, revision);
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let mut camera = serde_json::to_value(h.state().views.active.camera.saved_state()).unwrap();
    camera["zoom"] = json!(6.);
    h.state_mut().views.active.camera = crate::canvas_camera::Camera::from_state(&serde_json::from_value(camera).unwrap());
    h.state_mut().views.active.camera.center_on(Point { x_nm: 0, y_nm: 2_000_000 });
    h.run();
    let canvas = h.get_by_label("PCB canvas").rect();
    let copper_bounds = |h: &Harness<'_, PcbApp>, color| -> Vec<egui::Rect> {
        h.output()
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Path(path) = &shape.shape
                    && path.fill == color
                {
                    Some(path.points.iter().fold(egui::Rect::NOTHING, |r, point| r.union(egui::Rect::from_pos(*point))))
                } else {
                    None
                }
            })
            .collect()
    };
    let lands = copper_bounds(&h, Tokens::get().board_front);
    assert_eq!(lands.len(), 15, "surface and plated lands use their front copper layer color");
    let labels: Vec<_> = h
        .output()
        .shapes
        .iter()
        .filter_map(|shape| {
            if shape.clip_rect == canvas
                && let egui::Shape::Text(text) = &shape.shape
                && text.galley.job.text.lines().next().is_some_and(|line| line.parse::<u8>().is_ok())
            {
                Some(text)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(labels.len(), 15);
    assert_eq!(labels.iter().filter(|text| text.galley.job.text.contains("Net ")).count(), 12);
    for label in labels {
        assert!(
            lands.iter().any(|land| land.expand(1.).contains_rect(label.visual_bounding_rect())),
            "label escaped copper bounds: {:?}",
            label.galley.job.text
        );
        assert_eq!(label.fallback_color, egui::Color32::WHITE);
    }
    assert_eq!(
        h.output().shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::Circle(c) if c.fill == Tokens::get().board_canvas)).count(),
        2
    );
    let target = h.state().session.project().unwrap().board_target().unwrap();
    let setup = h.state().session.project().unwrap().board_view(target).unwrap().setup().unwrap();
    let top = setup.layers().iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)).unwrap().id.to_string();
    let bottom = setup.layers().iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Back)).unwrap().id.to_string();
    let details: Vec<_> = setup
        .layers()
        .iter()
        .filter(|l| matches!(l.purpose, LayerPurpose::Fabrication | LayerPurpose::Courtyard))
        .map(|l| l.id.to_string())
        .collect();
    assert!(!details.is_empty());
    assert!(details.iter().all(|id| h.state().views.active.board.hidden_layers.contains(id)));
    h.state_mut().views.active.board.hidden_layers.insert(top.clone());
    h.run();
    assert!(copper_bounds(&h, Tokens::get().board_front).is_empty());
    assert_eq!(copper_bounds(&h, Tokens::get().board_back).len(), 2);
    h.state_mut().views.active.board.hidden_layers.insert(bottom);
    h.run();
    assert!(copper_bounds(&h, Tokens::get().board_back).is_empty());
    h.state_mut().views.active.board.hidden_layers.remove(&top);
    h.state_mut().views.active.board.hidden_layers.remove(&details[0]);
    h.run();
    let state = h.state().views.active.saved_state(&h.state().session).unwrap();
    let mut restored = crate::editor_view::EditorView::from_state(ViewId(99), &state, &h.state().session);
    restored.sync(&h.state().session);
    assert_eq!(restored.board.hidden_layers, h.state().views.active.board.hidden_layers);
}

/// Open board-wide print settings through the shared setup navigation at either host width.
pub(super) fn open_silkscreen(h: &mut Harness<'_, PcbApp>) {
    click(h, "Board setup…");
    if h.query_by_label("Board setup page").is_some() {
        click(h, "Board setup page");
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Silkscreen").click();
    h.run();
}

/// Board settings edit physical geometry together and share undo with other workspaces.
#[test]
fn silkscreen_settings_apply_cancel_and_undo() {
    use egui::accesskit::Role;
    let mut h = editable_board();
    let revision = h.state().session.revision();
    h.state_mut()
        .run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    open_silkscreen(&mut h);
    let input = h.get_by_role_and_label(Role::TextInput, "Text height");
    input.click();
    input.type_text("2 mm");
    h.run();
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    open_silkscreen(&mut h);
    let input = h.get_by_role_and_label(Role::TextInput, "Text height");
    input.click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Text height").type_text("1.2 mm");
    h.run();
    click(&mut h, "Apply board setup");
    assert_eq!(h.state().session.inspect()["project"]["board"]["silkscreen"]["text_height_nm"], 1_200_000);
    let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_ne!(after, before);
    let center = h.get_by_label("PCB canvas").rect().center();
    click_canvas_at(&mut h, center);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Editing a rounded readout must preserve untouched nanometer values, including when another
/// field changes. Opening and applying the form alone is a byte-stable, history-preserving no-op.
#[test]
fn silkscreen_editor_preserves_unedited_precision_in_both_units() {
    use egui::accesskit::Role;
    for unit in [pcbcraft_chrome::units::Unit::Mil, pcbcraft_chrome::units::Unit::Mm] {
        for extra in [0, 1] {
            let mut h = editable_board();
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::INITIALIZE_BOARD_SETUP,
                &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}),
            );
            h.run();
            h.state_mut().preferences.units = unit;
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::SET_SILKSCREEN,&json!({"settings":{"text_height_nm":1_000_000+extra,"stroke_width_nm":150_000+extra,"pad_clearance_nm":250_000+extra},"expected_revision":revision}));
            h.run();
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            let revision = h.state().session.revision();
            open_silkscreen(&mut h);
            click(&mut h, "Apply board setup");
            assert_eq!(h.state().session.revision(), revision);
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
            open_silkscreen(&mut h);
            h.get_by_role_and_label(Role::TextInput, "Stroke width").click();
            h.run();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.run();
            h.get_by_role_and_label(Role::TextInput, "Stroke width").type_text("0.18 mm");
            h.run();
            click(&mut h, "Apply board setup");
            let style = h.state().session.inspect()["project"]["board"]["silkscreen"].clone();
            assert_eq!(style["stroke_width_nm"], 180_000);
            assert_eq!(style["text_height_nm"], 1_000_000 + extra);
            assert_eq!(style["pad_clearance_nm"], 250_000 + extra);
        }
    }
}

/// Ribbon and Delete act on an explicit board selection, never just a hovered footprint.
/// Movement previews and Board setup own keyboard input, and undo restores the exact placement.
#[test]
fn footprint_deletion_uses_selected_placements_and_shared_undo() {
    for ribbon in [false, true] {
        let mut h = editable_board();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let schematic = h.state().session.inspect()["project"]["schematic"].clone();
        let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
        h.hover_at(origin);
        h.run();
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click_canvas_at(&mut h, origin);
        h.key_press(Key::M);
        h.run();
        assert!(h.state().views.active.board.draft.is_some());
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.active.board.draft.is_none());
        click(&mut h, "Board setup");
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Cancel");
        let revision = h.state().session.revision();
        if ribbon {
            click(&mut h, "Modify menu");
            click(&mut h, "Delete");
        } else {
            h.key_press(Key::Delete);
            h.run();
        }
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
        assert_eq!(h.state().session.inspect()["project"]["schematic"], schematic);
        assert!(h.state().views.active.board.selection.is_empty());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        h.run();
        assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    }
}

/// Live clients use the same deletion command and stale gate, and can undo from the GUI.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_footprint_deletion_shares_history_and_rejects_stale_requests() {
    let mut h = editable_board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let board = h.state().session.project().unwrap().board_target().unwrap().board;
    let footprint = h.state().session.inspect()["project"]["board"]["footprints"][0]["id"].clone();
    let ctx = h.ctx.clone();
    let request = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::DELETE_FOOTPRINTS,"params":{"board":board,"footprints":[footprint],"expected_revision":revision}}});
    let answer = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap();
    assert_eq!(answer["revision"], revision + 1);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    h.run();
    assert!(pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The secondary menu action also accepts real pointer input; Delete has no prominent ribbon
/// button, matching Capture. Opening the menu must retain the canvas selection.
#[test]
fn footprint_delete_menu_accepts_pointer_clicks() {
    for width in [1400.0, 700.0] {
        let mut h = editable_board();
        h.set_size(egui::vec2(width, 700.));
        h.run();
        let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
        click_canvas_at(&mut h, origin);
        assert_eq!(h.state().views.active.board.selection.len(), 1);
        let revision = h.state().session.revision();
        assert!(h.query_by_label("Delete selected footprints").is_none());
        h.get_by_label("Modify menu").scroll_to_me();
        h.run();
        let menu = h.get_by_label("Modify menu").rect();
        click_canvas_at(&mut h, menu.center());
        let action = h.get_by_label("Delete").rect();
        click_canvas_at(&mut h, action.center());
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    }
}

/// Ordinary left-click selection followed immediately by Delete is the primary interaction,
/// for legacy markers and physical packages with or without a configured stackup.
#[test]
fn click_then_delete_removes_footprint_without_using_the_ribbon() {
    for configured in [false, true] {
        for definition in ["generic-through-hole-2", "resistor-0402"] {
            let mut h = harness();
            h.set_size(egui::vec2(1400., 900.));
            resistor_with_board_at_origin(&mut h);
            if configured {
                let revision = h.state().session.revision();
                h.state_mut().run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
            }
            let revision = h.state().session.revision();
            let symbol = h.state().session.project().unwrap().symbols()[0].id();
            let mut params = json!({"symbol":symbol,"definition":definition,"x_nm":0,"y_nm":0,"expected_revision":revision});
            if definition == "generic-through-hole-2" {
                params["pin_1_pad"] = json!(1);
            } else {
                params["mapping"] = json!({"1":"1","2":"2"});
            }
            h.state_mut().run(pcbcraft_engine::PLACE_FOOTPRINT, &params);
            h.run();
            click(&mut h, "Choose workspace");
            click(&mut h, "PCB layout");
            h.state_mut().browser.open = false;
            h.run();
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            let revision = h.state().session.revision();
            let origin = board_point(&h, Point { x_nm: 0, y_nm: 0 });
            h.event(egui::Event::PointerMoved(origin));
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: origin, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run();
            }
            assert_eq!(h.state().views.active.board.selection.len(), 1, "{definition} configured={configured}");
            h.key_press(Key::Delete);
            h.run();
            assert_eq!(h.state().session.revision(), revision + 1);
            assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.run();
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// Carry pointer modifiers through a real board drag, optionally keeping the button held so
/// the live overlay and cancellation paths can be inspected before release.
fn board_box(h: &mut Harness<'_, PcbApp>, from: egui::Pos2, to: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers, release: bool) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers });
    h.run();
    h.event(egui::Event::PointerMoved(from + (to - from) * 0.5));
    h.run();
    h.event(egui::Event::PointerMoved(to));
    h.run();
    if release {
        board_box_release(h, to, button, modifiers);
    }
}

/// End the same captured gesture without generating a click.
fn board_box_release(h: &mut Harness<'_, PcbApp>, to: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers) {
    h.event(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers });
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Both buttons show directional PCB selection over visible geometry without editing the
/// project. Modifiers combine hits; selected footprints can be deleted together and undone.
#[test]
fn pcb_box_selection_direction_modifiers_and_delete() {
    use egui::PointerButton::{Primary, Secondary};
    for button in [Primary, Secondary] {
        let mut h = editable_board();
        if button == Secondary {
            h.state_mut().preferences.theme = crate::preferences::ThemeChoice::Light;
            h.run();
        }
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":20_000_000,"y_nm":0,"expected_revision":revision}))
            .unwrap();
        let symbol = h.state().session.project().unwrap().symbols().last().unwrap().id();
        let revision = h.state().session.revision();
        h.state_mut()
            .session
            .apply(
                pcbcraft_engine::PLACE_FOOTPRINT,
                &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":12_000_000,"y_nm":0,"expected_revision":revision}),
            )
            .unwrap();
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let from = board_point(&h, Point { x_nm: -5_000_000, y_nm: -5_000_000 });
        let to = board_point(&h, Point { x_nm: 5_000_000, y_nm: 5_000_000 });
        let original = h.state().session.inspect();
        board_box(&mut h, from, to, button, Modifiers::NONE, false);
        assert!(h.state().views.active.board.box_drag.is_some());
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text()=="Window · 1 items" && text.fallback_color==Tokens::get().board_ink)));
        board_box_release(&mut h, to, button, Modifiers::NONE);
        assert_eq!(h.state().views.active.board.selection.len(), 1);
        assert_eq!(h.state().session.inspect(), original);
        // Only part of the footprint is enclosed; reversing direction changes the result.
        let partial = board_point(&h, Point { x_nm: 0, y_nm: 5_000_000 });
        board_box(&mut h, from, partial, button, Modifiers::NONE, true);
        assert!(h.state().views.active.board.selection.is_empty());
        board_box(&mut h, partial, from, button, Modifiers::NONE, false);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text()=="Crossing · 1 items")));
        board_box_release(&mut h, from, button, Modifiers::NONE);
        assert_eq!(h.state().views.active.board.selection.len(), 1);
        board_box(&mut h, from, to, button, Modifiers::CTRL, true);
        assert!(h.state().views.active.board.selection.is_empty());
        board_box(&mut h, from, to, button, Modifiers::SHIFT, true);
        assert_eq!(h.state().views.active.board.selection.len(), 1);
        let second_from = board_point(&h, Point { x_nm: 7_000_000, y_nm: -5_000_000 });
        let second_to = board_point(&h, Point { x_nm: 17_000_000, y_nm: 5_000_000 });
        board_box(&mut h, second_from, second_to, button, Modifiers::SHIFT, true);
        assert_eq!(h.state().views.active.board.selection.len(), 2);
        h.key_press(Key::Delete);
        h.run();
        assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Escape, focus loss, changed revisions and release over chrome cannot commit a box. Disabled
/// footprint filters and hidden geometry produce no hits, including fast move/release frames.
#[test]
fn pcb_box_selection_cancels_and_respects_filters() {
    use egui::PointerButton::Primary;
    let mut h = editable_board();
    let from = board_point(&h, Point { x_nm: -5_000_000, y_nm: -5_000_000 });
    let to = board_point(&h, Point { x_nm: 5_000_000, y_nm: 5_000_000 });
    for cancellation in ["escape", "focus", "revision", "panel"] {
        board_box(&mut h, from, to, Primary, Modifiers::NONE, false);
        match cancellation {
            "escape" => h.key_press(Key::Escape),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "revision" => {
                let revision = h.state().session.revision();
                h.state_mut().run(
                    pcbcraft_engine::SET_SYMBOL_FIELDS,
                    &json!({"symbol":"symbol-2","reference":"R1","value":"2k","expected_revision":revision}),
                );
            }
            _ => {}
        }
        h.run();
        let end = if cancellation == "panel" { h.get_by_label("Collapse Layers").rect().center() } else { to };
        board_box_release(&mut h, end, Primary, Modifiers::NONE);
        assert!(h.state().views.active.board.box_drag.is_none());
        assert!(h.state().views.active.board.selection.is_empty(), "{cancellation}");
        h.event(egui::Event::WindowFocused(true));
        h.run();
    }
    let view = h.state().views.active.id();
    h.state_mut().set_selection_filters(view, &json!({"footprints":false})).unwrap();
    board_box(&mut h, from, to, Primary, Modifiers::NONE, true);
    assert!(h.state().views.active.board.selection.is_empty());
    h.state_mut().set_selection_filters(view, &json!({"footprints":true})).unwrap();
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    h.event(egui::Event::PointerMoved(to));
    board_box_release(&mut h, to, Primary, Modifiers::NONE);
    assert!(h.state().views.active.board.box_drag.is_none());
    assert_eq!(h.state().views.active.board.selection.len(), 1);
    let revision = h.state().session.revision();
    h.state_mut()
        .run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}));
    h.run();
    let project = h.state().session.project().unwrap();
    let target = project.board_target().unwrap();
    let layers = project.board_view(target).unwrap().layers().into_iter().map(|layer| layer.id.to_string()).collect::<Vec<_>>();
    h.state_mut().views.active.board.hidden_layers = layers.into_iter().collect();
    h.run();
    board_box(&mut h, from, to, Primary, Modifiers::NONE, false);
    assert!(h.state().session.project().unwrap().board_view(target).unwrap().setup().is_some(), "setup missing {:?}", h.state().message);
    assert!(h.state().views.active.board.box_drag.is_some(), "box missing {:?}", h.state().views.active.board);
    assert!(
        h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text()=="Window · 0 items")),
        "wrong hits {:?}",
        h.state().views.active.board
    );
    board_box_release(&mut h, to, Primary, Modifiers::NONE);
    assert!(h.state().views.active.board.selection.is_empty());
}
