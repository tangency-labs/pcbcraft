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

/// Open a sheet Properties selector by its label, then pick one of its rows.
fn choose(h: &mut Harness<'_, PcbApp>, selector: &str, row: &str) {
    h.get_by_role_and_label(Role::ComboBox, selector).click();
    h.run();
    h.get_by_label(row).click();
    h.run();
}

/// Properties shows the sheet's paper size, orientation and frame. Each pick runs the same
/// `sheet.set_page` or `sheet.set_template` edit as the Browser menu, one undo step each, and
/// picking the current value changes nothing.
#[test]
fn sheet_paper_and_frame_are_shown_and_edited_in_properties() {
    use pcbcraft_engine::sheets::page::{Orientation, Paper};
    let mut h = fixture();
    let before = h.state().session.snapshot().unwrap();
    let page = |h: &Harness<'_, PcbApp>| {
        let page = h.state().session.project().unwrap().page().unwrap();
        (page.paper(), page.orientation(), page.template().map(|frame| frame.title().to_owned()))
    };
    let initial = page(&h);
    assert_eq!((initial.0, initial.1), (Paper::A4, Orientation::Landscape));
    let frame = initial.2.clone().expect("new sheets start with the default frame");
    for (selector, value) in [("Size", "A4"), ("Orientation", "Landscape"), ("Frame", frame.as_str())] {
        let shown = h.get_by_role_and_label(Role::ComboBox, selector).accesskit_node().value();
        assert_eq!(shown.as_deref(), Some(value), "{selector} shows the current choice");
    }

    let revision = h.state().session.revision();
    choose(&mut h, "Size", "A4");
    assert_eq!(h.state().session.revision(), revision, "picking the current size is not an edit");

    choose(&mut h, "Size", "A3");
    assert_eq!(page(&h), (Paper::A3, Orientation::Landscape, Some(frame.clone())));
    assert_eq!(h.state().session.revision(), revision + 1);
    choose(&mut h, "Orientation", "Portrait");
    assert_eq!(page(&h), (Paper::A3, Orientation::Portrait, Some(frame.clone())));
    choose(&mut h, "Frame", "None");
    assert_eq!(page(&h), (Paper::A3, Orientation::Portrait, None));
    assert_eq!(h.state().session.project().unwrap().symbols(), before.symbols(), "drawing stays in place");

    // The Browser's Paper menu reflects the choice made in Properties.
    let row = h.get_by_role_and_label(Role::Button, "Main").rect().center();
    h.hover_at(row);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: row, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Paper");
    assert_eq!(h.get_by_role_and_label(Role::RadioButton, "A3").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    h.key_press(Key::Escape);
    h.run();

    for _ in 0..3 {
        h.state_mut().run("edit.undo", &Value::Null);
    }
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "three picks undo in three steps");
}
