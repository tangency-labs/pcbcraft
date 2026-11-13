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
        click(&mut h, "Delete");
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
    click(&mut h, "Mark unused pins or wire ends (Q)");
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
        click(&mut h, "Mark unused pins or wire ends (Q)");
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
    click(&mut h, "Mark unused pins or wire ends (Q)");
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
        assert_eq!(lines.len(), 2, "the armed cursor is a cross centered on its attachment point");
        let bounds = lines.iter().fold(egui::Rect::NOTHING, |bounds, points| bounds.union(egui::Rect::from_two_pos(points[0], points[1])));
        assert!(bounds.width() >= 32. && bounds.height() >= 32., "the cursor must remain legible");
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
    h.state_mut().view_click(source, Click::NoConnect(target.into(), true));
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
            let revision = h.state().session.revision();
            h.state_mut()
                .run(no_connect::SET, &json!({"target":{"part":first.component_id(),"terminal":"3"},"marked":false,"expected_revision":revision}));
            let p = h.state().session.project().unwrap().for_sheet(second).unwrap();
            let terminal = p.symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
            let at = p.terminal_position(terminal).unwrap();
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::DRAW_WIRE,
                &json!({"sheet":second.id(),"from":terminal.inspect(),"to":{"x_nm":at.x_nm+12_700_000,"y_nm":at.y_nm},"bends":[],"expected_revision":revision}),
            );
            let revision = h.state().session.revision();
            h.state_mut()
                .run(no_connect::SET, &json!({"target":{"part":first.component_id(),"terminal":"3"},"marked":true,"expected_revision":revision}));
        }
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        for sheet in [root, second] {
            let app = h.state_mut();
            app.views.active.open_document(crate::editor_view::DocumentTarget::Sheet(sheet), &app.session);
            h.run();
            let center = h.get_by_label("No connect U1 pin 3").rect().center();
            let expected = if connected { Tokens::get().error } else { Tokens::get().schematic_no_connect };
            let crosses = h
                .output()
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 2. && stroke.color == expected && points[0].lerp(points[1], 0.5).distance(center) < 0.1)
                })
                .count();
            assert_eq!(crosses, 2, "connected={connected}, sheet={sheet:?}");
            let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
            assert_eq!(report.markers[0].issues.contains(&"no_connect_connected"), connected);
        }
    }
}

/// Clicking the visible pin lead snaps to its terminal at every mirrored/rotated pose, rather
/// than requiring a click on an invisible six-point target at the end of the lead.
#[test]
fn no_connect_clicks_the_drawn_pin_lead_at_every_pose() {
    use pcbcraft_engine::schematic::{Point, place};
    for turn in 0..4 {
        for mirrored in [false, true] {
            let mut h = editor(1400., true);
            h.state_mut().browser.open = false;
            let symbol = h.state().session.project().unwrap().symbols()[0].id();
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"rotation":turn*90,"expected_revision":revision}));
            if mirrored {
                let revision = h.state().session.revision();
                h.state_mut().run(pcbcraft_engine::MIRROR_SYMBOL, &json!({"symbol":symbol,"axis":"horizontal","expected_revision":revision}));
            }
            h.run();
            click(&mut h, "Mark unused pins or wire ends (Q)");
            let p = h.state().session.project().unwrap();
            let symbol = &p.symbols()[0];
            let (x_nm, y_nm) = place(-4_445_000, -3_810_000, symbol.turns(), symbol.is_mirrored());
            let canvas = h.get_by_label("Schematic canvas").rect();
            let point = h.state().views.active.camera.screen(Point { x_nm: symbol.position.x_nm + x_nm, y_nm: symbol.position.y_nm + y_nm }, canvas);
            assert!(point.distance(pin(&h, 0)) > 12.);
            click_canvas_at(&mut h, point);
            assert!(h.state().message.is_none(), "{:?}", h.state().message);
            h.get_by_label("No connect J1 pin 1");
            let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
            assert_eq!(report.total_markers, 1);
            assert_eq!(report.markers[0].target.terminal, "1");
        }
    }
}

/// The toolbar and Q both mark a real open wire end. Report navigation/removal share the same
/// address, while an interior click gets feedback instead of silently doing nothing.
#[test]
fn no_connect_wire_end_placement_report_and_removal_work() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    for (width, dark) in [(1400., true), (700., false)] {
        let mut h = editor(width, dark);
        h.state_mut().browser.open = false;
        let p = h.state().session.project().unwrap();
        let terminal = p.symbols()[0].terminals()[1];
        let at = Point { x_nm: -13 * GRID_NM, y_nm: -GRID_NM };
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":terminal.inspect(),"to":at,"bends":[],"expected_revision":revision}));
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let graph = before.inspect_schematic()["netlist"].clone();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let end = h.state().views.active.camera.screen(at, canvas);
        click(&mut h, "Mark unused pins or wire ends (Q)");
        click_canvas_at(&mut h, end + egui::vec2(5., 3.));
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        h.get_by_label("No connect wire end");
        let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
        assert_eq!(report.wire_markers.len(), 1);
        assert!(report.wire_findings.is_empty());
        assert_eq!(report.wire_markers[0].appearance.at, at);
        assert_eq!(h.state().session.project().unwrap().inspect_schematic()["netlist"], graph);
        h.key_press(Key::Escape);
        h.run();
        h.state_mut().inspect_no_connects();
        h.run();
        click(&mut h, "Show wire end");
        assert!(h.state().views.active.schematic.selection.single_wire().is_some());
        h.state_mut().inspect_no_connects();
        h.run();
        click(&mut h, "Remove marker");
        assert!(h.query_by_label("No connect wire end").is_none());
        let current = h.get_by_label("Schematic canvas").rect();
        let end = h.state().views.active.camera.screen(at, current);
        h.hover_at(end);
        h.key_press(Key::Q);
        h.run();
        h.get_by_label("No connect wire end");
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert!(h.query_by_label("No connect wire end").is_none());
        click(&mut h, "Mark unused pins or wire ends (Q)");
        let p = h.state().session.project().unwrap();
        let midpoint = Point { x_nm: -9 * GRID_NM, y_nm: -GRID_NM };
        let current = h.get_by_label("Schematic canvas").rect();
        let middle = h.state().views.active.camera.screen(midpoint, current);
        let revision = h.state().session.revision();
        let snapshot = Arc::new(p.clone());
        click_canvas_at(&mut h, middle);
        assert!(h.state().message.as_ref().is_some_and(|message| message.contains("open wire end")));
        assert_eq!(h.state().session.revision(), revision);
        assert_same_content(&snapshot, &h.state().session.snapshot().unwrap());
    }
}

/// A marker placed by GUI input survives the native Save and Open completion paths, with
/// byte-identical repeat saves and a restored navigable ERC intent record.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn no_connect_wire_end_survives_native_save_and_reopen() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    let mut h = editor(1400., true);
    h.state_mut().browser.open = false;
    let pin = h.state().session.project().unwrap().symbols()[0].terminals()[1];
    let at = Point { x_nm: -13 * GRID_NM, y_nm: -GRID_NM };
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":pin.inspect(),"to":at,"bends":[],"expected_revision":revision}));
    h.run();
    let canvas = h.get_by_label("Schematic canvas").rect();
    let end = h.state().views.active.camera.screen(at, canvas);
    click(&mut h, "Mark unused pins or wire ends (Q)");
    click_canvas_at(&mut h, end);
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("No connect wire end");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intentional-termination");
    h.state_mut().files.use_stand_in();
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run();
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().files.finish_stand_in(path.clone());
    h.run();
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let docs = pcbcraft_storage::load(&path).unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run();
    h.state_mut().files.finish_stand_in(path.clone());
    h.run();
    assert_eq!(pcbcraft_storage::load(&path).unwrap(), docs);
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&path, revision);
    h.run();
    h.get_by_label("No connect wire end");
    let report = h.state().session.project().unwrap().check_no_connects(h.state().session.revision());
    assert_eq!(report.wire_markers.len(), 1);
    assert!(report.wire_findings.is_empty());
    assert_eq!(report.wire_markers[0].appearance.at, at);
}

/// Use actual pointer modifiers for additive/toggle selection without borrowing a tool's input.
fn marker_click(h: &mut Harness<'_, PcbApp>, at: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.run();
    h.event(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
        if pressed {
            h.run_steps(1);
        } else {
            h.run();
        }
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Clicking an X takes precedence over starting a wire. Delete/Backspace remove only selected
/// intent; modifier selection, Undo/Redo and the dedicated filter all preserve the owners.
#[test]
fn no_connect_select_and_delete_preserves_pins_and_wires() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    for (width, dark) in [(1400., true), (700., false)] {
        let mut h = editor(width, dark);
        h.state_mut().browser.open = false;
        for index in 0..2 {
            let at = pin(&h, index);
            h.hover_at(at);
            h.key_press(Key::Q);
            h.run();
        }
        let terminal = h.state().session.project().unwrap().symbols()[0].terminals()[2];
        let end = Point { x_nm: -13 * GRID_NM, y_nm: GRID_NM };
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":terminal.inspect(),"to":end,"bends":[],"expected_revision":revision}));
        h.run();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let wire_at = h.state().views.active.camera.screen(end, canvas);
        h.hover_at(wire_at);
        h.key_press(Key::Q);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        assert_eq!(before.check_no_connects(0).total_markers, 3, "fixture markers={:?}", before.check_no_connects(0));
        let docs = encode_documents(before.clone()).unwrap();
        let revision = h.state().session.revision();
        let at = pin(&h, 0);
        assert!(crate::no_connect::marker_at(&before, &h.state().views.active.camera, canvas, at).is_some(), "missing geometric hit at {at:?}");
        marker_click(&mut h, at, Modifiers::NONE);
        assert_eq!(
            h.state().views.active.schematic.selection.no_connects.len(),
            1,
            "selection={:?},tool={:?},filter={},message={:?}",
            h.state().views.active.schematic.selection,
            h.state().views.active.schematic.tool,
            h.state().views.active.filters.no_connects,
            h.state().message
        );
        assert!(h.state().views.active.schematic.selection.symbols.is_empty());
        assert!(h.state().views.active.schematic.selection.wires.is_empty());
        assert!(h.state().views.active.schematic.selected_leg.is_none());
        assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "a selectable X must not show the wiring cursor");
        assert!(h.state().views.active.schematic.tool.is_none(), "clicking X must not start a wire");
        assert_eq!(h.state().session.revision(), revision);
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.project().unwrap().check_no_connects(0).total_markers, 2);
        assert_eq!(h.state().session.project().unwrap().wires(), before.wires());
        assert_eq!(h.state().session.project().unwrap().symbols(), before.symbols());
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), docs);
        let at = pin(&h, 0);
        marker_click(&mut h, at, Modifiers::NONE);
        let at = pin(&h, 1);
        marker_click(&mut h, at, Modifiers::SHIFT);
        assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 2);
        let at = pin(&h, 0);
        marker_click(&mut h, at, Modifiers::CTRL);
        assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 1);
        marker_click(&mut h, wire_at, Modifiers::SHIFT);
        assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 2);
        h.key_press(Key::Backspace);
        h.run();
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.check_no_connects(0).total_markers, 1);
        assert_eq!(after.symbols(), before.symbols());
        assert_eq!(after.wires(), before.wires());
        assert_eq!(after.inspect_schematic()["netlist"], before.inspect_schematic()["netlist"]);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        let id = h.state().views.active.id();
        h.state_mut().set_selection_filters(id, &json!({"symbols":false,"wires":false})).unwrap();
        h.run();
        let at = pin(&h, 0);
        marker_click(&mut h, at, Modifiers::NONE);
        assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 1);
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.project().unwrap().check_no_connects(0).total_markers, 2);
        assert_eq!(h.state().session.project().unwrap().symbols(), before.symbols());
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        let at = pin(&h, 0);
        marker_click(&mut h, at, Modifiers::NONE);
        h.state_mut().set_selection_filters(id, &json!({"no_connects":false})).unwrap();
        h.run();
        assert!(h.state().views.active.schematic.selection.no_connects.is_empty());
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.project().unwrap().check_no_connects(0).total_markers, 3);
        assert_eq!(h.state().session.project().unwrap().wires(), before.wires());
    }
}

/// A containment marquee can select only the mark, and Select All includes marks as distinct
/// objects. Marker selection and its filter never mutate the document.
#[test]
fn no_connect_marquee_and_select_all_include_markers() {
    let mut h = editor(1400., true);
    h.state_mut().browser.open = false;
    let at = pin(&h, 0);
    h.hover_at(at);
    h.key_press(Key::Q);
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let at = pin(&h, 0);
    let (from, to) = (at - egui::vec2(10., 9.), at + egui::vec2(10., 9.));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    h.event(egui::Event::PointerMoved(to));
    h.run();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 1);
    assert!(h.state().views.active.schematic.selection.symbols.is_empty());
    h.key_press(Key::Delete);
    h.run();
    assert!(h.query_by_label("No connect J1 pin 1").is_none());
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    h.state_mut().select_all_schematic();
    h.run();
    assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 1);
    assert_eq!(h.state().views.active.schematic.selection.symbols.len(), 1);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Explicit wiring is blocked at marked pins and wire ends in either direction. The draft
/// remains available after a rejected destination; removing the X allows the same route.
#[test]
fn no_connect_marks_block_explicit_wiring_until_removed() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    for wire_end in [false, true] {
        let mut h = editor(1400., true);
        h.state_mut().browser.open = false;
        let mut target_at = pin(&h, 0);
        if wire_end {
            let terminal = h.state().session.project().unwrap().symbols()[0].terminals()[0];
            let end = Point { x_nm: -13 * GRID_NM, y_nm: -3 * GRID_NM };
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":terminal.inspect(),"to":end,"bends":[],"expected_revision":revision}));
            h.run();
            target_at = h.state().views.active.camera.screen(end, h.get_by_label("Schematic canvas").rect());
        }
        h.hover_at(target_at);
        h.key_press(Key::Q);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        // W at a marked endpoint arms the tool but must not capture it as a source.
        h.hover_at(target_at);
        h.key_press(Key::W);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
        assert!(h.state().message.as_ref().is_some_and(|m| m.contains("Remove the no-connect")));
        assert_eq!(h.state().session.revision(), revision);
        h.key_press(Key::Escape);
        h.run();
        let from = pin(&h, 1);
        h.hover_at(from);
        h.key_press(Key::W);
        h.run();
        h.hover_at(target_at + egui::vec2(0., -10.));
        h.run_steps(1);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::NotAllowed);
        click_canvas_at(&mut h, target_at);
        assert!(h.state().message.as_ref().is_some_and(|m| m.contains("Remove the no-connect")));
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: Some(_), .. })));
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        h.key_press(Key::Escape);
        h.run();
        marker_click(&mut h, target_at, Modifiers::NONE);
        h.key_press(Key::Delete);
        h.run();
        let from = pin(&h, 1);
        h.hover_at(from);
        h.key_press(Key::W);
        h.run();
        click_canvas_at(&mut h, target_at);
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        assert_eq!(h.state().session.project().unwrap().wires().len(), before.wires().len() + 1);
    }
}

/// Approach from outside the X's narrow ink hit area, checking each first frame and clicking
/// without a settled hover. Neither the magnetic preview nor an automatic wire may appear.
#[test]
fn no_connect_hover_approach_and_fast_click_do_not_start_wires() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    for (width, dark) in [(1400., true), (700., false)] {
        let mut h = editor(width, dark);
        h.state_mut().browser.open = false;
        let at = pin(&h, 0);
        h.hover_at(at);
        h.key_press(Key::Q);
        h.run();
        let terminal = h.state().session.project().unwrap().symbols()[0].terminals()[2];
        let end = Point { x_nm: -13 * GRID_NM, y_nm: GRID_NM };
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::DRAW_WIRE, &json!({"from":terminal.inspect(),"to":end,"bends":[],"expected_revision":revision}));
        h.run();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let wire_at = h.state().views.active.camera.screen(end, canvas);
        h.hover_at(wire_at);
        h.key_press(Key::Q);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        let docs = encode_documents(before.clone()).unwrap();
        for center in [pin(&h, 0), wire_at] {
            for offset in
                [egui::vec2(0., -13.), egui::vec2(12., 0.), egui::vec2(-12., 0.), egui::vec2(-9., -9.), egui::vec2(9., -9.), egui::Vec2::ZERO]
            {
                let point = center + offset;
                h.hover_at(point);
                h.run_steps(1);
                assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "hover offset={offset:?}");
                assert!(
                    !h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Circle(circle)
                    if (circle.radius-pcbcraft_chrome::theme::WIRE_CURSOR_TARGET_RADIUS).abs()<0.01
                    && circle.center.distance(center)<0.01 && circle.stroke.color==Tokens::get().schematic_wire)),
                    "connection ring at offset={offset:?}"
                );
                // Move back out, then press at the new location immediately: no settled frame.
                h.hover_at(canvas.center() + egui::vec2(150., 100.));
                h.run();
                h.drag_at(point);
                h.run_steps(1);
                h.drop_at(point);
                h.run();
                assert!(h.state().views.active.schematic.tool.is_none(), "fast click offset={offset:?}: {:?}", h.state().views.active.schematic.tool);
                assert_eq!(h.state().session.revision(), revision);
                assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), docs);
            }
        }
        let id = h.state().views.active.id();
        h.state_mut().set_selection_filters(id, &json!({"no_connects":false})).unwrap();
        h.run();
        let point = pin(&h, 0) + egui::vec2(0., -13.);
        click_canvas_at(&mut h, point);
        assert!(h.state().views.active.schematic.tool.is_none(), "hiding marker selection must not enable accidental wiring");
        assert_eq!(h.state().session.revision(), revision);
    }
}

/// Begin a physical marker drag with a held primary button, preserving its click offset.
fn start_marker_drag(h: &mut Harness<'_, PcbApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run();
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(from.lerp(to, 0.5)));
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
}

/// Release without PointerGone so the existing endpoint tool can resolve the final destination.
fn release_marker_drag(h: &mut Harness<'_, PcbApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
}

/// The selected wire-end X stretches its attached route through the junction tool, retaining
/// identity and selection. Preview/cancel are read-only; commit, reopen and Undo retain intent.
#[test]
fn no_connect_wire_end_drag_moves_wire_and_keeps_marker_selected() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    for (width, dark) in [(1400., true), (700., false)] {
        let mut h = editor(width, dark);
        h.state_mut().browser.open = false;
        let pin0 = h.state().session.project().unwrap().symbols()[0].terminals()[0];
        let (end, to) = (Point { x_nm: -13 * GRID_NM, y_nm: 5 * GRID_NM }, Point { x_nm: -18 * GRID_NM, y_nm: 9 * GRID_NM });
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::DRAW_WIRE,
            &json!({"from":pin0.inspect(),"to":end,"bends":[{"x_nm":end.x_nm,"y_nm":-3*GRID_NM}],"expected_revision":revision}),
        );
        h.run();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let at = h.state().views.active.camera.screen(end, canvas);
        h.hover_at(at);
        h.key_press(Key::Q);
        h.run();
        marker_click(&mut h, at, Modifiers::NONE);
        let before = h.state().session.snapshot().unwrap();
        let docs = encode_documents(before.clone()).unwrap();
        let target = before.check_no_connects(0).wire_markers[0].target.clone();
        let screen = h.state().views.active.camera.screen(to, canvas);
        start_marker_drag(&mut h, at + egui::vec2(3., 3.), screen + egui::vec2(3., 3.));
        assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::Reshape{ref drag,..}) if drag.marker.as_ref()==Some(&target)));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert!(h.get_by_label("No connect wire end").rect().center().distance(screen) < 1.);
        release_marker_drag(&mut h, screen + egui::vec2(3., 3.));
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let moved = h.state().session.snapshot().unwrap();
        assert_eq!(moved.check_no_connects(0).wire_markers[0].appearance.at, to);
        assert_eq!(moved.check_no_connects(0).wire_markers[0].target, target);
        assert_ne!(moved.wire_path(&moved.wires()[0]), before.wire_path(&before.wires()[0]));
        assert_eq!(moved.inspect_schematic()["netlist"], before.inspect_schematic()["netlist"]);
        assert_eq!(h.state().views.active.schematic.selection.no_connects.len(), 1);
        assert!(h.state().views.active.schematic.selection.wires.is_empty());
        let moved_docs = encode_documents(moved.clone()).unwrap();
        assert_eq!(encode_documents(pcbcraft_engine::persistence::decode_documents(&moved_docs).unwrap()).unwrap(), moved_docs);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), docs);
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), moved_docs);
        // Cancel a second drag, then reject a connection drop onto an unmarked pin.
        let cancel = screen + egui::vec2(-35., 25.);
        start_marker_drag(&mut h, screen, cancel);
        h.key_press(Key::Escape);
        h.run();
        release_marker_drag(&mut h, cancel);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), moved_docs);
        let destination = pin(&h, 1);
        start_marker_drag(&mut h, screen, destination);
        release_marker_drag(&mut h, destination);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), moved_docs);
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.project().unwrap().check_no_connects(0).total_markers, 0);
        assert_eq!(h.state().session.project().unwrap().wires(), moved.wires());
    }
}

/// Immediate Q placement, duplicate Q and Capture activation clear prior owner/segment
/// selection and counterpart highlights, so Delete cannot act on a previously selected item.
#[test]
fn no_connect_activation_clears_previous_selection_and_peer_highlights() {
    use pcbcraft_engine::schematic::{GRID_NM, Point};
    let mut h = editor(1400., true);
    h.state_mut().browser.open = false;
    let terminal = h.state().session.project().unwrap().symbols()[0].terminals()[1];
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::DRAW_WIRE,
        &json!({"from":terminal.inspect(),"to":{"x_nm":-13*GRID_NM,"y_nm":-GRID_NM},"bends":[],"expected_revision":revision}),
    );
    h.run();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let source = h.state().views.active.id();
    let peer = h.state().views.hosts.values().flat_map(|host| host.layout.views()).find(|id| *id != source).unwrap();
    let p = h.state().session.snapshot().unwrap();
    let wire = p.wires()[0].id();
    let midpoint = Point { x_nm: -8 * GRID_NM, y_nm: -GRID_NM };
    let leg = schematic_canvas::LegTarget::at_model(&p, &wire, midpoint, h.state().session.revision()).unwrap();
    h.state_mut().select_leg(leg.clone());
    h.run();
    assert!(!h.state().views.highlight_for(peer, &h.state().session).is_empty());
    let canvas = h.get_by_label(&format!("Schematic canvas {}", source.0 + 1)).rect();
    let at = h.state().views.active.camera.screen(p.symbols()[0].pins()[0], canvas);
    h.hover_at(at);
    h.key_press(Key::Q);
    h.run();
    assert!(h.state().views.active.schematic.selection.is_empty());
    assert!(h.state().views.active.schematic.selected_leg.is_none());
    assert!(h.state().views.highlight_for(peer, &h.state().session).is_empty());
    let marked = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().select(Selection::one_symbol(p.symbols()[0].id()));
    h.run();
    assert!(!h.state().views.highlight_for(peer, &h.state().session).is_empty());
    let revision = h.state().session.revision();
    h.hover_at(at);
    h.key_press(Key::Q);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert!(h.state().views.active.schematic.selection.is_empty());
    assert!(h.state().views.highlight_for(peer, &h.state().session).is_empty());
    h.state_mut().select_leg(leg);
    h.run();
    click(&mut h, "Capture");
    click(&mut h, "Mark unused pins or wire ends (Q)");
    assert!(h.state().views.active.schematic.selection.is_empty());
    assert!(h.state().views.active.schematic.selected_leg.is_none());
    assert!(h.state().views.highlight_for(peer, &h.state().session).is_empty());
    h.key_press(Key::Escape);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), marked);
}
