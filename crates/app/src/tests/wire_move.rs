use super::*;
use pcbcraft_engine::{
    DRAW_WIRE, PLACE_SYMBOL, no_connect, persistence,
    schematic::{GRID_NM, Point},
};

/// Original free route with several bends beside a symbol, authored through shared commands.
fn editor(width: f32, dark: bool, marked: bool) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.preferences.theme = if dark { crate::ThemeChoice::Dark } else { crate::ThemeChoice::Light };
    app.browser.open = false;
    app.parts.open = false;
    app.properties_open = false;
    app.run("project.new", &json!({}));
    app.run(PLACE_SYMBOL, &json!({"definition":"header-4","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    app.run(
        DRAW_WIRE,
        &json!({"from":point(-20,-5),"to":point(15,-5),
        "bends":[point(-5,-5),point(-5,-15),point(25,-15),point(25,-5)],"expected_revision":app.session.revision()}),
    );
    if marked {
        let target = app.session.project().unwrap().wire_end_targets().find(|(_, at)| *at == point(-20, -5)).unwrap().0;
        app.run(no_connect::SET, &json!({"target":target,"marked":true,"expected_revision":app.session.revision()}));
    }
    for view in app.views.all_mut() {
        view.filters_open = false;
    }
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Resolve model coordinates through the active canvas instead of assuming pixel positions.
fn screen(h: &Harness<'_, PcbApp>, at: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(at, h.get_by_label("Schematic canvas").rect())
}

fn point(x: i64, y: i64) -> Point {
    Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
}

/// A primary click selects a leg and its owner through the real canvas picker.
fn select_wire(h: &mut Harness<'_, PcbApp>) {
    let at = screen(h, point(-10, -5));
    click_canvas_at(h, at);
    assert!(h.state().views.active.schematic.selection.single_wire().is_some());
    assert!(h.state().views.active.schematic.selected_leg.is_some());
}

/// M picks up the highlighted route, preserves every bend and endpoint mark, and publishes
/// exactly one undoable move on click. Hovering another symbol cannot redirect the selection.
#[test]
fn selected_wire_m_previews_and_places_whole_route() {
    for (width, dark, marked) in [(1400., true, false), (700., false, true)] {
        let mut h = editor(width, dark, marked);
        select_wire(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let docs = persistence::encode_documents(before.clone()).unwrap();
        let path = before.wire_path(&before.wires()[0]);
        let revision = h.state().session.revision();
        let origin = screen(&h, point(0, 0));
        h.hover_at(origin);
        h.run();
        h.key_press(Key::M);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::MoveItems { pickup: true, .. })));
        assert!(h.state().views.active.schematic.selected_leg.is_none());
        assert!(h.state().views.active.schematic.whole_wire_drag);
        let to = screen(&h, point(6, 7));
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().session.revision(), revision);
        if marked {
            let target = screen(&h, point(-14, 2));
            assert!(h.get_by_label("No connect wire end").rect().center().distance(target) < 1.);
        }
        click_canvas_at(&mut h, to);
        assert!(h.state().views.active.schematic.tool.is_none());
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.symbols(), before.symbols());
        assert_eq!(
            after.wire_path(&after.wires()[0]),
            path.iter().map(|at| Point { x_nm: at.x_nm + 6 * GRID_NM, y_nm: at.y_nm + 7 * GRID_NM }).collect::<Vec<_>>()
        );
        assert_eq!(after.check_no_connects(0).total_markers, usize::from(marked));
        assert_eq!(h.state().views.active.schematic.selection.single_wire(), Some(after.wires()[0].id().as_str()));
        let moved = persistence::encode_documents(after.clone()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&moved).unwrap()).unwrap(), moved);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), docs);
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert_eq!(persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), moved);
    }
}

/// Keyboard pickup has the normal cancellation paths and rejects stale completion. A marked
/// end translated onto a symbol pin cannot bypass no-connect attachment blocking.
#[test]
fn selected_wire_m_cancels_and_rejects_connection_or_stale_drop() {
    for mode in ["escape", "secondary", "stale", "connection"] {
        let mut h = editor(1400., true, true);
        if mode == "connection" {
            // Group translation follows the grid; match the pin's off-grid residue so the
            // moved endpoint can reach an exact electrical contact rather than merely nearby.
            let p = h.state().session.project().unwrap();
            let pin = p.symbols()[0].pins()[0];
            let target = p.check_no_connects(0).wire_markers[0].target.clone();
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::MOVE_JUNCTION,
                &json!({"junction":target.junction,
                "to":Point{x_nm:pin.x_nm-16*GRID_NM,y_nm:pin.y_nm-2*GRID_NM},"expected_revision":revision}),
            );
            h.run();
        }
        select_wire(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let at = screen(&h, point(-10, -5));
        h.hover_at(at);
        h.key_press(Key::M);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::MoveItems { pickup: true, .. })));
        // The connection case moves the aligned marked source by (+16,+2) onto pin 1.
        let model = if mode == "connection" { point(6, -3) } else { point(-4, 2) };
        let to = screen(&h, model);
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        match mode {
            "escape" => {
                h.key_press(Key::Escape);
                h.run();
            }
            "secondary" => {
                h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
                h.run_steps(1);
                h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
                h.run();
            }
            "stale" => {
                h.state_mut().run("project.rename", &json!({"name":"Concurrent edit"}));
                h.run();
                let changed = h.state().session.snapshot().unwrap();
                click_canvas_at(&mut h, to);
                assert_same_content(&changed, &h.state().session.snapshot().unwrap());
            }
            _ => click_canvas_at(&mut h, to),
        }
        assert!(h.state().views.active.schematic.tool.is_none(), "{mode}");
        assert_eq!(h.state().session.project().unwrap().wires(), before.wires(), "{mode}");
        assert_eq!(h.state().session.project().unwrap().check_no_connects(0).total_markers, 1);
    }
}
