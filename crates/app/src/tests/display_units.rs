//! The display unit is the person's view setting: it changes how lengths read and how bare typed
//! numbers are taken, never the geometry stored in the project.

use egui::accesskit::Role;
use pcbcraft_chrome::units::Unit;
use pcbcraft_engine::{CONNECT_PINS, PLACE_SYMBOL, persistence::encode_documents};

use super::*;

/// Revision, unsaved state, undo availability and saved bytes: everything a view setting must leave alone.
fn project_state(h: &Harness<'_, PcbApp>) -> (u64, bool, bool, Value, pcbcraft_engine::persistence::Documents) {
    let app = h.state();
    let bytes = encode_documents(app.session.snapshot().unwrap()).unwrap();
    (app.session.revision(), app.files.dirty(&app.session), app.session.available("edit.undo"), app.session.inspect(), bytes)
}

/// The status bar's coordinate readout, if one is shown.
fn readout(h: &Harness<'_, PcbApp>) -> Option<String> {
    h.query_all_by_role(Role::Label).filter_map(|node| node.accesskit_node().value()).find(|text| text.starts_with("X "))
}

/// Two connected resistors, so switching units has wiring and connectivity to keep.
fn wired_project() -> Harness<'static, PcbApp> {
    let mut h = harness();
    new_project(&mut h);
    for x_nm in [0, 25_400_000] {
        let revision = h.state().session().revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": x_nm, "y_nm": -1_270_000, "expected_revision": revision}));
    }
    let revision = h.state().session().revision();
    let link = json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}, "expected_revision": revision});
    h.state_mut().run(CONNECT_PINS, &link);
    h.run();
    h
}

/// The readout shows the grid point under the pointer in the display unit, with Y increasing
/// down as the schematic stores it, and disappears off the canvas. The status bar's unit button
/// and Ctrl+U both switch, and the readout follows without the pointer moving.
#[test]
fn the_readout_follows_the_pointer_in_the_display_unit() {
    let h = harness();
    assert!(h.query_by_label("Units: mil").is_none(), "nothing to measure without a schematic");
    let mut h = wired_project();
    assert_eq!(readout(&h), None, "no readout before the pointer reaches the canvas");
    let step = h.state().views.active.camera.snap_step();
    h.hover_at(canvas_center(&h) + egui::vec2(2.0 * step, -3.0 * step));
    h.run();
    assert_eq!(readout(&h).as_deref(), Some("X 100.00  Y -150.00"));
    h.get_by_label("Units: mil");

    h.key_press_modifiers(Modifiers::COMMAND, Key::U);
    h.run();
    assert_eq!(h.state().preferences.units, Unit::Mm);
    assert_eq!(readout(&h).as_deref(), Some("X 2.5400  Y -3.8100"), "100 mil is 2.54 mm");
    h.get_by_label("Grid 1.27 mm");

    // The button switches back; the pointer is on it, off the canvas, until it returns.
    click(&mut h, "Units: mm");
    assert_eq!(h.state().preferences.units, Unit::Mil);
    h.get_by_label("Grid 50 mil");
    h.hover_at(canvas_center(&h) + egui::vec2(2.0 * step, -3.0 * step));
    h.run();
    assert_eq!(readout(&h).as_deref(), Some("X 100.00  Y -150.00"));

    // Off the canvas, over the app bar, there is no grid point to report.
    h.hover_at(h.get_by_label("File").rect().center());
    h.run();
    assert_eq!(readout(&h), None);
}

/// Switching units back and forth leaves the project's geometry, connectivity, revision,
/// history, unsaved state and saved bytes exactly as they were, and keeps the grid spacing.
#[test]
fn switching_units_never_touches_the_project() {
    let mut h = wired_project();
    let before = project_state(&h);
    let spacing = h.state().views.active.camera.snap_nm();
    for _ in 0..5 {
        let button = format!("Units: {}", h.state().preferences.units.symbol());
        click(&mut h, &button);
        h.key_press_modifiers(Modifiers::COMMAND, Key::U);
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::U);
        h.run();
    }
    assert_eq!(h.state().preferences.units, Unit::Mm, "an odd number of switches lands on mm");
    assert_eq!(project_state(&h), before);
    assert_eq!(h.state().views.active.camera.snap_nm(), spacing);
}

/// Mixed-unit stackup lengths enter canonical commands; switching display units preserves
/// their exact values, material reports, connectivity, saved bytes, dirty state and pending redo.
#[test]
fn board_setup_survives_unit_switching_without_an_electrical_edit() {
    use pcbcraft_chrome::units::parse_length;
    use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP, persistence::encode_documents};

    let mut h = wired_project();
    let revision = h.state().session.revision();
    h.state_mut().run("pcb.create_board", &json!({"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
    let mut setup = h.state().session.inspect()["project"]["board"]["setup"].clone();
    let thickness = parse_length("100 mil", Unit::Mm).unwrap();
    assert_eq!(thickness, parse_length("2.54 mm", Unit::Mil).unwrap());
    setup["stackup"]["strata"][1]["thickness_nm"] = json!(thickness);
    let material = json!({"id":"00000000-0000-0000-0000-000000000101","revision":1});
    setup["materials"] = json!([{"id":material["id"],"revision":1,"name":"User material",
        "tg":[{"temperature_milli_c":160125,"method":"dsc","basis":"typical","source":{"document":"Synthetic source"}}],
        "dielectric":[{"dk_millionths":4200000,"df_millionths":15000,"frequency_hz":1000000000,"basis":"typical","source":{"document":"Synthetic source"}}]
    }]);
    setup["stackup"]["strata"][1]["construction"] = json!({"kind":"prepreg","plies":[{
        "id":"00000000-0000-0000-0000-000000000201","material":material,"resin_content_ppm":550000,
        "nominal_thickness_nm":parse_length("4 mil", Unit::Mm).unwrap()
    }]});
    let revision = h.state().session.revision();
    h.state_mut().run(SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision}));
    h.state_mut().run("project.rename", &json!({"name":"An undoable rename"}));
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    let snapshot = h.state().session.snapshot().unwrap();
    let inspected = h.state().session.inspect();
    assert_eq!(inspected["project"]["board"]["setup"]["stackup"]["strata"][1]["thickness_nm"], json!(2_540_000));
    assert_eq!(inspected["project"]["board"]["setup"]["materials"], setup["materials"]);
    assert_eq!(inspected["project"]["board"]["setup"]["stackup"]["strata"][1]["construction"], setup["stackup"]["strata"][1]["construction"]);
    let dirty = h.state().files.dirty(&h.state().session);
    let documents = encode_documents(snapshot.clone()).unwrap();
    for _ in 0..5 {
        let button = format!("Units: {}", h.state().preferences.units.symbol());
        click(&mut h, &button);
        h.key_press_modifiers(Modifiers::COMMAND, Key::U);
        h.run();
    }
    assert!(std::sync::Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.inspect(), inspected);
    assert_eq!(h.state().files.dirty(&h.state().session), dirty);
    assert!(h.state().session.available("edit.redo"));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), documents);
}

/// A bare number typed for the grid is read in the display unit; a typed unit overrides it, so
/// metric values can be entered while showing mils and the other way round.
#[test]
fn typed_spacing_reads_bare_numbers_in_the_display_unit() {
    let mut h = wired_project();
    let custom = |h: &mut Harness<'_, PcbApp>, open: &str, text: &str| {
        click(h, open);
        let field = h.get_by_role_and_label(Role::TextInput, "Custom");
        field.click();
        field.type_text(text);
        h.run();
        click(h, "Apply");
    };
    custom(&mut h, "Grid 50 mil", "25");
    assert_eq!(h.state().views.active.camera.snap_nm(), 25 * grid::NM_PER_MIL);
    custom(&mut h, "Grid 25 mil", "0.635 mm");
    assert_eq!(h.state().views.active.camera.snap_nm(), 25 * grid::NM_PER_MIL, "0.635 mm is 25 mil");

    click(&mut h, "Units: mil");
    custom(&mut h, "Grid 0.635 mm", "0.5");
    assert_eq!(h.state().views.active.camera.snap_nm(), 500_000);
    custom(&mut h, "Grid 0.5 mm · dots 2.5 mm", "100 mil");
    assert_eq!(h.state().views.active.camera.snap_nm(), 2_540_000);
    h.get_by_label("Grid 2.54 mm");
}

/// The Appearance page chooses the unit too; it shows at once and Cancel puts it back.
#[test]
fn preferences_choose_the_unit() {
    let mut h = wired_project();
    click(&mut h, "Preferences");
    h.get_by_label("Units");
    click(&mut h, "mm");
    assert_eq!(h.state().preferences.units, Unit::Mm, "changes show at once");
    h.get_by_label("Grid 1.27 mm");
    click(&mut h, "Cancel");
    assert_eq!(h.state().preferences.units, Unit::Mil, "Cancel puts the unit back");
}

/// At a narrow window the readout, grid button and unit button stay inside the status bar without
/// overlapping, even far from the origin in millimetres.
#[test]
fn the_status_bar_fits_the_readout_at_narrow_widths() {
    let mut h = wired_project();
    h.set_size(egui::vec2(420.0, 600.0));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::U);
    h.run();
    let step = h.state().views.active.camera.snap_step();
    h.hover_at(canvas_center(&h) + egui::vec2(-10.0 * step, 10.0 * step));
    h.run();
    let readout_node =
        h.query_all_by_role(Role::Label).find(|node| node.accesskit_node().value().is_some_and(|text| text.starts_with("X "))).unwrap();
    let shown = readout_node.rect();
    let grid = h.get_by(|node| node.label().is_some_and(|label| label.starts_with("Grid "))).rect();
    let unit = h.get_by_label("Units: mm").rect();
    for rect in [shown, grid, unit] {
        assert!(rect.left() >= 0.0 && rect.right() <= 420.0, "{rect:?}");
    }
    assert!(shown.right() <= grid.left() && grid.right() <= unit.left(), "{shown:?} {grid:?} {unit:?}");
}

/// With the editor split, the readout follows the pointer over either pane, each read through its
/// own camera, whichever pane has focus.
#[test]
fn the_readout_follows_the_pointer_in_both_split_panes() {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    new_project(&mut h);
    h.state_mut().browser.open = false;
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    assert_eq!(h.state().views.active.id(), ViewId(0));
    for pane in ["Schematic canvas 1", "Schematic canvas 2"] {
        h.hover_at(h.get_by_label(pane).rect().center());
        h.run();
        assert_eq!(readout(&h).as_deref(), Some("X 0.00  Y 0.00"), "{pane}");
    }
    // Move only the unfocused pane's view, so its own camera, not the focused one, decides the point.
    let target = pcbcraft_engine::schematic::Point { x_nm: 200 * grid::NM_PER_MIL, y_nm: -100 * grid::NM_PER_MIL };
    h.state_mut()
        .views
        .view_mut(crate::ViewId(1))
        .unwrap()
        .camera
        .center_on(pcbcraft_engine::schematic::Point { x_nm: 500 * grid::NM_PER_MIL, y_nm: 0 });
    h.run();
    let at = h.state().views.view(crate::ViewId(1)).unwrap().camera.screen(target, h.get_by_label("Schematic canvas 2").rect());
    h.hover_at(at);
    h.run();
    assert_eq!(readout(&h).as_deref(), Some("X 200.00  Y -100.00"));
    assert_eq!(h.state().views.active.id(), ViewId(0), "hovering doesn't move focus");
}
