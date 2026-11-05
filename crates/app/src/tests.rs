//! Drives the real window through its accessibility tree, with clicks and keys, no GPU needed.

use egui::{Key, Modifiers};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use super::*;

fn harness<'a>() -> Harness<'a, PcbApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), PcbApp::default());
    harness.run();
    harness
}

fn choose(harness: &mut Harness<'_, PcbApp>, menu: &str, item: &str) {
    harness.get_by_label(menu).click();
    harness.run();
    harness.get_by_label(item).click();
    harness.run();
}

fn select_symbol(harness: &mut Harness<'_, PcbApp>, name: &str) {
    harness.get_by_label(name).click();
    harness.run();
}

fn new_project(harness: &mut Harness<'_, PcbApp>, name: &str) {
    choose(harness, "File", "New Project…");
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(name);
    harness.run();
    harness.get_by_label("OK").click();
    harness.run();
}

#[test]
fn it_opens_with_no_project_and_says_so() {
    let harness = harness();
    harness.get_by_label("No project open. Choose File, then New Project.");
    assert!(harness.state().session().project().is_none());
}

#[test]
fn a_person_makes_a_project_through_the_menu() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
    assert!(harness.query_by_role(egui::accesskit::Role::TextInput).is_none());
}

/// The promise of the whole project: what a person does through the menus is what the same commands
/// do for an agent.
#[test]
fn the_menus_reach_the_same_state_as_the_commands_an_agent_runs() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    choose(&mut harness, "Edit", "Undo");
    choose(&mut harness, "Edit", "Redo");
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "blinky"})).unwrap();
    session.execute("edit.undo", &Value::Null).unwrap();
    session.execute("edit.redo", &Value::Null).unwrap();
    assert_eq!(harness.state().session().inspect(), session.inspect());
}

#[test]
fn a_bad_name_stays_in_the_dialog_with_the_reason() {
    let mut harness = harness();
    choose(&mut harness, "File", "New Project…");
    harness.get_by_label("OK").click();
    harness.run();
    harness.get_by_label("the name can't be empty");
    assert!(harness.state().session().project().is_none());
}

#[test]
fn cancelling_changes_nothing() {
    let mut harness = harness();
    choose(&mut harness, "File", "New Project…");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().session().revision(), 0);
    assert!(harness.query_by_label("Cancel").is_none());
}

#[test]
fn rename_starts_from_the_current_name_and_can_be_undone() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    choose(&mut harness, "File", "Rename Project…");
    assert_eq!(harness.get_by_role(egui::accesskit::Role::TextInput).value().as_deref(), Some("blinky"));
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(" v2");
    harness.run();
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky v2");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
}

#[test]
fn undo_and_redo_keys_match_kicad_and_do_nothing_when_there_is_nothing_to_do() {
    let mut harness = harness();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().revision(), 0);
    new_project(&mut harness, "blinky");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(harness.state().session().project().is_none());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    harness.run();
    assert!(harness.state().session().project().is_some());
}

#[test]
fn typing_ctrl_z_in_a_name_field_does_not_undo_the_project() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    choose(&mut harness, "File", "Rename Project…");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
}

#[test]
fn preferences_change_the_palette_and_the_scale_and_the_about_box_opens() {
    let mut harness = harness();
    choose(&mut harness, "Preferences", "Preferences…");
    harness.get_by_label("Light").click();
    harness.run();
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Light);
    assert_eq!(harness.ctx.global_style().visuals.panel_fill, Tokens::light().panel);
    harness.get_by_label("125%").click();
    harness.run();
    assert_eq!(harness.state().preferences().scale_percent, 125);
    assert_eq!(harness.ctx.zoom_factor(), 1.25);
    harness.get_by_label("Close").click();
    harness.run();
    choose(&mut harness, "Help", "About PCBCraft");
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_none());
}

#[test]
fn canvas_placement_uses_the_shared_command_and_snaps_to_the_grid() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    choose(&mut harness, "Place", "Place Symbol…");
    select_symbol(&mut harness, "Resistor");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.get_by_label("Place Resistor · Esc to cancel");

    // A click near the center snaps to the schematic origin.
    click_canvas(&mut harness);
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "blinky"})).unwrap();
    session.execute(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": 1})).unwrap();
    assert_eq!(harness.state().session().inspect(), session.inspect());
    click_canvas(&mut harness);
    assert_eq!(harness.state().session().inspect(), session.inspect());
    choose(&mut harness, "Edit", "Undo");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    choose(&mut harness, "Edit", "Redo");
    assert_eq!(harness.state().session().project().unwrap().symbols(), session.project().unwrap().symbols());
}

#[test]
fn escape_and_opening_a_dialog_cancel_the_placement_preview() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("LED");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("LED").is_none());
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());

    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    harness.get_by_label("Place Resistor · Esc to cancel");
    harness.key_press(Key::Escape);
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());

    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    choose(&mut harness, "File", "Rename Project…");
    harness.get_by_label("Cancel").click();
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
}

/// Locate the actual canvas center after the surrounding workspace panels have laid out.
fn canvas_center(harness: &Harness<'_, PcbApp>) -> egui::Pos2 {
    harness.get_by_label("Schematic canvas").rect().center()
}

/// Click at a known offset from the center of the canvas at the default camera zoom.
fn click_canvas_offset(harness: &mut Harness<'_, PcbApp>, offset: egui::Vec2) {
    let point = canvas_center(harness) + offset;
    click_canvas_at(harness, point);
}

fn click_canvas(harness: &mut Harness<'_, PcbApp>) {
    click_canvas_offset(harness, egui::vec2(2.0, 0.0));
}

/// Send a real pointer click to the canvas at a specified logical position.
fn click_canvas_at(harness: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
    harness.hover_at(point);
    harness.run();
    harness.drag_at(point);
    harness.run();
    harness.drop_at(point);
    harness.run();
}

/// The grid coordinate under the zoom cursor follows the middle-button pan.
#[test]
fn wheel_zoom_and_middle_pan_preserve_the_placement_coordinate() {
    let mut harness = harness();
    new_project(&mut harness, "navigation");
    let point = canvas_center(&harness) + egui::vec2(40.64, 0.0);
    harness.hover_at(point);
    harness.run();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 4.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    harness.run();
    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    let middle = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    harness.event(middle(point, true));
    harness.run();
    let panned = point + egui::vec2(120.0, 65.0);
    harness.hover_at(panned);
    harness.run();
    harness.event(middle(panned, false));
    harness.run();
    assert_eq!(harness.state().session().revision(), 1);
    click_canvas_at(&mut harness, panned);
    let schematic = harness.state().session().project().unwrap();
    assert_eq!(schematic.symbols()[0].position, pcbcraft_engine::schematic::Point { x_nm: 4 * pcbcraft_engine::schematic::GRID_NM, y_nm: 0 });
}

#[test]
fn zoom_to_fit_on_an_empty_sheet_restores_the_default_origin() {
    let mut harness = harness();
    new_project(&mut harness, "empty");
    harness.hover_at(egui::pos2(600.0, 400.0));
    harness.run();
    harness.event(egui::Event::Zoom(2.0));
    harness.run();
    choose(&mut harness, "View", "Zoom to Fit");
    assert_eq!(harness.state().session().revision(), 1);
    choose(&mut harness, "Place", "Place Symbol…");
    select_symbol(&mut harness, "Resistor");
    click_canvas(&mut harness);
    assert_eq!(harness.state().session().project().unwrap().symbols()[0].position, pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 });
}

/// Place a resistor and LED through the canvas so their exact terminal positions are known.
fn wiring_project(harness: &mut Harness<'_, PcbApp>) {
    new_project(harness, "wiring");
    choose(harness, "Place", "Place Symbol…");
    select_symbol(harness, "Resistor");
    click_canvas(harness);
    choose(harness, "Place", "Place Symbol…");
    select_symbol(harness, "LED");
    harness.get_by_label("Place LED · Esc to cancel");
    click_canvas_offset(harness, egui::vec2(203.2, 60.96));
    assert_eq!(harness.state().session().project().unwrap().symbols()[1].definition.id, "led");
}

#[test]
fn the_wire_tool_runs_the_shared_command_and_is_undoable() {
    let mut harness = harness();
    wiring_project(&mut harness);
    choose(&mut harness, "Place", "Wire");
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    harness.get_by_label("Wire · Click the destination pin");
    assert_eq!(harness.state().session().revision(), 3);
    click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));

    let inspected = harness.state().session().inspect();
    let schematic = &inspected["project"]["schematic"];
    assert_eq!(
        schematic["wires"],
        json!([{
            "id": "wire-4",
            "from": {"symbol": "symbol-2", "terminal": "2"},
            "to": {"symbol": "symbol-3", "terminal": "1"},
        }])
    );
    assert_eq!(schematic["netlist"]["nets"].as_array().unwrap().len(), 1);
    choose(&mut harness, "Edit", "Undo");
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    choose(&mut harness, "Edit", "Redo");
    assert_eq!(&harness.state().session().inspect()["project"]["schematic"], schematic);
}

/// At a compact width, ribbon actions remain reachable and the right card leaves canvas room.
#[test]
fn compact_ribbon_places_and_wires_symbols_with_a_docked_inspector() {
    let mut harness = harness();
    harness.set_size(egui::vec2(400.0, 600.0));
    harness.run();
    new_project(&mut harness, "compact");

    for (name, x) in [("Resistor", -81.28), ("LED", 81.28)] {
        harness.get_by_label("Add symbol (A)").click();
        harness.run();
        select_symbol(&mut harness, name);
        click_canvas_offset(&mut harness, egui::vec2(x, 0.0));
    }
    harness.get_by_label("Draw wire (W)").click();
    harness.run();
    for x in [-30.48, 30.48] {
        click_canvas_offset(&mut harness, egui::vec2(x, 0.0));
    }
    let wired = harness.state().session().inspect();
    assert_eq!(wired["project"]["schematic"]["wires"].as_array().unwrap().len(), 1);

    let full_width = harness.get_by_label("Schematic canvas").rect().width();
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    harness.get_by_label("R1.2, D1.1");
    let canvas = harness.get_by_label("Schematic canvas").rect();
    let close = harness.get_by_label("Close");
    assert!(canvas.width() >= 0.6 * full_width - 1.0, "canvas: {canvas:?}");
    assert!(close.rect().left() >= canvas.right());
    close.click();
    harness.run();
    assert!(harness.query_by_label("R1.2, D1.1").is_none());
    assert_eq!(harness.state().session().inspect(), wired);
}

#[test]
fn wiring_ignores_empty_space_and_escape_discards_the_preview() {
    let mut harness = harness();
    wiring_project(&mut harness);
    let before = harness.state().session().inspect();
    harness.key_press(Key::W);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    click_canvas_offset(&mut harness, egui::vec2(100.0, 36.0));
    harness.get_by_label("Wire · Click the destination pin");
    harness.key_press(Key::Escape);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
    assert_eq!(harness.state().session().inspect(), before);
}

/// Open the context menu during an active wire draft with a real secondary click.
fn open_wire_context(harness: &mut Harness<'_, PcbApp>) {
    let point = canvas_center(harness) + egui::vec2(80.0, 26.0);
    harness.hover_at(point);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.run();
    }
    harness.get_by_label("Cancel");
}

#[test]
fn dismissing_the_wire_menu_over_a_pin_does_not_commit() {
    let mut harness = harness();
    wiring_project(&mut harness);
    harness.key_press(Key::W);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    open_wire_context(&mut harness);
    let destination = canvas_center(&harness) + egui::vec2(152.4, 60.96);
    click_canvas_at(&mut harness, destination);
    assert!(harness.query_by_label("Cancel").is_none());
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    harness.get_by_label("Wire · Click the destination pin");
    click_canvas_at(&mut harness, destination);
    assert_eq!(harness.state().session().project().unwrap().wires().len(), 1);
}

/// Both cancellation paths discard the draft, with no revision or undo change.
#[test]
fn the_wire_menu_cancel_button_and_escape_both_discard_the_preview() {
    let mut harness = harness();
    wiring_project(&mut harness);
    let before = harness.state().session().inspect();
    for escape in [false, true] {
        harness.key_press(Key::W);
        harness.run();
        click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
        open_wire_context(&mut harness);
        if escape {
            harness.key_press(Key::Escape);
        } else {
            harness.get_by_label("Cancel").click();
        }
        harness.run();
        assert!(harness.query_by_label("Cancel").is_none());
        assert!(harness.query_by_label("Wire · Click the destination pin").is_none());
        click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
        assert_eq!(harness.state().session().inspect(), before);
    }
}
