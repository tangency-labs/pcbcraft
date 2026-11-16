//! Properties shows the current sheet's parameters while nothing is selected.
use super::*;
use egui::accesskit::Role;

/// A new project with Properties open and nothing selected.
fn fixture() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_at_origin(&mut h);
    h.state_mut().properties_open = true;
    h.run();
    h
}

/// Type into a field and leave it with Enter, as a person does.
fn enter(h: &mut Harness<'_, PcbApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    if value.is_empty() {
        h.key_press(Key::Backspace);
    } else {
        h.get_by_role_and_label(Role::TextInput, label).type_text(value);
    }
    h.key_press(Key::Enter);
    h.run();
}

/// The sheet's own Title value as stored.
fn title(h: &Harness<'_, PcbApp>) -> Option<String> {
    let project = h.state().session.project().unwrap();
    project.sheet_parameters(project.sheet_target()).unwrap().get("Title").cloned()
}

/// Editing Title submits one undoable `parameter.set`; clearing it returns to the sheet name.
#[test]
fn editing_a_sheet_parameter_is_one_undo_step() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    enter(&mut h, "Title", "Power supply");
    assert_eq!(title(&h).as_deref(), Some("Power supply"));
    assert_eq!(h.state().session.revision(), revision + 1);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.resolve_text(Some(project.sheet_target()), "@sheet.Title").text, "Power supply");
    enter(&mut h, "Title", "");
    assert_eq!(title(&h), None);
    h.state_mut().session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(title(&h).as_deref(), Some("Power supply"));
}

/// A new name entered under the sheet group adds an empty sheet parameter.
#[test]
fn a_new_name_adds_a_parameter() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    h.get_all_by_role_and_label(Role::TextInput, "New").next().unwrap().click();
    h.run();
    h.get_all_by_role_and_label(Role::TextInput, "New").next().unwrap().type_text("Gain");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    // One step: the new row must not inherit the New field's leaving focus and remove itself.
    assert_eq!(h.state().session.revision(), revision + 1);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.sheet_parameters(project.sheet_target()).unwrap().get("Gain").map(String::as_str), Some(""));
}
