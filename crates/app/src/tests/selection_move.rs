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

/// Select the whole original fixture through an ordinary window box, including attached
/// markers. The connected variant also covers a named anchor and pin-to-label route.
fn select_group(h: &mut Harness<'_, PcbApp>, connected: bool) -> pcbcraft_engine::selection::Selection {
    if connected {
        let p = h.state().session.project().unwrap();
        let header = p.symbols()[0].clone();
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::net_labels::ADD,
            &json!({"name":"GROUP","style":"local","x_nm":20*GRID_NM,"y_nm":12*GRID_NM,"expected_revision":revision}),
        );
        let label = h.state().session.project().unwrap().net_labels()[0].object_id();
        let revision = h.state().session.revision();
        h.state_mut()
            .run(DRAW_WIRE, &json!({"from":{"symbol":header.id(),"terminal":"1"},"to":{"net_label":label},"bends":[{"x_nm":20*GRID_NM,"y_nm":header.pins()[0].y_nm}],"expected_revision":revision}));
        assert!(h.state().message.is_none(), "draw route: {:?}", h.state().message);
        assert_eq!(h.state().session.project().unwrap().wires().len(), 2);
        let revision = h.state().session.revision();
        h.state_mut()
            .run(no_connect::SET, &json!({"target":{"part":header.component_id(),"terminal":"4"},"marked":true,"expected_revision":revision}));
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        h.run();
    }
    let from = screen(h, point(-24, -20));
    let to = screen(h, point(30, 18));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    h.hover_at(to);
    h.run();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    let items = h.state().views.active.schematic.selection.clone();
    assert_eq!(items.symbols.len(), 1);
    assert_eq!(items.wires.len(), if connected { 2 } else { 1 });
    assert_eq!(items.net_labels.len(), usize::from(connected));
    assert_eq!(items.no_connects.len(), if connected { 2 } else { 1 });
    items
}

/// M translates the complete mixed selection once, including shared wire contacts and both
/// kinds of no-connect marker, while retaining all IDs and publishing only on placement.
#[test]
fn group_m_moves_symbols_routes_labels_and_owned_markers_together() {
    for (width, dark, connected) in [(1400., true, false), (700., false, true)] {
        let mut h = editor(width, dark, true);
        let items = select_group(&mut h, connected);
        let before = h.state().session.snapshot().unwrap();
        let docs = persistence::encode_documents(before.clone()).unwrap();
        let revision = h.state().session.revision();
        let origin = screen(&h, point(0, 0));
        h.hover_at(origin);
        h.key_press(Key::M);
        h.run();
        assert!(matches!(&h.state().views.active.schematic.tool,Some(Tool::MoveItems{items:captured,pickup:true,..}) if *captured==items));
        let to = screen(&h, point(6, 7));
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().session.revision(), revision);
        click_canvas_at(&mut h, to);
        assert!(h.state().views.active.schematic.tool.is_none());
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let after = h.state().session.snapshot().unwrap();
        let translate = |at: Point| Point { x_nm: at.x_nm + 6 * GRID_NM, y_nm: at.y_nm + 7 * GRID_NM };
        assert_eq!(after.symbols()[0].position, translate(before.symbols()[0].position));
        for (old, new) in before.wires().iter().zip(after.wires()) {
            assert_eq!(new.id(), old.id());
            assert_eq!(new.endpoints(), old.endpoints());
            assert_eq!(after.wire_path(new), before.wire_path(old).into_iter().map(translate).collect::<Vec<_>>());
        }
        for (old, new) in before.net_labels().iter().zip(after.net_labels()) {
            assert_eq!(new.id, old.id);
            assert_eq!(new.at, translate(old.at));
        }
        let old_marks: Vec<_> = before.no_connect_appearances().collect();
        let new_marks: Vec<_> = after.no_connect_appearances().collect();
        assert_eq!(new_marks, old_marks.into_iter().map(|(target, at)| (target, translate(at))).collect::<Vec<_>>());
        assert_eq!(h.state().session.revision(), revision + 1);
        assert_eq!(h.state().views.active.schematic.selection, items);
        let moved = persistence::encode_documents(after).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&moved).unwrap()).unwrap(), moved);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), docs);
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert_eq!(persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), moved);
    }
}

/// Group keyboard pickup shares cancellation and revision/filter guards with held group drags;
/// no partial subset is committed after its captured selection becomes ineligible.
#[test]
fn group_m_cancels_on_escape_secondary_focus_stale_or_filter_change() {
    for mode in ["escape", "secondary", "focus", "stale", "filter"] {
        let mut h = editor(1400., true, true);
        select_group(&mut h, true);
        let before = h.state().session.snapshot().unwrap();
        let origin = screen(&h, point(0, 0));
        h.hover_at(origin);
        h.key_press(Key::M);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::MoveItems { pickup: true, .. })));
        let to = screen(&h, point(6, 7));
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        match mode {
            "escape" => {
                h.key_press(Key::Escape);
                h.run();
            }
            "secondary" => {
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
                    h.run();
                }
            }
            "focus" => {
                h.event(egui::Event::WindowFocused(false));
                h.run();
            }
            "stale" => {
                h.state_mut().run("project.rename", &json!({"name":"Concurrent edit"}));
                h.run();
                let changed = h.state().session.snapshot().unwrap();
                click_canvas_at(&mut h, to);
                assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
            }
            _ => {
                let id = h.state().views.active.id();
                h.state_mut().set_selection_filters(id, &json!({"wires":false})).unwrap();
                h.run();
            }
        }
        assert!(h.state().views.active.schematic.tool.is_none(), "{mode}");
        if mode != "stale" {
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "{mode}");
        }
    }
}

/// Modifier clicks use the same real pointer sequence as canvas selection, with no held
/// modifier leaking into the next keyboard command.
fn modifier_click(h: &mut Harness<'_, PcbApp>, at: Point, modifiers: Modifiers) {
    let at = screen(h, at);
    h.hover_at(at);
    h.event(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run();
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Shift adds a segment's owner and a symbol to one group; Ctrl/Command click removes and
/// re-adds either owner independently without changing authored geometry or history.
#[test]
fn ctrl_click_toggles_wire_and_symbol_in_mixed_selection() {
    for toggle in [Modifiers::CTRL, Modifiers::COMMAND] {
        let mut h = editor(1400., true, false);
        select_wire(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        modifier_click(&mut h, point(0, 0), Modifiers::SHIFT);
        let all = h.state().views.active.schematic.selection.clone();
        assert_eq!(all.symbols.len(), 1);
        assert_eq!(all.wires.len(), 1);
        assert!(h.state().views.active.schematic.selected_leg.is_none());
        for at in [point(-10, -5), point(0, 0)] {
            modifier_click(&mut h, at, toggle);
            assert_eq!(h.state().views.active.schematic.selection.len(), 1);
            if at == point(0, 0) {
                assert!(h.state().views.active.schematic.selection.symbols.is_empty());
            } else {
                assert!(h.state().views.active.schematic.selection.wires.is_empty());
            }
            modifier_click(&mut h, at, toggle);
            assert_eq!(h.state().views.active.schematic.selection, all);
        }
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().session.revision(), revision);
    }
}

/// Add a stationary branch after selecting the group, so it stays outside the captured items.
/// The branch shares a selected pin with an internal selected route.
fn add_unselected_branch(h: &mut Harness<'_, PcbApp>) -> String {
    let p = h.state().session.project().unwrap();
    let header = p.symbols()[0].clone();
    let at = header.pins()[0];
    let revision = h.state().session.revision();
    h.state_mut().run(DRAW_WIRE, &json!({"from":{"symbol":header.id(),"terminal":"1"},"to":{"x_nm":at.x_nm-12*GRID_NM,"y_nm":at.y_nm},"bends":[],"auto_heal":false,"expected_revision":revision}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.run();
    let p = h.state().session.project().unwrap();
    let branch = p.wires().last().unwrap().id();
    assert!(!h.state().views.active.schematic.selection.wires.contains(&branch));
    branch
}

/// M disconnects at a mixed selection boundary and leaves external routes fixed. G uses
/// the same group preview with retained pin contacts, stretching the unselected branch instead.
#[test]
fn group_move_and_grab_have_distinct_selection_boundary_behavior() {
    for (width, dark, key) in [(1400., true, Key::M), (700., false, Key::G)] {
        let mut h = editor(width, dark, true);
        let items = select_group(&mut h, true);
        let branch = add_unselected_branch(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let old_branch = before.wires().iter().find(|wire| wire.id() == branch).unwrap();
        let origin = screen(&h, point(0, 0));
        h.hover_at(origin);
        h.key_press(key);
        h.run();
        assert!(
            matches!(&h.state().views.active.schematic.tool,Some(Tool::MoveItems{items:captured,pickup:true,disconnect,..}) if *captured==items && *disconnect==(key==Key::M))
        );
        let to = screen(&h, point(6, 7));
        h.hover_at(to);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click_canvas_at(&mut h, to);
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let after = h.state().session.snapshot().unwrap();
        let new_branch = after.wires().iter().find(|wire| wire.id() == branch).unwrap();
        if key == Key::M {
            assert_eq!(after.wire_path(new_branch), before.wire_path(old_branch));
            assert!(new_branch.terminals().is_empty());
        } else {
            assert_eq!(new_branch.endpoints(), old_branch.endpoints());
            assert_ne!(after.wire_path(new_branch), before.wire_path(old_branch));
        }
        for old in before.wires().iter().filter(|wire| items.wires.contains(&wire.id())) {
            let new = after.wires().iter().find(|wire| wire.id() == old.id()).unwrap();
            assert_eq!(new.endpoints(), old.endpoints());
            assert_eq!(
                after.wire_path(new),
                before.wire_path(old).into_iter().map(|at| Point { x_nm: at.x_nm + 6 * GRID_NM, y_nm: at.y_nm + 7 * GRID_NM }).collect::<Vec<_>>()
            );
        }
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_same_content(&before, &h.state().session.snapshot().unwrap());
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert_same_content(&after, &h.state().session.snapshot().unwrap());
    }
}
