use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, no_connect, persistence::encode_documents};

/// Original multi-pin connector exercises several unused terminals at desktop and compact sizes.
fn editor(width: f32, dark: bool) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.preferences.theme = if dark { crate::ThemeChoice::Dark } else { crate::ThemeChoice::Light };
    app.parts.open = false;
    app.properties_open = false;
    app.run("project.new", &json!({}));
    app.run(PLACE_SYMBOL, &json!({"definition":"header-4","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    for view in app.views.all_mut() {
        view.filters_open = false;
    }
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Resolve each click from the current real canvas camera rather than assuming screen geometry.
fn pin(h: &Harness<'_, PcbApp>, index: usize) -> egui::Pos2 {
    let p = h.state().session.project().unwrap();
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.state().views.active.camera.screen(p.symbols()[0].pins()[index], canvas)
}

/// Q and the shared context menu edit one logical pin, with real marks, no-op and history behavior.
#[test]
fn no_connect_pin_actions_and_history_work_in_both_themes_and_widths() {
    for (width, dark) in [(1400., true), (1400., false), (700., true), (700., false)] {
        let mut h = editor(width, dark);
        for index in 0..3 {
            let at = pin(&h, index);
            h.hover_at(at);
            h.key_press(Key::Q);
            h.run();
            h.get_by_label(&format!("No connect J1 pin {}", index + 1));
        }
        let marked = h.state().session.snapshot().unwrap();
        assert_eq!(marked.check_no_connects(h.state().session.revision()).markers.len(), 3);
        let revision = h.state().session.revision();
        let at = pin(&h, 0);
        h.hover_at(at);
        h.key_press(Key::Q);
        h.run();
        assert_eq!(h.state().session.revision(), revision);
        assert!(Arc::ptr_eq(&marked, &h.state().session.snapshot().unwrap()));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
        click(&mut h, "Remove no-connect marker");
        assert!(h.query_by_label("No connect J1 pin 1").is_none());
        h.get_by_label("No connect J1 pin 2");
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        h.get_by_label("No connect J1 pin 1");
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert!(h.query_by_label("No connect J1 pin 1").is_none());
    }
}

/// Explicit marking cannot create free flags; cancelling leaves exact project bytes and history.
#[test]
fn no_connect_tool_cancel_and_empty_space_are_inert() {
    let mut h = editor(1400., true);
    let before = h.state().session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    click(&mut h, "Mark intentionally unused pins (Q)");
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NoConnect { .. })));
    let canvas = h.get_by_label("Schematic canvas").rect();
    click_canvas_at(&mut h, canvas.right_bottom() - egui::vec2(30., 30.));
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// The repeated ribbon tool authors visible marks on actual pin clicks, then remains armed
/// for the next pin. The Q shortcut must not be required to finish the toolbar interaction.
#[test]
fn no_connect_ribbon_tool_marks_clicked_pins_repeatedly() {
    for (width, dark) in [(1400., true), (700., false)] {
        let mut h = editor(width, dark);
        click(&mut h, "Mark intentionally unused pins (Q)");
        for index in 0..3 {
            let at = pin(&h, index);
            click_canvas_at(&mut h, at);
            assert!(h.state().message.is_none(), "{:?}", h.state().message);
            h.get_by_label(&format!("No connect J1 pin {}", index + 1));
            assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NoConnect { .. })));
            h.hover_at(at);
            h.run();
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
        }
        assert_eq!(h.state().session.project().unwrap().check_no_connects(h.state().session.revision()).total_markers, 3);
    }
}

/// The no-connect glyph replaces the system arrow with its cross centered on the click point,
/// including exact pin snaps. Moving or canceling it leaves the authored document untouched.
#[test]
fn no_connect_tool_shows_a_cursor_before_finding_a_pin() {
    let mut h = editor(1400., true);
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Mark intentionally unused pins (Q)");
    let canvas = h.get_by_label("Schematic canvas").rect();
    let terminal = pin(&h, 0);
    for (pointer, target) in [
        (canvas.center() + egui::vec2(100., 80.), canvas.center() + egui::vec2(100., 80.)),
        (canvas.right_bottom() - egui::vec2(10., 10.), canvas.right_bottom() - egui::vec2(10., 10.)),
        (terminal + egui::vec2(3., 2.), terminal),
    ] {
        h.hover_at(pointer);
        h.run();
        let lines = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == Tokens::get().accent && points.iter().all(|point| point.distance(pointer) < 90.) =>
                {
                    Some(*points)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3, "the armed cursor needs a lead and two cross strokes");
        let bounds = lines.iter().fold(egui::Rect::NOTHING, |bounds, points| bounds.union(egui::Rect::from_two_pos(points[0], points[1])));
        assert!(bounds.width() >= 30. && bounds.height() >= 18., "the cursor must remain legible");
        let diagonals = lines.iter().filter(|points| (points[1].x - points[0].x).abs() > 1. && (points[1].y - points[0].y).abs() > 1.);
        assert_eq!(diagonals.clone().count(), 2);
        for points in diagonals {
            assert!(points[0].lerp(points[1], 0.5).distance(target) < 0.1, "the cross must mark the click point");
        }
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    }
    h.hover_at(canvas.center() + egui::vec2(100., 80.));
    h.key_press(Key::Escape);
    h.run();
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.project().unwrap().check_no_connects(h.state().session.revision()).total_markers, 0);
}

/// Read-only report navigation and deliberate removal use stable addresses, and a stale
/// captured view action cannot mark a changed document.
#[test]
fn no_connect_report_and_stale_actions_keep_revision_authority() {
    let mut h = editor(1400., true);
    let at = pin(&h, 0);
    h.hover_at(at);
    h.key_press(Key::Q);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.state_mut().inspect_no_connects();
    h.run();
    h.get_by_label("No-connect check");
    click(&mut h, "Show J1 pin 1");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let source = h.state().views.active.input(&h.state().session).unwrap();
    let p = h.state().session.project().unwrap();
    let target = p.no_connect_target(p.symbols()[0].terminals()[1]).unwrap();
    h.state_mut().run("project.rename", &json!({"name":"Changed"}));
    let changed = h.state().session.snapshot().unwrap();
    h.state_mut().view_click(source, Click::NoConnect(target, true));
    h.run();
    assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
    let report = h.state_mut().session.execute(no_connect::CHECK, &json!({})).unwrap();
    assert_eq!(report["no_connect_check"]["markers"].as_array().unwrap().len(), 1);
}

/// Mark geometry follows a real Grab/rotate preview, and cancelling restores both the drawn
/// endpoint and exact authored bytes without creating an extra marker transform record.
#[test]
fn no_connect_marks_follow_cancelled_grab_and_rotation_preview() {
    let mut h = editor(1400., true);
    let at = pin(&h, 0);
    h.hover_at(at);
    h.key_press(Key::Q);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let marker = h.get_by_label("No connect J1 pin 1").rect().center();
    let canvas = h.get_by_label("Schematic canvas").rect();
    let center = h.state().views.active.camera.screen(before.symbols()[0].position, canvas);
    h.hover_at(center);
    h.key_press(Key::G);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Move { .. })));
    h.hover_at(center + egui::vec2(100., 60.));
    h.key_press(Key::R);
    h.run();
    assert_ne!(h.get_by_label("No connect J1 pin 1").rect().center(), marker);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.get_by_label("No connect J1 pin 1").rect().center(), marker);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Real X/Y shortcuts carry the drawn cross to the same logical pin at every quarter turn;
/// Undo/Redo restores the mark position, and Q can still pick the mirrored pin without edits.
#[test]
fn no_connect_marks_follow_mirror_shortcuts_and_remain_pickable() {
    for turn in 0..4 {
        for key in [Key::X, Key::Y] {
            let mut h = editor(1400., true);
            let symbol = h.state().session.project().unwrap().symbols()[0].id();
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"rotation":turn*90,"expected_revision":revision}));
            h.run();
            h.hover_at(pin(&h, 0));
            h.key_press(Key::Q);
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let terminal = before.symbols()[0].terminals()[0];
            let target = before.no_connect_target(terminal).unwrap();
            let marker = h.get_by_label("No connect J1 pin 1").rect().center();
            let canvas = h.get_by_label("Schematic canvas").rect();
            let center = h.state().views.active.camera.screen(before.symbols()[0].position, canvas);
            h.hover_at(center);
            h.key_press(key);
            h.run();
            assert!(h.state().message.is_none(), "{:?}", h.state().message);
            assert!(h.state().session.project().unwrap().symbols()[0].is_mirrored());
            let mirrored = h.get_by_label("No connect J1 pin 1").rect().center();
            assert!(mirrored.distance(pin(&h, 0)) < 0.1);
            let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
            assert_eq!(report.markers[0].target, target);

            h.state_mut().run("edit.undo", &json!({}));
            h.run();
            assert_eq!(h.get_by_label("No connect J1 pin 1").rect().center(), marker);
            h.state_mut().run("edit.redo", &json!({}));
            h.run();
            assert_eq!(h.get_by_label("No connect J1 pin 1").rect().center(), mirrored);
            let snapshot = h.state().session.snapshot().unwrap();
            let revision = h.state().session.revision();
            h.hover_at(pin(&h, 0));
            h.key_press(Key::Q);
            h.run();
            assert_eq!(h.state().session.revision(), revision);
            assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// A supply pin repeated on separate sheets has one logical conflict color. Bare repeated
/// appearances stay normal; wiring either appearance colors both crosses as errors.
#[test]
fn no_connect_shared_terminal_color_matches_the_report_on_every_sheet() {
    let mut h = editor(1400., true);
    h.state_mut().run("project.new", &json!({}));
    h.state_mut().run(
        pcbcraft_engine::library::sources::LOAD,
        &json!({"source":"units","documents":{
            "library.json":include_str!("../../../../examples/native-parts/library.json"),
            "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
        }}),
    );
    let definition = h.state().session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    let revision = h.state().session.revision();
    h.state_mut().place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    let first = h.state().session.project().unwrap().symbols()[0].clone();
    let root = h.state().session.project().unwrap().sheet_target();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Second","expected_revision":revision}));
    let second = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_UNIT,
        &json!({"sheet":second.id(),"component":first.component_id(),"unit":"b","x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(no_connect::SET, &json!({"target":{"part":first.component_id(),"terminal":"3"},"marked":true,"expected_revision":revision}));
    for connected in [false, true] {
        if connected {
            let p = h.state().session.project().unwrap().for_sheet(second).unwrap();
            let terminal = p.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
            let at = p.terminal_position(terminal).unwrap();
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::DRAW_WIRE,
                &json!({"sheet":second.id(),"from":terminal.inspect(),"to":{"x_nm":at.x_nm+12_700_000,"y_nm":at.y_nm},"bends":[],"expected_revision":revision}),
            );
        }
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        for sheet in [root, second] {
            let app = h.state_mut();
            app.views.active.open_document(crate::editor_view::DocumentTarget::Sheet(sheet), &app.session);
            h.run();
            let center = h.get_by_label("No connect U1 pin 3").rect().center();
            let expected = if connected { Tokens::get().error } else { Tokens::get().schematic_wire };
            let crosses = h
                .output()
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 1.5 && stroke.color == expected && points[0].lerp(points[1], 0.5).distance(center) < 0.1)
                })
                .count();
            assert_eq!(crosses, 2, "connected={connected}, sheet={sheet:?}");
            let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
            assert_eq!(report.markers[0].issues.contains(&"no_connect_connected"), connected);
        }
    }
}
