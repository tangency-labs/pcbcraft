//! Real Rules controls use the same revision-checked engine commands as external clients.
use super::*;
use crate::tests::click;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::persistence::{decode_documents, encode_documents};

/// A schematic-only session makes accidental physical-board prerequisites observable.
fn harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.start_new_project_in(crate::Workspace::Schematic);
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Replace text through actual focus, select-all and typing events.
fn replace(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
    h.get_by_role_and_label(egui::Role::TextInput, label).scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, label).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.get_by_role_and_label(egui::Role::TextInput, label).type_text(text);
    h.run();
}

/// Cancel, invalid dimensions, unit mixing and exact untouched precision work without a board.
#[test]
fn class_form_controls_create_edit_cancel_and_exact_units() {
    for width in [1200., 700., 400.] {
        let mut h = harness(width);
        h.state_mut().preferences.units = units::Unit::Mil;
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.state_mut().begin_rules();
        h.run();
        click(&mut h, "New class");
        replace(&mut h, "Class name", "Cancelled");
        click(&mut h, "Cancel");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.state_mut().begin_rules();
        h.run();
        click(&mut h, "New class");
        replace(&mut h, "Class name", "Power");
        assert_eq!(h.get_by_role_and_label(egui::Role::TextInput, "Copper clearance value").value().as_deref(), Some("Not specified"));
        assert!(h.get_by_role_and_label(egui::Role::TextInput, "Copper clearance value").accesskit_node().is_disabled());
        let diagram = h.get_by_role_and_label(egui::Role::Image, "Copper clearance diagram: Space between copper edges on different nets.");
        let field = h.get_by_role_and_label(egui::Role::TextInput, "Copper clearance value");
        assert!(field.rect().width() <= 140.1 && field.rect().left() >= diagram.rect().right());
        click(&mut h, "Copper clearance");
        replace(&mut h, "Copper clearance value", "invalid");
        assert!(h.get_by_role_and_label(egui::Role::Button, "Save class").accesskit_node().is_disabled());
        replace(&mut h, "Copper clearance value", "0.350001 mm");
        click(&mut h, "Preferred track width");
        replace(&mut h, "Preferred track width value", "20 mil");
        let revision = h.state().session.revision();
        click(&mut h, "Save class");
        assert_eq!(h.state().session.revision(), revision + 1);
        let project = h.state().session.project().unwrap();
        assert!(project.board_target().is_none());
        let class = project.net_classes().definitions.first().unwrap();
        let id = class.id;
        assert_eq!(class.rules[&Property::ClearanceNm], 350001);
        assert_eq!(class.rules[&Property::PreferredTrackWidthNm], 508000);
        assert_eq!(class.rules.len(), 2);
        replace(&mut h, "Class name", "Power renamed");
        click(&mut h, "Save class");
        let class = h.state().session.project().unwrap().net_classes().get(id).unwrap();
        assert_eq!(class.rules[&Property::ClearanceNm], 350001);
        assert_eq!(class.revision, 2);
        let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
        click(&mut h, "Cancel");
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(h.state().session.project().unwrap().net_classes().get(id).unwrap().name, "Power");
        h.state_mut().run("edit.redo", &json!({}));
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// A selected wire enters the same page as the global chooser. Assignment is one undoable
/// transaction, and Apply retains a manually chosen target after the source selection clears.
#[test]
fn selected_wire_assignments_share_inspection_and_history() {
    let mut h = harness(1200.);
    let app = h.state_mut();
    for x in [0, 25_400_000] {
        app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let symbols: Vec<_> = app.session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    app.run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":app.session.revision()}),
    );
    app.run(net_classes::CREATE, &json!({"name":"Signal","rules":{"clearance_nm":250000},"expected_revision":app.session.revision()}));
    let class = app.session.project().unwrap().net_classes().definitions[0].id;
    let wire = app.session.project().unwrap().wires()[0].id();
    app.views.active.schematic.selection.wires.insert(wire);
    app.begin_net_rules();
    h.run();
    let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
    assert!(edit.target.is_some());
    let name = "Signal";
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.get_by_role_and_label(egui::Role::CheckBox, name).scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::Role::CheckBox, name).click();
    h.run();
    click(&mut h, "Assign classes");
    assert_eq!(h.state().session.revision(), revision + 1);
    let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
    assert_eq!(edit.original_classes, BTreeSet::from([class]));
    let target = edit.target.clone().unwrap();
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(h.state().session.project().unwrap().inspect_net_classes(&target).unwrap()["bindings"][0]["class"], json!(class));
    click(&mut h, "Cancel");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.state_mut().run("edit.redo", &json!({}));
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    h.state_mut().begin_net_rules();
    h.run();
    h.state_mut().views.active.schematic.selection.wires.clear();
    h.get_by_role_and_label(egui::Role::CheckBox, name).scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::Role::CheckBox, name).click();
    h.run();
    click(&mut h, "Assign classes");
    let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
    assert!(edit.target.is_some());
    assert!(edit.original_classes.is_empty());
}

/// Changed revision or original view authority disables publication without discarding the draft.
#[test]
fn class_form_stale_revision_and_live_mcp_refusal_preserve_state() {
    let mut h = harness(1000.);
    h.state_mut().begin_rules();
    h.run();
    click(&mut h, "New class");
    replace(&mut h, "Class name", "Power");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let request = json!({"method":"engine.execute","params":{"command":net_classes::CREATE,"params":{"name":"Remote","rules":{},"expected_revision":h.state().session.revision()}}});
        let ctx = egui::Context::default();
        let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
        assert!(pcbcraft_mcp::live::call_editor(&mut editor, &request).is_err());
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
    h.state_mut().run("project.rename", &json!({"name":"Concurrent edit"}));
    h.run();
    assert!(h.get_by_role_and_label(egui::Role::Button, "Save class").accesskit_node().is_disabled());
    click(&mut h, "Cancel");
    assert!(h.state().session.project().unwrap().net_classes().definitions.is_empty());
}

/// Target transitions invalidate a disposable draft even at the same project revision.
#[test]
fn class_form_original_view_target_cannot_be_replaced_by_active_focus() {
    let mut h = harness(1200.);
    h.state_mut().begin_rules();
    h.run();
    click(&mut h, "New class");
    replace(&mut h, "Class name", "Power");
    let before = h.state().session.snapshot().unwrap();
    let app = h.state_mut();
    app.views.active.switch(crate::Workspace::Board, &app.session);
    h.run();
    assert!(h.get_by_role_and_label(egui::Role::Button, "Save class").accesskit_node().is_disabled());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
}

/// Signal/Power authored through the shared forms reach PCB contact policy and DRC, survive
/// Update PCB and a split editor, and reopen with the same UUID/revision/binding sources.
#[test]
fn two_classes_reach_pcb_update_split_and_native_reopen() {
    use pcbcraft_engine::{CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL};
    let mut h = harness(1400.);
    let app = h.state_mut();
    app.run("project.new", &json!({"create_board":true}));
    let mut symbols = Vec::new();
    for index in 0..2 {
        app.run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*10_000_000,"y_nm":0,"expected_revision":app.session.revision()}));
        let symbol = app.session.project().unwrap().symbols().last().unwrap().id();
        app.run(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":index*10_160_000,"y_nm":0,"expected_revision":app.session.revision()}));
        symbols.push(symbol);
    }
    for pin in ["1", "2"] {
        app.run(CONNECT_PINS, &json!({"from":{"symbol":symbols[0],"terminal":pin},"to":{"symbol":symbols[1],"terminal":pin},"expected_revision":app.session.revision()}));
    }
    app.run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":app.session.revision()}));
    app.run(pcbcraft_engine::board::update::INITIALIZE_BASELINE, &json!({"expected_revision":app.session.revision()}));
    assert_eq!(app.message, None);
    let wires: Vec<_> = app.session.project().unwrap().wires().iter().map(|wire| wire.id()).collect();
    h.run();
    let mut expected = Vec::new();
    for (index, (name, clearance, width)) in [("Signal", "0.25 mm", "0.3 mm"), ("Power", "0.5 mm", "0.6 mm")].into_iter().enumerate() {
        h.state_mut().begin_rules();
        h.run();
        click(&mut h, "New class");
        replace(&mut h, "Class name", name);
        click(&mut h, "Copper clearance");
        replace(&mut h, "Copper clearance value", clearance);
        click(&mut h, "Preferred track width");
        replace(&mut h, "Preferred track width value", width);
        click(&mut h, "Save class");
        let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
        let id = edit.selected_class.unwrap();
        click(&mut h, "Cancel");
        h.state_mut().views.active.schematic.selection.wires = BTreeSet::from([wires[index].clone()]);
        h.state_mut().begin_net_rules();
        h.run();
        h.get_by_role_and_label(egui::Role::CheckBox, name).scroll_to_me();
        h.run();
        h.get_by_role_and_label(egui::Role::CheckBox, name).click();
        h.run();
        click(&mut h, "Assign classes");
        let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
        let target = edit.target.clone().unwrap();
        let inspected = h.state().session.project().unwrap().inspect_net_classes(&target).unwrap();
        assert_eq!(inspected["resolution"]["classes"], json!([id]));
        expected.push((target, inspected));
        click(&mut h, "Cancel");
    }
    // Update actual component metadata while preserving the previously accepted board.
    let app = h.state_mut();
    app.run(
        pcbcraft_engine::SET_SYMBOL_FIELDS,
        &json!({"symbol":symbols[0],"reference":"R1","value":"2k","expected_revision":app.session.revision()}),
    );
    app.begin_board_update(app.views.active.id()).unwrap();
    h.run();
    click(&mut h, "Update PCB");
    assert_eq!(h.state().message, None);
    if !matches!(h.state().dialog, Dialog::None) {
        click(&mut h, "Close");
    }
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let project = h.state().session.project().unwrap();
    let board = project.board_view(project.board_target().unwrap()).unwrap();
    for (target, inspected) in &expected {
        assert_eq!(&project.inspect_net_classes(target).unwrap(), inspected);
        let class: Uuid = serde_json::from_value(inspected["resolution"]["classes"][0].clone()).unwrap();
        assert!(
            board
                .footprints()
                .flat_map(|footprint| footprint.pads)
                .any(|pad| { board.route_rules_at(pad.center, None).unwrap().is_some_and(|policy| policy.resolution.classes.contains(&class)) })
        );
    }
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    for (target, inspected) in expected {
        assert_eq!(reopened.inspect_net_classes(&target).unwrap(), inspected);
    }
    assert_eq!(encode_documents(reopened).unwrap(), saved);
}

/// Static text exposes its displayed value separately from its contextual accessible name.
fn source_shown(h: &Harness<'_, PcbApp>, name: &'static str) -> bool {
    h.query(egui_kittest::kittest::By::new().role(egui::Role::Label).predicate(move |node| node.label().as_deref() == Some(name))).is_some()
}

/// The effective page identifies the actual source even when class/default numbers match,
/// and staged checkbox changes are explicitly previews until the shared command succeeds.
#[test]
fn effective_sources_and_unsaved_previews_are_distinct() {
    let mut h = harness(1200.);
    let app = h.state_mut();
    for x in [0, 25400000] {
        app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let ids: Vec<_> = app.session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    app.run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"from":{"symbol":ids[0],"terminal":"1"},"to":{"symbol":ids[1],"terminal":"1"},"expected_revision":app.session.revision()}),
    );
    app.run(
        net_classes::CREATE,
        &json!({"name":"Weak","rules":{"clearance_nm":100000,"preferred_track_width_nm":250000},"expected_revision":app.session.revision()}),
    );
    let wire = app.session.project().unwrap().wires()[0].id();
    app.views.active.schematic.selection.wires.insert(wire);
    app.begin_net_rules();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.get_by_role_and_label(egui::Role::CheckBox, "Weak").click();
    h.run();
    assert!(h.query_by_label("Unsaved assignment preview").is_some());
    assert!(source_shown(&h, "Preview source Copper clearance: Built-in defaults"));
    assert!(source_shown(&h, "Preview source Preferred track width: Weak"));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Assign classes");
    assert!(h.query_by_label("Unsaved assignment preview").is_none());
    assert!(source_shown(&h, "Active source Copper clearance: Built-in defaults"));
    assert!(source_shown(&h, "Active source Preferred track width: Weak"));
    click(&mut h, "Contributions and identities");
    assert!(h.query_by_label_contains("Overridden · Built-in defaults:").is_some());
}

/// A baseline/class tie is presented as a matching bound, while both exact contributors
/// remain in Details and assignment retains the same resolved clearance.
#[test]
fn equal_clearance_is_labelled_as_matching_the_baseline() {
    let mut h = harness(1200.);
    h.state_mut().preferences.units = units::Unit::Mm;
    let app = h.state_mut();
    for x in [0, 25400000] {
        app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let ids: Vec<_> = app.session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    app.run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"from":{"symbol":ids[0],"terminal":"1"},"to":{"symbol":ids[1],"terminal":"1"},"expected_revision":app.session.revision()}),
    );
    app.run(net_classes::CREATE, &json!({"name":"My constraint","rules":{"clearance_nm":200000},"expected_revision":app.session.revision()}));
    let project = app.session.snapshot().unwrap();
    let id = project.net_classes().definitions[0].id;
    let result = project.resolve_net_classes(&[id], None).unwrap();
    let (summary, names) = effective_sources(&project, &result, Property::ClearanceNm);
    assert_eq!(summary, "Matches built-in minimum");
    assert!(names.contains("Built-in defaults") && names.contains("My constraint"));
    assert_eq!(result.values[&Property::ClearanceNm], Some(200000));
    assert_eq!(result.contribution_statuses(Property::ClearanceNm).filter(|(_, status)| *status == ContributionStatus::Active).count(), 2);
    let wire = project.wires()[0].id();
    app.views.active.schematic.selection.wires.insert(wire);
    app.begin_net_rules();
    h.run();
    h.get_by_role_and_label(egui::Role::CheckBox, "My constraint").click();
    h.run();
    assert!(source_shown(&h, "Preview source Copper clearance: Matches built-in minimum"));
    click(&mut h, "Assign classes");
    assert!(source_shown(&h, "Active source Copper clearance: Matches built-in minimum"));
    click(&mut h, "Contributions and identities");
    assert!(h.query_by_label("Active · Built-in defaults: 0.2 mm").is_some());
    assert!(h.query_by_label_contains("Active · My constraint").is_some());
}
