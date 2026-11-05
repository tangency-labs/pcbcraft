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
    choose(&mut harness, "Place", "Place Resistor");
    assert!(harness.state().session().project().unwrap().resistors().is_empty());
    harness.get_by_label("Place resistor · Esc to cancel");

    // A click near the center snaps to the schematic origin.
    click_canvas(&mut harness);
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "blinky"})).unwrap();
    session.execute(PLACE_RESISTOR, &json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1})).unwrap();
    assert_eq!(harness.state().session().inspect(), session.inspect());
    click_canvas(&mut harness);
    assert_eq!(harness.state().session().inspect(), session.inspect());
    choose(&mut harness, "Edit", "Undo");
    assert!(harness.state().session().project().unwrap().resistors().is_empty());
    choose(&mut harness, "Edit", "Redo");
    assert_eq!(harness.state().session().project().unwrap().resistors(), session.project().unwrap().resistors());
}

#[test]
fn escape_and_opening_a_dialog_cancel_the_placement_preview() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("Place resistor · Esc to cancel");
    harness.key_press(Key::Escape);
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().resistors().is_empty());

    harness.key_press(Key::A);
    harness.run();
    choose(&mut harness, "File", "Rename Project…");
    harness.get_by_label("Cancel").click();
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().resistors().is_empty());
}

fn click_canvas(harness: &mut Harness<'_, PcbApp>) {
    let point = egui::pos2(502.0, 354.0);
    harness.hover_at(point);
    harness.run();
    harness.drag_at(point);
    harness.run();
    harness.drop_at(point);
    harness.run();
}
