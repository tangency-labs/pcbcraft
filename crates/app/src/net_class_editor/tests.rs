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

/// A PCB workspace may display a schematic-only project; class authoring must stay available
/// while physical-board actions and the board-default editor remain disabled.
#[test]
fn pcb_rules_entry_does_not_require_a_physical_board() {
    let mut h = harness(1200.);
    let app = h.state_mut();
    app.views.active.switch(crate::Workspace::Board, &app.session);
    h.run();
    crate::tests::click_tab(&mut h, "Analyze");
    assert!(!h.get_by_role_and_label(egui::Role::Button, "Edit net classes, assignments and board defaults.").accesskit_node().is_disabled());
    for label in ["Run design rules check", "Measure distance"] {
        assert!(h.get_by_role_and_label(egui::Role::Button, label).accesskit_node().is_disabled());
    }
    click(&mut h, "Edit net classes, assignments and board defaults.");
    assert!(h.get_by_role_and_label(egui::Role::Button, "Board defaults…").accesskit_node().is_disabled());
    click(&mut h, "New class");
    replace(&mut h, "Class name", "Signal");
    click(&mut h, "Save class");
    assert_eq!(h.state().session.project().unwrap().net_classes().definitions[0].name, "Signal");
    assert!(h.state().session.project().unwrap().board_target().is_none());
}

/// Mixed-unit GUI edits match canonical MCP updates without a board, including exact untouched
/// values, stale MCP rejection, cancellation and native history.
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
        let mut canonical = pcbcraft_mcp::Headless::with_project(h.state().session.snapshot().unwrap());
        use pcbcraft_mcp::Backend;
        let canonical_revision = canonical.call("project.inspect", json!({})).unwrap()["revision"].clone();
        replace(&mut h, "Class name", "Power renamed");
        assert_eq!(
            h.get_by_role_and_label(egui::Role::TextInput, "Class name").value().as_deref(),
            Some("Power renamed"),
            "{width}: retain focus as discard availability changes"
        );
        click(&mut h, "Save class");
        let class = h.state().session.project().unwrap().net_classes().get(id).unwrap();
        assert_eq!(class.rules[&Property::ClearanceNm], 350001);
        assert_eq!(class.revision, 2);
        let params = json!({"command":net_classes::UPDATE,"inspect":false,"params":{
            "class":id,"name":"Power renamed","rules":{"clearance_nm":350001,"preferred_track_width_nm":508000},
            "expected_revision":canonical_revision}});
        canonical.call("engine.execute", params.clone()).unwrap();
        let canonical_saved = canonical.call("project.inspect", json!({})).unwrap();
        assert_eq!(canonical_saved["project"], h.state().session.inspect()["project"], "mixed-unit GUI values match canonical MCP dimensions");
        assert!(canonical.call("engine.execute", params).is_err(), "canonical MCP rejects the earlier revision");
        assert_eq!(canonical.call("project.inspect", json!({})).unwrap(), canonical_saved);
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
/// Update PCB and a split editor, and match live/headless MCP inspection and native reopen.
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
    let mut headless = pcbcraft_mcp::Headless::with_project(h.state().session.snapshot().unwrap());
    use pcbcraft_mcp::Backend;
    for (target, inspected) in &expected {
        let params = json!({"command":pcbcraft_engine::nets::INSPECT_CLASSES,"params":{"target":target},"inspect":false});
        assert_eq!(headless.call("engine.execute", params.clone()).unwrap()["net_assignments"], *inspected);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ctx = h.ctx.clone();
            let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
            let live = pcbcraft_mcp::live::call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap();
            assert_eq!(live["net_assignments"], *inspected, "live/headless queries retain bindings, values and source identities");
        }
    }
    let project = h.state().session.project().unwrap();
    let classes: Vec<Uuid> =
        expected.iter().map(|(_, inspected)| serde_json::from_value(inspected["resolution"]["classes"][0].clone()).unwrap()).collect();
    let conflicting = project.resolve_net_classes(&classes, project.board_target()).unwrap();
    assert!(!conflicting.conflicts.is_empty());
    let params = json!({"command":net_classes::RESOLVE,"params":{"classes":classes,"board":project.board_target().unwrap().board},"inspect":false});
    assert_eq!(headless.call("engine.execute", params.clone()).unwrap()["resolution"], json!(conflicting));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = h.ctx.clone();
        let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
        let live = pcbcraft_mcp::live::call_editor(&mut editor, &json!({"method":"engine.execute","params":params})).unwrap();
        assert_eq!(live["resolution"], json!(conflicting), "conflict and contribution data matches the Rules resolver");
    }
    for (target, inspected) in expected {
        assert_eq!(reopened.inspect_net_classes(&target).unwrap(), inspected);
    }
    assert_eq!(encode_documents(reopened).unwrap(), saved);
}

/// Static text exposes its displayed value separately from its contextual accessible name.
fn source_shown(h: &Harness<'_, PcbApp>, name: &'static str) -> bool {
    h.query(egui_kittest::kittest::By::new().role(egui::Role::Label).predicate(move |node| node.label().as_deref() == Some(name))).is_some()
}

/// Status rows carry an accessible name beside their visible text, so they are found as
/// labelled Label nodes, as the effective-source cells are.
fn status_shown(h: &Harness<'_, PcbApp>, name: &str) -> bool {
    let name = name.to_owned();
    h.query(egui_kittest::kittest::By::new().role(egui::Role::Label).predicate(move |node| node.label().as_deref() == Some(name.as_str()))).is_some()
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

/// A designer declares a working voltage on a selected schematic net in Rules, reads its
/// source and unverified status, and sees a needs-review notice after the reference net is
/// deleted. Saving a new value keeps the retained reference. Each save is one Undo step and
/// the shared `electrical_intent.inspect` report survives native save/reopen.
#[test]
fn working_voltage_page_authors_inspects_and_flags_review() {
    use pcbcraft_engine::{net_labels, nets::intent};
    let mut h = harness(1200.);
    let app = h.state_mut();
    for (name, x) in [("TEST_SUPPLY", 0), ("TEST_RETURN", 25_400_000)] {
        app.run(net_labels::ADD, &json!({"name":name,"style":"local","x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let project = app.session.project().unwrap();
    let labels: Vec<_> = project.net_labels().iter().map(|label| label.object_id()).collect();
    app.views.active.schematic.selection.net_labels.insert(labels[0].clone());
    app.begin_net_rules();
    h.run();
    h.get_by_role_and_label(egui::Role::Button, "Working voltage").click();
    h.run();
    let target = {
        let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
        assert_eq!(edit.page, Page::Voltage);
        let draft = edit.voltage.as_ref().expect("the selected net is preselected");
        edit.owners[draft.owner].target.clone()
    };
    assert_eq!(target["kind"], "net");
    assert!(status_shown(&h, "Requirement source: Not declared"));
    click(&mut h, "Declare working voltage");
    replace(&mut h, "Working voltage value", "48");
    let revision = h.state().session.revision();
    click(&mut h, "Save working voltage");
    assert_eq!(h.state().session.revision(), revision + 1);
    let report = h.state().session.project().unwrap().inspect_electrical_intent(&target).unwrap();
    let earth = json!({"magnitude":{"value":"48","unit":"V"},"basis":"dc","reference":{"kind":"earth"}});
    assert_eq!(report["intent"]["working_voltage"], earth);
    assert_eq!(report["requirement"], json!({"source":"authored_on_net","verification":"unverified","review":"current","reasons":[]}));
    assert!(status_shown(&h, "Requirement source: Entered on this net"));
    assert!(status_shown(&h, "Verification: Unverified: no ERC, DRC or solver checks this value yet"));
    assert!(status_shown(&h, "Review: Current"));
    let mut canonical = pcbcraft_mcp::Headless::with_project(h.state().session.snapshot().unwrap());
    use pcbcraft_mcp::Backend;
    let shared = canonical.call("engine.execute", json!({"command":intent::INSPECT,"inspect":false,"params":{"version":1,"target":target}})).unwrap();
    assert_eq!((&shared["intent"], &shared["requirement"]), (&report["intent"], &report["requirement"]));
    click(&mut h, "Cancel");

    let reference = h.state().session.project().unwrap().inspect_net_registry().unwrap()["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|net| net["name"] == "TEST_RETURN")
        .unwrap()["id"]
        .clone();
    let mut declared = earth.clone();
    declared["reference"] =
        json!({"kind":"net","address":{"project":target["address"]["project"],"scope":target["address"]["scope"],"net":reference}});
    let app = h.state_mut();
    app.run(intent::SET, &json!({"version":1,"expected_revision":app.session.revision(),"target":target,"intent":{"working_voltage":declared}}));
    app.run(net_labels::DELETE, &json!({"id":labels[1],"expected_revision":app.session.revision()}));
    assert!(app.message.is_none());
    app.begin_net_rules();
    h.run();
    h.get_by_role_and_label(egui::Role::Button, "Working voltage").click();
    h.run();
    assert!(h.query_by_label("Needs review").is_some());
    assert!(h.query_by_label("The reference net no longer exists in the design.").is_some());
    replace(&mut h, "Working voltage value", "50");
    click(&mut h, "Save working voltage");
    let report = h.state().session.project().unwrap().inspect_electrical_intent(&target).unwrap();
    let mut kept = declared.clone();
    kept["magnitude"]["value"] = json!("50");
    assert_eq!(report["intent"]["working_voltage"], kept, "editing the value keeps the retained reference");
    assert_eq!(report["requirement"]["review"], "needs_review");
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap().inspect_electrical_intent(&target).unwrap();
    assert_eq!(reopened, report);
    click(&mut h, "Cancel");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(h.state().session.project().unwrap().inspect_electrical_intent(&target).unwrap()["intent"]["working_voltage"], declared);
}

/// A declaration whose own net was deleted stays reachable in Rules as a former net. Its
/// review reason is shown, its value cannot be redeclared, and clearing it is one Undo step.
#[test]
fn working_voltage_page_reaches_and_clears_a_former_net() {
    use pcbcraft_engine::{net_labels, nets::intent};
    let mut h = harness(1200.);
    let app = h.state_mut();
    app.run(net_labels::ADD, &json!({"name":"TEST_SUPPLY","style":"local","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    let project = app.session.project().unwrap();
    let label = project.net_labels()[0].object_id();
    let registry = project.inspect_net_registry().unwrap();
    let net = registry["nets"].as_array().unwrap().iter().find(|net| net["name"] == "TEST_SUPPLY").unwrap()["id"].clone();
    let target = json!({"kind":"net","address":{"project":project.id(),"scope":registry["scope"],"net":net}});
    let declared = json!({"magnitude":{"value":"48","unit":"V"},"basis":"dc","reference":{"kind":"earth"}});
    app.run(intent::SET, &json!({"version":1,"expected_revision":app.session.revision(),"target":target,"intent":{"working_voltage":declared}}));
    app.run(net_labels::DELETE, &json!({"id":label,"expected_revision":app.session.revision()}));
    assert!(app.message.is_none());
    app.begin_rules();
    h.run();
    h.get_by_role_and_label(egui::Role::Button, "Working voltage").click();
    h.run();
    let former = format!("Former net · {}", &net.as_str().unwrap()[..8]);
    {
        let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
        let owner = edit.owners.iter().find(|owner| owner.target == target).expect("the former net is listed");
        assert!(owner.historical);
        assert_eq!(owner.label, former);
    }
    h.get_by_role_and_label(egui::Role::ComboBox, "Declared on").click();
    h.run();
    click(&mut h, &former);
    assert!(h.query_by_label("Needs review").is_some());
    assert!(h.query_by_label("This net no longer exists in the design.").is_some());
    assert!(h.get_by_role_and_label(egui::Role::TextInput, "Working voltage value").accesskit_node().is_disabled());
    let revision = h.state().session.revision();
    click(&mut h, "Declare working voltage");
    click(&mut h, "Save working voltage");
    assert_eq!(h.state().session.revision(), revision + 1);
    let report = h.state().session.project().unwrap().inspect_electrical_intent(&target).unwrap();
    assert_eq!((&report["intent"]["working_voltage"], &report["requirement"]), (&Value::Null, &Value::Null));
    click(&mut h, "Cancel");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(h.state().session.project().unwrap().inspect_electrical_intent(&target).unwrap()["intent"]["working_voltage"], declared);
}

/// The reference picker offers only named nets, as `electrical_intent.set` requires, while an
/// unnamed saved net can still own a declaration.
#[test]
fn working_voltage_references_offer_only_named_nets() {
    use pcbcraft_engine::net_labels;
    let mut h = harness(1200.);
    let app = h.state_mut();
    let mut pins = Vec::new();
    for x in [0, 25_400_000] {
        app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":50_800_000,"expected_revision":app.session.revision()}));
        pins.push(json!({"symbol":app.session.project().unwrap().symbols().last().unwrap().id(),"terminal":"1"}));
    }
    app.run(pcbcraft_engine::CONNECT_PINS, &json!({"from":pins[0],"to":pins[1],"expected_revision":app.session.revision()}));
    app.run(pcbcraft_engine::nets::PROMOTE, &json!({"anchor":pins[0],"expected_revision":app.session.revision()}));
    app.run(net_labels::ADD, &json!({"name":"TEST_RETURN","style":"local","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    assert!(app.message.is_none());
    let registry = app.session.project().unwrap().inspect_net_registry().unwrap();
    let rows = registry["nets"].as_array().unwrap();
    let id = |named: bool| -> Uuid {
        let row = rows.iter().find(|row| !row["id"].is_null() && row["names"].as_array().unwrap().is_empty() != named).unwrap();
        Uuid::parse_str(row["id"].as_str().unwrap()).unwrap()
    };
    let (unnamed, named) = (id(false), id(true));
    app.begin_rules();
    h.run();
    h.get_by_role_and_label(egui::Role::Button, "Working voltage").click();
    h.run();
    let Dialog::Rules(edit) = &h.state().dialog else { panic!("rules") };
    assert!(edit.owners.iter().any(|owner| owner.net == Some(unnamed) && !owner.historical));
    assert_eq!(edit.references.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![named]);
}
