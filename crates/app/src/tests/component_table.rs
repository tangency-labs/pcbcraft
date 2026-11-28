//! #83 component table: one row per physical component, display-only filter and sort, and a
//! grouped field edit previewed and applied through `part.set_fields` as one undo step.
use super::*;
use pcbcraft_engine::{PLACE_RESISTOR, PLACE_UNIT, persistence::encode_documents};

/// Four resistors and a two-unit amplifier with both units placed. R2 and R3 carry a
/// manufacturer; R1 carries a datasheet and a custom field, all set through commands.
fn editor() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.parts.open = false;
    app.properties_open = false;
    app.run("project.new", &json!({}));
    for i in 0..4 {
        app.run(PLACE_RESISTOR, &json!({"x_nm": i * 10_160_000, "y_nm": 0, "expected_revision": app.session.revision()}));
    }
    app.run(
        pcbcraft_engine::library::sources::LOAD,
        &json!({"source":"units","documents":{
            "library.json":include_str!("../../../../examples/native-parts/library.json"),
            "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
        }}),
    );
    let definition = app.session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    let revision = app.session.revision();
    app.place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":25_400_000,"expected_revision":revision}));
    let amplifier = app.session.project().unwrap().symbols().last().unwrap().component_id();
    app.run(PLACE_UNIT, &json!({"component":amplifier,"unit":"b","x_nm":25_400_000,"y_nm":25_400_000,"expected_revision":app.session.revision()}));
    let ids: Vec<String> = app.session.project().unwrap().components().map(|part| part.id()).collect();
    let maker = json!({"manufacturer":{"name":"Manufacturer","value":"Old Maker"}});
    app.run(
        "part.set_fields",
        &json!({"edits":[
            {"component":ids[0],"fields":{"datasheet":{"name":"Datasheet","value":"https://example.com/r.pdf"},
                "6f9a2a3c-0c6b-4a55-9d2e-2b1d7f0c1a11":{"name":"Tolerance","value":"1%"}}},
            {"component":ids[1],"fields":maker},
            {"component":ids[2],"fields":maker},
        ],"expected_revision":app.session.revision()}),
    );
    assert!(app.message.is_none(), "{:?}", app.message);
    for view in app.views.all_mut() {
        view.filters_open = false;
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Open the table from Analyze > Reports, where the sibling review tools live.
fn open(h: &mut Harness<'_, PcbApp>) {
    click(h, "Analyze");
    click(h, "Review and edit component fields in a table");
}

/// Click a table row by its reference, with real modifier state for Shift and Ctrl.
fn row(h: &mut Harness<'_, PcbApp>, reference: &str, modifiers: Modifiers) {
    let at = h.get_by_label(reference).rect().center();
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

/// Choose the grouped edit's field. The popup option is drawn after the same-named header.
fn field(h: &mut Harness<'_, PcbApp>, name: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Field").click();
    h.run();
    h.query_all_by_label(name).last().unwrap().click();
    h.run();
}

/// Type the new value into the grouped edit.
fn value(h: &mut Harness<'_, PcbApp>, text: &str) {
    h.get_by_label("New value").click();
    h.run();
    h.get_by_label("New value").type_text(text);
    h.run();
}

/// The authored field of the component with this reference, or None when absent.
fn field_of(h: &Harness<'_, PcbApp>, reference: &str, id: &str) -> Option<String> {
    let project = h.state().session.project().unwrap();
    let part = project.components().find(|part| part.reference() == reference)?;
    part.metadata.fields.get(id).map(|field| field.value.clone())
}

/// Saved bytes of the current project.
fn bytes(h: &Harness<'_, PcbApp>) -> std::collections::BTreeMap<String, Vec<u8>> {
    encode_documents(h.state().session.snapshot().unwrap()).unwrap()
}

/// One field set on three selected parts is previewed with every change, applied as one undo
/// step, and leaves every other field and every unselected part untouched.
#[test]
fn a_field_edit_across_several_parts_is_previewed_and_applied_as_one_undo_step() {
    let mut h = editor();
    let before = bytes(&h);
    let revision = h.state().session.revision();
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    row(&mut h, "R3", Modifiers::SHIFT);
    h.get_by_label("5 components · 5 shown · 3 selected");
    field(&mut h, "Manufacturer");
    value(&mut h, "Acme");
    click(&mut h, "Preview change to 3 selected");
    h.get_by_label("3 field changes on 3 components");
    assert_eq!(h.query_all_by_label("Old Maker").count(), 4, "two table cells and two old values in the preview");
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(bytes(&h), before);
    click(&mut h, "Apply");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(h.state().session.revision(), revision + 1);
    for reference in ["R1", "R2", "R3"] {
        assert_eq!(field_of(&h, reference, "manufacturer").as_deref(), Some("Acme"));
    }
    assert_eq!(field_of(&h, "R1", "datasheet").as_deref(), Some("https://example.com/r.pdf"));
    assert_eq!(field_of(&h, "R1", "6f9a2a3c-0c6b-4a55-9d2e-2b1d7f0c1a11").as_deref(), Some("1%"));
    assert_eq!(field_of(&h, "R4", "manufacturer"), None);
    let after = bytes(&h);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(bytes(&h), before);
    h.state_mut().run("edit.redo", &json!({}));
    h.run();
    assert_eq!(bytes(&h), after);
}

/// The two-unit amplifier is one row; editing its value changes the one component both units
/// share, in one change.
#[test]
fn a_multi_unit_component_is_one_row_and_edited_once() {
    let mut h = editor();
    open(&mut h);
    h.get_by_label("5 components · 5 shown · 0 selected");
    row(&mut h, "U1", Modifiers::NONE);
    field(&mut h, "Value");
    value(&mut h, "Dual op-amp");
    click(&mut h, "Preview change to 1 selected");
    h.get_by_label("1 field changes on 1 components");
    click(&mut h, "Apply");
    let project = h.state().session.project().unwrap();
    assert_eq!(project.components().count(), 5);
    let units: Vec<_> = project.symbols().iter().filter(|symbol| project.reference(symbol) == "U1").collect();
    assert_eq!(units.len(), 2);
    assert!(units.iter().all(|symbol| project.component(symbol).unwrap().value() == "Dual op-amp"));
}

/// The engine refuses a duplicate reference and an incompatible footprint mapping; the table
/// shows its message naming the component by its reference, offers no Apply and leaves the
/// project and history unchanged.
#[test]
fn duplicate_references_and_incompatible_footprints_are_refused_unchanged() {
    let mut h = editor();
    let before = bytes(&h);
    let revision = h.state().session.revision();
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    row(&mut h, "R2", Modifiers::SHIFT);
    field(&mut h, "Reference");
    value(&mut h, "R9");
    click(&mut h, "Preview change to 2 selected");
    h.get_by_label_contains("another component is already R9");
    let named =
        |h: &Harness<'_, PcbApp>, reference: &str| h.query_by_label_contains(&format!("{reference}: another component is already R9")).is_some();
    assert!(named(&h, "R1") || named(&h, "R2"), "the refusal names the component by its reference");
    assert!(h.query_by_label("Apply").is_none());
    row(&mut h, "U1", Modifiers::NONE);
    field(&mut h, "Footprint");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Footprint").click();
    h.run();
    h.get_by_label_contains("basic-components / resistor-0402").click();
    h.run();
    click(&mut h, "Preview change to 1 selected");
    h.get_by_label_contains("Map every terminal and pad exactly once.");
    assert!(h.query_by_label("Apply").is_none());
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(bytes(&h), before);
}

/// Rows the filter hides are not edited even when selected, and the edited field is the only
/// field that changes on the shown rows.
#[test]
fn a_filtered_selection_edits_only_shown_rows_and_keeps_other_fields() {
    let mut h = editor();
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    row(&mut h, "R3", Modifiers::SHIFT);
    h.get_by_label("Filter components").click();
    h.run();
    h.get_by_label("Filter components").type_text("old maker");
    h.run();
    h.get_by_label("5 components · 2 shown · 2 selected");
    field(&mut h, "Part number");
    value(&mut h, "PN-100");
    click(&mut h, "Preview change to 2 selected");
    click(&mut h, "Apply");
    assert_eq!(field_of(&h, "R1", "manufacturer_part_number"), None);
    for reference in ["R2", "R3"] {
        assert_eq!(field_of(&h, reference, "manufacturer_part_number").as_deref(), Some("PN-100"));
        assert_eq!(field_of(&h, reference, "manufacturer").as_deref(), Some("Old Maker"));
    }
    assert_eq!(field_of(&h, "R1", "datasheet").as_deref(), Some("https://example.com/r.pdf"));
}

/// Cancelling a preview or closing the table changes nothing and adds no history.
#[test]
fn cancel_and_close_leave_the_project_and_history_unchanged() {
    let mut h = editor();
    let before = h.state().session.snapshot().unwrap();
    open(&mut h);
    row(&mut h, "R4", Modifiers::NONE);
    field(&mut h, "Datasheet");
    value(&mut h, "https://example.com/new.pdf");
    click(&mut h, "Preview change to 1 selected");
    click(&mut h, "Cancel");
    assert!(h.query_by_label("Apply").is_none());
    click(&mut h, "Preview change to 1 selected");
    h.key_press(Key::Escape);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.state_mut().run("edit.undo", &json!({}));
    assert_eq!(field_of(&h, "R2", "manufacturer"), None, "undo reverts the fixture's own edit, not a table edit");
}

/// An edit made elsewhere after the preview makes Apply stale: the engine refuses it and the
/// table shows the refusal, leaving the other edit in place.
#[test]
fn a_preview_made_stale_by_another_edit_is_refused() {
    let mut h = editor();
    open(&mut h);
    row(&mut h, "R4", Modifiers::NONE);
    field(&mut h, "Value");
    value(&mut h, "22k");
    click(&mut h, "Preview change to 1 selected");
    h.state_mut().run("project.rename", &json!({"name":"Renamed"}));
    let renamed = h.state().session.snapshot().unwrap();
    click(&mut h, "Apply");
    h.get_by_label_contains("project changed");
    assert!(Arc::ptr_eq(&renamed, &h.state().session.snapshot().unwrap()));
    assert_ne!(h.state().session.project().unwrap().components().nth(3).unwrap().value(), "22k");
}

/// Sorting and filtering change only the display: saving afterwards gives the same bytes. An
/// applied edit survives native Save and Open, and saving the reopened project is byte-identical.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sort_and_filter_keep_saved_bytes_and_an_edit_reopens_identically() {
    let mut h = editor();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("component-table");
    h.state_mut().files.use_stand_in();
    let save = |h: &mut Harness<'_, PcbApp>| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        h.run();
        h.state_mut().files.finish_stand_in(path.clone());
        h.run();
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        pcbcraft_storage::load(&path).unwrap()
    };
    let saved = save(&mut h);
    open(&mut h);
    let top = |h: &Harness<'_, PcbApp>, reference: &str| h.get_by_label(reference).rect().top();
    assert!(top(&h, "R1") < top(&h, "U1"));
    click(&mut h, "Reference");
    click(&mut h, "Reference");
    assert!(top(&h, "U1") < top(&h, "R1"), "Z to A puts U1 first");
    h.get_by_label("Filter components").click();
    h.run();
    h.get_by_label("Filter components").type_text("R");
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(save(&mut h), saved);
    open(&mut h);
    row(&mut h, "R2", Modifiers::NONE);
    field(&mut h, "Value");
    value(&mut h, "4.7k");
    click(&mut h, "Preview change to 1 selected");
    click(&mut h, "Apply");
    h.key_press(Key::Escape);
    h.run();
    let edited = save(&mut h);
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&path, revision);
    h.run();
    assert_eq!(h.state().session.project().unwrap().components().nth(1).unwrap().value(), "4.7k");
    assert_eq!(save(&mut h), edited);
}

/// A footprint chosen for two selected resistors is assigned to both with the catalog's default
/// pin mapping, as one change per part.
#[test]
fn a_footprint_is_assigned_to_selected_parts_with_their_default_mapping() {
    let mut h = editor();
    open(&mut h);
    row(&mut h, "R3", Modifiers::NONE);
    row(&mut h, "R4", Modifiers::SHIFT);
    field(&mut h, "Footprint");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Footprint").click();
    h.run();
    h.get_by_label_contains("basic-components / resistor-0402").click();
    h.run();
    click(&mut h, "Preview change to 2 selected");
    h.get_by_label("2 field changes on 2 components");
    click(&mut h, "Apply");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let rows = h.state().session.project().unwrap().component_rows();
    let assigned: Vec<_> = rows.iter().filter(|row| row.footprint.is_some()).map(|row| row.reference.as_str()).collect();
    assert_eq!(assigned, ["R3", "R4"]);
    assert!(rows.iter().all(|row| row.footprint.as_ref().is_none_or(|footprint| footprint.definition == "resistor-0402")));
}

/// Give the component with this reference one custom field through the shared edit command.
fn set_field(h: &mut Harness<'_, PcbApp>, reference: &str, id: &str, name: &str, value: &str) {
    let component = h.state().session.project().unwrap().components().find(|part| part.reference() == reference).unwrap().id();
    let revision = h.state().session.revision();
    h.state_mut()
        .run("part.set_fields", &json!({"edits":[{"component":component,"fields":{id:{"name":name,"value":value}}}],"expected_revision":revision}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.run();
}

/// Custom field names that differ only in non-ASCII case share one column, every row shows its
/// own value there, and an edit changes the existing field instead of adding a second one.
#[test]
fn custom_fields_differing_only_in_non_ascii_case_share_one_column_and_edit_in_place() {
    let mut h = editor();
    set_field(&mut h, "R2", "8b1e5c2d-3f4a-4b6c-9d7e-1a2b3c4d5e6f", "Étage", "upper");
    set_field(&mut h, "R3", "9c2f6d3e-4a5b-4c7d-8e9f-2b3c4d5e6f70", "étage", "lower");
    open(&mut h);
    assert!(h.query_by_label("lower").is_some(), "R3 shows its own value in the shared column");
    row(&mut h, "R3", Modifiers::NONE);
    field(&mut h, "Étage");
    value(&mut h, "middle");
    click(&mut h, "Preview change to 1 selected");
    click(&mut h, "Apply");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(field_of(&h, "R3", "9c2f6d3e-4a5b-4c7d-8e9f-2b3c4d5e6f70").as_deref(), Some("middle"));
    let part = h.state().session.project().unwrap().components().find(|part| part.reference() == "R3").unwrap();
    assert_eq!(part.metadata.fields.values().filter(|field| field.name.to_lowercase() == "étage").count(), 1, "no second field was added");
}

/// A custom field named like a standard column is its own field choice: editing it changes
/// that field and never writes the standard one.
#[test]
fn a_custom_field_named_like_a_standard_column_is_edited_as_itself() {
    let mut h = editor();
    set_field(&mut h, "R4", "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", "Manufacturer", "Custom Co");
    open(&mut h);
    row(&mut h, "R4", Modifiers::NONE);
    field(&mut h, "Manufacturer");
    value(&mut h, "New Co");
    click(&mut h, "Preview change to 1 selected");
    click(&mut h, "Apply");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(field_of(&h, "R4", "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d").as_deref(), Some("New Co"));
    assert_eq!(field_of(&h, "R4", "manufacturer"), None, "the standard field stays absent");
}

/// A preview is dropped when the filter hides its rows or the selection changes, so Apply can
/// only submit the edit for the rows the preview showed.
#[test]
fn a_preview_is_dropped_when_the_filter_or_selection_changes() {
    let mut h = editor();
    let before = bytes(&h);
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    value(&mut h, "22k");
    click(&mut h, "Preview change to 1 selected");
    h.get_by_label("Apply");
    h.get_by_label("Filter components").click();
    h.run();
    h.get_by_label("Filter components").type_text("R2");
    h.run();
    assert!(h.query_by_label("Apply").is_none(), "a preview of a hidden row is still offered");
    click(&mut h, "Close");
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    value(&mut h, "22k");
    click(&mut h, "Preview change to 1 selected");
    row(&mut h, "R2", Modifiers::NONE);
    assert!(h.query_by_label("Apply").is_none(), "a preview of a deselected row is still offered");
    assert_eq!(bytes(&h), before);
}

/// An empty New value removes an optional field, but a reference or value cannot be empty: the
/// engine refuses it and nothing changes.
#[test]
fn an_empty_reference_or_value_is_refused_unchanged() {
    let mut h = editor();
    let before = bytes(&h);
    open(&mut h);
    row(&mut h, "R1", Modifiers::NONE);
    field(&mut h, "Reference");
    click(&mut h, "Preview change to 1 selected");
    h.get_by_label_contains("reference` must be 1 to");
    assert!(h.query_by_label("Apply").is_none());
    field(&mut h, "Value");
    click(&mut h, "Preview change to 1 selected");
    h.get_by_label_contains("value` must be 1 to");
    assert!(h.query_by_label("Apply").is_none());
    assert_eq!(bytes(&h), before);
}
