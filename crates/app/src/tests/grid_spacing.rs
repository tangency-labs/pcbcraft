//! Grid spacing is view state: it changes where points snap in one pane, never the project.

use egui::accesskit::Role;
use pcbcraft_chrome::grid::{self, NM_PER_MIL};
use pcbcraft_engine::{
    persistence::encode,
    schematic::{GRID_NM, Point},
    selection::Selection,
};

use super::*;

const MIL: i64 = NM_PER_MIL;

/// Pick a row in the grid menu of the `pane`th sheet tab (0 when not split).
fn choose_grid(h: &mut Harness<'_, PcbApp>, pane: usize, row: &str) {
    let button = h.query_all(egui_kittest::kittest::by().predicate(|node| node.label().is_some_and(|l| l.starts_with("Grid ")))).nth(pane).unwrap();
    button.click();
    h.run();
    click(h, row);
}

/// Type into the grid menu's Custom field and press Enter, leaving the menu as it ends up.
fn type_custom(h: &mut Harness<'_, PcbApp>, text: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, "Custom");
    field.click();
    h.run();
    // Replace whatever an earlier attempt left in the field.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Custom").type_text(text);
    h.run();
    h.key_press(Key::Enter);
    h.run();
}

/// What a grid change must leave alone: revision, unsaved state, undo and the saved bytes.
fn project_state(h: &Harness<'_, PcbApp>) -> (u64, bool, bool, Vec<u8>) {
    let app = h.state();
    let bytes = encode(app.session.snapshot().unwrap()).unwrap();
    (app.session.revision(), app.files.dirty(&app.session), app.session.available("edit.undo"), bytes)
}

/// Presets change where a click places a part and are remembered as a preference, without
/// dirtying the project, advancing its revision or adding an undo step.
#[test]
fn grid_menu_changes_snapping_without_touching_the_project() {
    let mut h = harness();
    new_project(&mut h, "grid");
    let before = project_state(&h);
    assert_eq!(h.state().views.active.camera.snap_nm(), 50 * MIL);
    choose_grid(&mut h, 0, "25 mil");
    h.get_by_label("Grid 25 mil");
    assert_eq!(h.state().views.active.camera.snap_nm(), 25 * MIL);
    assert_eq!(h.state().preferences.schematic_grid_nm, 25 * MIL);
    choose_grid(&mut h, 0, "10 mil");
    choose_grid(&mut h, 0, "25 mil");
    assert_eq!(project_state(&h), before);

    // At the default zoom 50 mil is 10.16 points, so 25 mil is 5.08.
    click(&mut h, "Add symbol (A)");
    select_symbol(&mut h, "Resistor");
    click_canvas_offset(&mut h, egui::vec2(3.0 * 5.08 + 1.0, -2.0));
    assert_eq!(h.state().session.project().unwrap().symbols()[0].position, Point { x_nm: 75 * MIL, y_nm: 0 });
    click(&mut h, "Undo");
    assert!(h.state().session.project().unwrap().symbols().is_empty());
    // Undo restores the document, not the grid.
    assert_eq!(h.state().views.active.camera.snap_nm(), 25 * MIL);
}

/// The Custom field reads mils and millimetres, explains what it refuses and keeps the spacing
/// until something valid is applied.
#[test]
fn custom_spacing_reads_units_and_explains_refusals() {
    let mut h = harness();
    new_project(&mut h, "custom");
    let before = project_state(&h);
    click(&mut h, "Grid 50 mil");
    type_custom(&mut h, "2 cm");
    h.get_by_label("Unknown unit \"cm\". Use mil, mm or in");
    assert_eq!(h.state().views.active.camera.snap_nm(), 50 * MIL);

    type_custom(&mut h, "0");
    h.get_by_label("Use 0.01 mm to 1000 mil");
    assert_eq!(h.state().views.active.camera.snap_nm(), 50 * MIL);
    assert_eq!(h.state().preferences.schematic_grid_nm, 50 * MIL);

    type_custom(&mut h, "0.5 mm");
    assert_eq!(h.state().views.active.camera.snap_nm(), 500_000);
    assert!(h.query_by_role_and_label(Role::TextInput, "Custom").is_none(), "an applied spacing closes the menu");
    // 0.5 mm is about 4 points at the default zoom, so only every fifth dot is drawn.
    click(&mut h, "Grid 0.5 mm · dots 2.5 mm");
    h.get_by_label("0.5 mm (custom)");
    h.get_by_label("Pins are on 50 mil; wire to them by clicking the pin.");
    assert_eq!(project_state(&h), before);
}

/// N steps finer and Shift+N coarser through the presets, wrapping, like KiCad's grid keys.
/// Text fields keep the letter for typing.
#[test]
fn grid_keys_step_through_the_presets() {
    let mut h = harness();
    new_project(&mut h, "keys");
    let before = project_state(&h);
    let snap = |h: &Harness<'_, PcbApp>| h.state().views.active.camera.snap_nm() / MIL;
    h.key_press(Key::N);
    h.run();
    assert_eq!(snap(&h), 25);
    for expected in [10, 5, 100] {
        h.key_press(Key::N);
        h.run();
        assert_eq!(snap(&h), expected);
    }
    h.key_press_modifiers(Modifiers::SHIFT, Key::N);
    h.run();
    assert_eq!(snap(&h), 5);
    h.key_press_modifiers(Modifiers::SHIFT, Key::N);
    h.run();
    assert_eq!(snap(&h), 10);
    assert_eq!(project_state(&h), before);

    h.key_press(Key::A);
    h.run();
    h.get_by_role(Role::TextInput).type_text("n");
    h.run();
    assert_eq!(snap(&h), 10);
}

/// Changing the grid never moves anything, and moving a group keeps each item's offset from
/// the grid, so off-grid parts stay off-grid by the same amount and connections are unchanged.
#[test]
fn off_grid_items_stay_put_and_keep_their_offset_when_moved() {
    let mut h = harness();
    new_project(&mut h, "offgrid");
    let place = |h: &mut Harness<'_, PcbApp>, x: i64, y: i64| {
        let revision = h.state().session.revision();
        h.state_mut().session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":y,"expected_revision":revision})).unwrap();
    };
    place(&mut h, 7 * MIL, 3 * MIL);
    place(&mut h, 307 * MIL, 3 * MIL);
    let revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply(
            CONNECT_PINS,
            &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
        )
        .unwrap();
    let app = h.state_mut();
    app.views.sync(&app.session);
    let nets = |h: &Harness<'_, PcbApp>| h.state().session.inspect()["project"]["schematic"]["netlist"].clone();
    let connected = nets(&h);
    let before = project_state(&h);
    for row in ["100 mil", "10 mil", "25 mil"] {
        choose_grid(&mut h, 0, row);
    }
    assert_eq!(project_state(&h), before);

    let all = Selection { symbols: ["symbol-2".into(), "symbol-3".into()].into(), ..Selection::default() };
    h.state_mut().select(all);
    h.run();
    let rect = h.get_by_label("Schematic canvas").rect();
    let camera = &h.state().views.active.camera;
    let from = camera.screen(Point { x_nm: 7 * MIL, y_nm: 3 * MIL }, rect);
    let to = camera.screen(Point { x_nm: 57 * MIL, y_nm: 78 * MIL }, rect);
    h.drag_at(from);
    h.run();
    for step in 1..=4 {
        h.hover_at(from + (to - from) * step as f32 / 4.0);
        h.run();
    }
    h.drop_at(to);
    h.run();
    let positions: Vec<_> = h.state().session.project().unwrap().symbols().iter().map(|symbol| symbol.position).collect();
    assert_ne!(positions[0], Point { x_nm: 7 * MIL, y_nm: 3 * MIL }, "the group moved");
    for (position, x) in positions.iter().zip([7, 307]) {
        assert_eq!(((position.x_nm - x * MIL) % (25 * MIL), (position.y_nm - 3 * MIL) % (25 * MIL)), (0, 0));
    }
    assert_eq!(nets(&h), connected);
}

/// Pins sit on the 50 mil grid, but they take wires on a coarser or uneven custom grid because
/// they are picked by distance, not by snapping.
#[test]
fn pins_stay_reachable_on_coarse_and_uneven_grids() {
    for spacing in [100 * MIL, 30 * MIL, 330_000] {
        let mut h = harness();
        wiring_project(&mut h);
        let id = h.state().views.active.id();
        h.state_mut().set_view_grid(id, spacing);
        click(&mut h, "Draw wire (W)");
        click_pin(&mut h, 0, 1);
        click_pin(&mut h, 1, 0);
        let wires = h.state().session.inspect()["project"]["schematic"]["wires"].clone();
        assert_eq!(wires[0]["from"], json!({"symbol": "symbol-2", "terminal": "2"}), "{spacing}");
        assert_eq!(wires[0]["to"], json!({"symbol": "symbol-3", "terminal": "1"}), "{spacing}");
    }
}

/// When the zoom thins the dots, the button says how far apart the drawn dots are while the
/// snap spacing stays the same. Zooming in far enough shows every snap point again.
#[test]
fn the_grid_button_reports_thinned_dots() {
    let mut h = harness();
    new_project(&mut h, "thin");
    // 10 mil is about 2 points at the default zoom: every fifth dot is drawn.
    choose_grid(&mut h, 0, "10 mil");
    h.get_by_label("Grid 10 mil · dots 50 mil");
    h.hover_at(canvas_center(&h));
    h.run();
    let zoom = |h: &mut Harness<'_, PcbApp>, factor: f32| {
        for _ in 0..3 {
            h.event(egui::Event::Zoom(factor));
            h.run();
        }
        h.run();
        let camera = &h.state().views.active.camera;
        assert_eq!(camera.snap_nm(), 10 * MIL);
        grid::shown_every(camera.snap_step()).unwrap()
    };
    assert_eq!(zoom(&mut h, 2.0), 1);
    h.get_by_label("Grid 10 mil");
    let every = zoom(&mut h, 0.25);
    assert!(every >= 25, "{every}");
    h.get_by_label(&format!("Grid 10 mil · dots {}", grid::spacing_label(10 * MIL * i64::from(every))));
}

/// Each pane keeps its own spacing through focus changes and reopening; opening a split seeds
/// the new pane from the focused one, and the last spacing chosen is the remembered preference.
#[test]
fn each_pane_keeps_its_own_grid() {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    new_project(&mut h, "panes");
    h.state_mut().browser.open = false;
    choose_grid(&mut h, 0, "25 mil");
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let spacing = |h: &Harness<'_, PcbApp>, id: u64| {
        let views = &h.state().views;
        let view = if views.active.id() == ViewId(id) { &views.active } else { &views.other };
        view.camera.snap_nm() / MIL
    };
    assert_eq!((spacing(&h, 0), spacing(&h, 1)), (25, 25));

    choose_grid(&mut h, 1, "100 mil");
    assert_eq!((spacing(&h, 0), spacing(&h, 1)), (25, 100));
    assert_eq!(h.state().preferences.schematic_grid_nm, 100 * MIL);
    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    h.get_all_by_label(&path).next().unwrap().click();
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(0));
    h.key_press(Key::N);
    h.run();
    assert_eq!((spacing(&h, 0), spacing(&h, 1)), (10, 100));

    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert_eq!((spacing(&h, 0), spacing(&h, 1)), (10, 100));

    // Opening the split again seeds the second pane from the focused one, as its camera is.
    click(&mut h, "Split editor views");
    click(&mut h, "Split editor views");
    assert_eq!((spacing(&h, 0), spacing(&h, 1)), (10, 10));
}

/// At startup both panes take the remembered spacing; a damaged value falls back to 50 mil.
#[test]
fn startup_seeds_both_panes_from_the_preference() {
    let mut views = EditorViews::default();
    views.set_grid_everywhere(Preferences::parse("{\"schematic_grid_nm\":635000}").schematic_grid_nm);
    assert_eq!((views.active.camera.snap_nm(), views.other.camera.snap_nm()), (635_000, 635_000));
    views.set_grid_everywhere(Preferences::parse("{\"schematic_grid_nm\":0}").schematic_grid_nm);
    assert_eq!((views.active.camera.snap_nm(), views.other.camera.snap_nm()), (GRID_NM, GRID_NM));
    views.set_grid_everywhere(-5);
    assert_eq!(views.active.camera.snap_nm(), GRID_NM);
}
