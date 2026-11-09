use super::*;
use crate::tests::click;

/// Page navigation uses a rail on wide hosts and the same labelled chooser on compact hosts.
/// Advance bounded frames because an active file spinner intentionally never becomes idle.
fn page(h: &mut Harness<'_, PcbApp>, title: &str) {
    if h.query_by_label("Board setup page").is_some() {
        h.get_by_label("Board setup page").click();
        h.run_steps(2);
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, title).click();
    h.run_steps(2);
}

/// Copper counts all use the schema's single chooser, including the former starter presets.
fn count(h: &mut Harness<'_, PcbApp>, count: u64) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Copper layers").click();
    h.run();
    h.get_by_label(&format!("{count} copper layers")).scroll_to_me();
    h.run();
    click(h, &format!("{count} copper layers"));
}

use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP,
    persistence::{decode_documents, encode_documents},
};

/// Exercise the real board ribbon and modal without a platform window or filesystem writes.
fn harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Board);
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Replace or clear one real field through ordinary editing keys, retaining its accessible name.
fn replace(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).type_text(text);
    h.run();
}

/// Setup creation is an explicit undoable edit; opening/cancelling does not dirty the project.
#[test]
fn board_setup_creation_cancel_and_undo_use_real_controls() {
    for (width, copper) in [(1200.0, 2), (700.0, 8)] {
        let mut h = harness(width);
        let before = h.state().session.inspect();
        click(&mut h, "Board setup");
        click(&mut h, "Cancel");
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Board setup…");
        count(&mut h, copper);
        click(&mut h, "Apply board setup");
        assert!(matches!(h.state().dialog, Dialog::None));
        let setup = h.state().session.project().unwrap().board_setup().unwrap();
        assert_eq!(setup.layers().iter().filter(|l| l.purpose == LayerPurpose::Copper).count(), copper as usize);
        assert_eq!(setup.stackup().strata[0].thickness_nm(), Some(35_000));
        if copper == 2 {
            assert_eq!(setup.laminate_thickness_nm(), Some(1_600_000));
        }
        let expected = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let roundtrip = decode_documents(&expected).unwrap();
        assert_eq!(encode_documents(roundtrip).unwrap(), expected);
        click(&mut h, "Undo");
        assert!(h.state().session.project().unwrap().board_setup().is_none());
        click(&mut h, "Redo");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), expected);
    }
}

/// Long catalogs scroll inside a short modal while Apply/Cancel remain visible and names
/// retain a practical editing width rather than shrinking to the column heading's width.
#[test]
fn short_board_setup_forms_keep_actions_and_names_reachable() {
    let mut h = harness(700.0);
    h.set_size(egui::vec2(700.0, 520.0));
    h.run();
    click(&mut h, "Board setup");
    count(&mut h, 4);
    click(&mut h, "Apply board setup");
    click(&mut h, "Board setup");
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 520.0));
    assert!(viewport.contains_rect(h.get_by_label("Apply board setup").rect()));
    assert!(viewport.contains_rect(h.get_by_label("Cancel").rect()));
    assert!(h.get_by_label("Layer name Top").rect().width() > 200.0);
    page(&mut h, "Physical stackup");
    assert!(h.get_by_label("Top thickness").rect().width() > 100.0);
    assert!(viewport.contains_rect(h.get_by_label("Apply board setup").rect()));
}

/// Invalid names cannot commit, while one valid name/thickness edit retains all IDs and is one undo step.
#[test]
fn board_setup_names_and_stackup_validate_before_atomic_apply() {
    let mut h = harness(1200.0);
    click(&mut h, "Board setup");
    click(&mut h, "Apply board setup");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let ids: Vec<_> = h.state().session.project().unwrap().board_setup().unwrap().layers().iter().map(|l| l.id).collect();
    click(&mut h, "Board setup");
    replace(&mut h, "Layer name Top", "Bottom");
    click(&mut h, "Apply board setup");
    assert!(matches!(h.state().dialog, Dialog::BoardSetup(_)));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    replace(&mut h, "Layer name Top", "Signal");
    page(&mut h, "Physical stackup");
    replace(&mut h, "Signal thickness", "0");
    click(&mut h, "Apply board setup");
    assert_eq!(h.state().session.revision(), revision);
    replace(&mut h, "Signal thickness", "0.035 mm");
    replace(&mut h, "Dielectric 1 thickness", "1.53 mm");
    replace(&mut h, "Bottom thickness", "0.035 mm");
    click(&mut h, "Apply board setup");
    assert!(matches!(h.state().dialog, Dialog::None));
    let setup = h.state().session.project().unwrap().board_setup().unwrap();
    assert_eq!(setup.layers().iter().map(|l| l.id).collect::<Vec<_>>(), ids);
    assert!(setup.layers().iter().any(|l| l.name == "Signal"));
    assert_eq!(setup.laminate_thickness_nm(), Some(1_600_000));
    assert_eq!(h.state().session.revision(), revision + 1);
    click(&mut h, "Undo");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Populated boards explicitly create setup through the same binding-resolution command;
/// existing names remain editable, and layer-membership changes retain the engine's guard.
#[test]
fn populated_board_setup_retains_engine_layer_binding_boundaries() {
    for initialized in [false, true] {
        let mut h = harness(1200.0);
        if initialized {
            click(&mut h, "Board setup");
            click(&mut h, "Apply board setup");
        }
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
        let revision = h.state().session.revision();
        let symbol = h.state().session.project().unwrap().symbols().first().unwrap().id().to_owned();
        h.state_mut().run(
            pcbcraft_engine::PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":revision}),
        );
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        h.run();
        let before = h.state().session.inspect();
        click(&mut h, "Board setup");
        if initialized {
            assert!(h.query_by_label("Add layer").is_some());
        }
        if initialized {
            replace(&mut h, "Layer name Top", "Signal");
            click(&mut h, "Apply board setup");
            assert!(matches!(h.state().dialog, Dialog::None));
            assert!(h.state().session.project().unwrap().board_setup().unwrap().layers().iter().any(|l| l.name == "Signal"));
        } else {
            count(&mut h, 4);
            click(&mut h, "Apply board setup");
            assert!(matches!(h.state().dialog, Dialog::None));
            let after = h.state().session.inspect();
            assert_eq!(after["project"]["board"]["footprints"], before["project"]["board"]["footprints"]);
            assert_eq!(after["project"]["schematic"], before["project"]["schematic"]);
            assert!(h.state().session.project().unwrap().board_setup().is_some());
            click(&mut h, "Undo");
            assert!(h.state().session.project().unwrap().board_setup().is_none());
            click(&mut h, "Redo");
            click(&mut h, "Board setup");
            replace(&mut h, "Layer name Top", "Signal");
            click(&mut h, "Apply board setup");
            assert!(matches!(h.state().dialog, Dialog::None));
        }
    }
}

/// A newly staged technical layer has no model effect until Apply, and a concurrent edit cannot
/// turn its old form into authority over the new project revision.
#[test]
fn technical_layer_draft_and_stale_form_preserve_authored_data() {
    let mut h = harness(1200.0);
    click(&mut h, "Board setup");
    click(&mut h, "Apply board setup");
    let revision = h.state().session.revision();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Board setup");
    replace(&mut h, "Layer name", "Assembly notes");
    click(&mut h, "Add layer");
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Board setup");
    replace(&mut h, "Layer name", "Assembly notes");
    click(&mut h, "Add layer");
    click(&mut h, "Apply board setup");
    assert!(
        h.state()
            .session
            .project()
            .unwrap()
            .board_setup()
            .unwrap()
            .layers()
            .iter()
            .any(|l| l.name == "Assembly notes" && l.purpose == LayerPurpose::User)
    );
    click(&mut h, "Board setup");
    h.state_mut().run("project.rename", &json!({"name":"Changed while editing"}));
    h.run();
    let current = h.state().session.inspect();
    click(&mut h, "Apply board setup");
    assert!(matches!(h.state().dialog, Dialog::BoardSetup(_)));
    assert_eq!(h.state().session.inspect(), current);
    click(&mut h, "Cancel");
}

/// Untouched unit strings retain nanometer precision and existing dielectric tolerance metadata.
#[test]
fn rounded_display_does_not_rewrite_exact_setup_dimensions() {
    let mut s = pcbcraft_engine::Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    let mut value = serde_json::to_value(s.project().unwrap().board_setup().unwrap()).unwrap();
    value["stackup"]["strata"][0]["thickness_nm"] = json!(1_234_567);
    value["stackup"]["strata"][1]["thickness_nm"] = json!(1_000_000);
    value["stackup"]["strata"][1]["thickness_tolerance_nm"] = json!(100_000);
    s.apply(SET_BOARD_SETUP, &json!({"setup":value,"expected_revision":s.revision()})).unwrap();
    let setup = s.project().unwrap().board_setup().unwrap();
    for unit in [units::Unit::Mm, units::Unit::Mil] {
        assert_eq!(&Candidate::new(setup.clone(), unit).value(unit).unwrap(), setup);
    }
}

/// Material characterization is staged through real controls and pinned by explicit revision;
/// changing copper count and adding Tg/Dk/Df never commits until the overall Apply action.
#[test]
fn expanded_count_and_sourced_material_reports_use_the_real_form() {
    let mut h = harness(1200.);
    click(&mut h, "Board setup");
    count(&mut h, 6);
    click(&mut h, "Apply board setup");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Board setup");
    count(&mut h, 8);
    page(&mut h, "Materials");
    click(&mut h, "Add material");
    click(&mut h, "Technical properties");
    for (label, value) in [
        ("Material name", "Test laminate"),
        ("Tg (°C)", "180"),
        ("Dk", "4.2"),
        ("Df", "0.018"),
        ("Frequency (Hz)", "1000000000"),
        ("Source document", "Original test specification"),
    ] {
        replace(&mut h, label, value);
    }
    for (field, choice) in [("Tg method", "DSC"), ("Basis", "Typical")] {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, field).scroll_to_me();
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, field).click();
        h.run();
        click(&mut h, choice);
    }
    h.get_by_label("Save material to draft").scroll_to_me();
    h.run();
    click(&mut h, "Save material to draft");
    assert_eq!(h.state().session.revision(), revision);
    if let Dialog::BoardSetup(edit) = &h.state().dialog {
        assert!(edit.material_edit.is_none(), "{:?}", edit.error);
        assert_eq!(edit.candidate.materials.len(), 2);
    }
    page(&mut h, "Physical stackup");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dielectric 1").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Construction").scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Construction").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Core").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Material specification").scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Material specification").click();
    h.run();
    click(&mut h, "Test laminate · r1");
    click(&mut h, "Apply board setup");
    assert!(matches!(h.state().dialog, Dialog::None));
    let setup = h.state().session.project().unwrap().board_setup().unwrap();
    assert_eq!(setup.stackup().strata.len(), 15);
    let material = setup.materials().iter().find(|m| m.name == "Test laminate").unwrap();
    assert_eq!(material.tg[0].temperature_milli_c, 180000);
    assert_eq!(material.dielectric[0].dk_millionths, Some(4200000));
    assert_eq!(material.dielectric[0].df_millionths, Some(18000));
    assert!(
        matches!(&setup.stackup().strata[1],Stratum::Dielectric{construction:Some(Construction::Core{material:Some(reference),..}),..} if *reference==material.reference())
    );
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    click(&mut h, "Undo");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Thickness fitting preserves locked dielectric dimensions and all copper, distributes the
/// exact residual nanometers, and rejects impossible targets without partial draft changes.
#[test]
fn dielectric_fitting_respects_locks_and_rejects_invalid_targets() {
    let mut h = harness(1200.);
    click(&mut h, "Board setup");
    count(&mut h, 6);
    click(&mut h, "Apply board setup");
    click(&mut h, "Board setup");
    let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
    let candidate = &mut edit.candidate;
    for i in (0..candidate.thickness.len()).step_by(2) {
        candidate.thickness[i] = Length::new(Some(35000), edit.unit);
    }
    let Stratum::Dielectric { id, .. } = candidate.setup.stackup().strata[1] else { panic!("gap") };
    candidate.thickness[1] = Length::new(Some(300000), edit.unit);
    edit.locked.insert(id.to_string());
    edit.target_thickness = "1.6 mm".into();
    edit.fit().unwrap();
    let value = edit.candidate.value(edit.unit).unwrap();
    assert_eq!(value.laminate_thickness_nm(), Some(1600000));
    assert_eq!(value.stackup().strata[1].thickness_nm(), Some(300000));
    edit.target_thickness = "0.1 mm".into();
    assert!(edit.fit().is_err());
    assert_eq!(edit.candidate.value(edit.unit).unwrap(), value);
    edit.target_thickness = "-1 mm".into();
    assert!(edit.fit().is_err());
    assert_eq!(edit.candidate.value(edit.unit).unwrap(), value);
}

/// Navigating between layer and print pages retains a single draft; invalid print settings
/// publish neither edit, and a successful Apply/Undo changes and restores them together.
#[test]
fn layer_and_silkscreen_drafts_share_validation_and_undo() {
    for width in [1200., 700.] {
        let mut h = harness(width);
        click(&mut h, "Board setup");
        click(&mut h, "Apply board setup");
        assert!(h.query_by_label("Edit silkscreen settings").is_none());
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        click(&mut h, "Board setup…");
        replace(&mut h, "Layer name Top", "Signal");
        page(&mut h, "Silkscreen");
        replace(&mut h, "Stroke width", "2 mm");
        click(&mut h, "Apply board setup");
        assert!(matches!(h.state().dialog, Dialog::BoardSetup(_)));
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        replace(&mut h, "Stroke width", "0.2 mm");
        page(&mut h, "Board editor layers");
        assert_eq!(h.get_by_label("Layer name Top").value().as_deref(), Some("Signal"));
        click(&mut h, "Apply board setup");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().session.revision(), revision + 1);
        assert_eq!(h.state().session.inspect()["project"]["board"]["silkscreen"]["stroke_width_nm"], 200_000);
        click(&mut h, "Undo");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// A board without saved setup opens the complete form immediately. All pages share one
/// disposable draft, and reopening after Apply exposes the same controls and adopted values.
#[test]
fn first_and_subsequent_setup_openings_share_the_complete_editor() {
    for width in [1200., 700.] {
        let mut h = harness(width);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        click(&mut h, "Board setup…");
        assert!(h.get_by_label("Board setup…").rect().height() >= pcbcraft_chrome::Tokens::get().control_height);
        h.get_by_label("Apply board setup");
        replace(&mut h, "Layer name Top", "Signal");
        page(&mut h, "Physical stackup");
        replace(&mut h, "Signal thickness", "0.035 mm");
        page(&mut h, "Silkscreen");
        replace(&mut h, "Text height", "1.2 mm");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Cancel");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Board setup…");
        replace(&mut h, "Layer name Top", "Signal");
        page(&mut h, "Silkscreen");
        replace(&mut h, "Text height", "1.2 mm");
        click(&mut h, "Apply board setup");
        assert!(matches!(h.state().dialog, Dialog::None));
        click(&mut h, "Board setup…");
        assert_eq!(h.get_by_label("Layer name Signal").value().as_deref(), Some("Signal"));
        page(&mut h, "Silkscreen");
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Text height");
        click(&mut h, "Cancel");
        click(&mut h, "Undo");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Profile imports preview only in their owning form; Apply/Undo share the existing board
/// transaction. Closing/reopening, stale revisions and lost workers cannot adopt old results.
#[test]
fn profile_import_is_staged_revision_checked_and_owned_by_its_form() {
    use pcbcraft_engine::board::setup::{StackupPreset, profile::SetupProfile};
    let mut h = harness(1200.);
    let original = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let source =
        h.state().session.project().unwrap().boards().next().unwrap().setup_draft(2).unwrap().with_starting_stackup(StackupPreset::Fr4TwoOz).unwrap();
    let profile = SetupProfile::new("Shared heavy copper".into(), source, Default::default()).unwrap();
    click(&mut h, "Board setup");
    let (ticket, send) = h.state_mut().board_profile_files.stand_in();
    let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
    edit.profile_ticket = Some(ticket);
    send.send(Ok(profile_io::Outcome::Imported(Box::new(profile.clone())))).unwrap();
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
    page(&mut h, "Profiles");
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Profile name").value().as_deref(), Some("Shared heavy copper"));
    click(&mut h, "Apply board setup");
    assert_eq!(h.state().session.project().unwrap().board_setup().unwrap().stackup().strata[0].thickness_nm(), Some(70000));
    click(&mut h, "Undo");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
    click(&mut h, "Board setup");
    let (ticket, send) = h.state_mut().board_profile_files.stand_in();
    let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
    edit.profile_ticket = Some(ticket);
    click(&mut h, "Cancel");
    assert!(h.state().board_profile_files.busy());
    send.send(Ok(profile_io::Outcome::Imported(Box::new(profile.clone())))).unwrap();
    click(&mut h, "Board setup");
    let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
    assert_eq!(edit.profile_name, "Board setup");
    let (ticket, send) = h.state_mut().board_profile_files.stand_in();
    let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
    edit.profile_ticket = Some(ticket);
    h.state_mut().run("project.rename", &json!({"name":"Changed while importing"}));
    assert!(h.state().message.is_none());
    send.send(Ok(profile_io::Outcome::Imported(Box::new(profile)))).unwrap();
    h.run();
    let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
    assert_eq!(edit.profile_name, "Board setup");
    assert!(edit.error.as_ref().unwrap().contains("project or view changed"));
    click(&mut h, "Cancel");
    click(&mut h, "Undo");
    click(&mut h, "Board setup");
    let (ticket, send) = h.state_mut().board_profile_files.stand_in();
    let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
    edit.profile_ticket = Some(ticket);
    drop(send);
    h.run();
    assert!(!h.state().board_profile_files.busy());
    let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
    assert!(edit.error.as_ref().unwrap().contains("worker stopped"));
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
}

/// An abandoned picker reserves the one file slot, but only its owning form waits to Apply.
/// A reopened form can commit initial or existing setup without adopting the old result.
#[test]
fn abandoned_profile_job_does_not_block_reopened_setup_apply() {
    use pcbcraft_engine::board::setup::{StackupPreset, profile::SetupProfile};
    for width in [1200., 700.] {
        for initialized in [false, true] {
            let mut h = harness(width);
            click(&mut h, "Board setup");
            if initialized {
                click(&mut h, "Apply board setup");
                click(&mut h, "Board setup");
            }
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            let source = h
                .state()
                .session
                .project()
                .unwrap()
                .boards()
                .next()
                .unwrap()
                .setup_draft(2)
                .unwrap()
                .with_starting_stackup(StackupPreset::Fr4TwoOz)
                .unwrap();
            let profile = SetupProfile::new("Abandoned import".into(), source, Default::default()).unwrap();
            let (ticket, send) = h.state_mut().board_profile_files.stand_in();
            let Dialog::BoardSetup(edit) = &mut h.state_mut().dialog else { panic!("setup") };
            edit.profile_ticket = Some(ticket);
            h.run();
            assert!(h.get_by_label("Apply board setup").accesskit_node().is_disabled());
            click(&mut h, "Cancel");
            click(&mut h, "Board setup");
            assert!(h.state().board_profile_files.busy());
            assert!(!h.get_by_label("Apply board setup").accesskit_node().is_disabled());
            page(&mut h, "Profiles");
            assert!(h.get_by_label("Import profile…").accesskit_node().is_disabled());
            assert!(h.get_by_label("Export profile…").accesskit_node().is_disabled());
            page(&mut h, "Board editor layers");
            replace(&mut h, "Layer name Top", "Signal");
            click(&mut h, "Apply board setup");
            assert!(matches!(h.state().dialog, Dialog::None));
            assert!(h.state().board_profile_files.busy());
            let setup = h.state().session.project().unwrap().board_setup().unwrap();
            assert!(setup.layers().iter().any(|layer| layer.name == "Signal"));
            assert_eq!(setup.stackup().strata[0].thickness_nm(), Some(35_000));
            let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            send.send(Ok(profile_io::Outcome::Imported(Box::new(profile)))).unwrap();
            click(&mut h, "Board setup");
            assert!(!h.state().board_profile_files.busy());
            let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
            assert_eq!(edit.profile_name, "Board setup");
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
            click(&mut h, "Cancel");
            click(&mut h, "Undo");
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// Blank, malformed and out-of-range silkscreen dimensions explain the preview failure before
/// Apply. Apply/export share that error and cannot change authored bytes or launch a file job.
#[test]
fn silkscreen_validation_is_visible_before_apply_and_export() {
    for width in [1200., 700.] {
        for initialized in [false, true] {
            let mut h = harness(width);
            click(&mut h, "Board setup");
            if initialized {
                click(&mut h, "Apply board setup");
                click(&mut h, "Board setup");
            }
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            let revision = h.state().session.revision();
            page(&mut h, "Silkscreen");
            for (label, value, error) in [
                ("Text height", "", "Enter a text height."),
                ("Stroke width", "", "Enter a stroke width."),
                ("Pad clearance", "", "Enter a pad clearance."),
                ("Stroke width", "nope", "Invalid stroke width: Enter a number, such as 20 mil or 0.5 mm"),
                (
                    "Stroke width",
                    "2 mm",
                    "silkscreen requires positive text height up to 10 mm, stroke width up to half the text height, and clearance from 0 to 10 mm",
                ),
            ] {
                let original = h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).value().unwrap();
                replace(&mut h, label, value);
                h.get_by_label(error);
                assert_eq!(h.state().session.revision(), revision);
                click(&mut h, "Apply board setup");
                let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
                assert_eq!(edit.error.as_deref(), Some(error));
                assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
                page(&mut h, "Profiles");
                click(&mut h, "Export profile…");
                assert!(!h.state().board_profile_files.busy());
                let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
                assert_eq!(edit.error.as_deref(), Some(error));
                page(&mut h, "Silkscreen");
                replace(&mut h, label, &original);
            }
            click(&mut h, "Cancel");
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// Family and copper presets stay disposable until Apply, reuse immutable catalog pins, and
/// make no supplier measurements. Custom material editing keeps characterization optional.
#[test]
fn starting_material_and_copper_presets_stage_real_board_settings() {
    for width in [1200., 700.] {
        let mut h = harness(width);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        click(&mut h, "Board setup");
        page(&mut h, "Materials");
        click(&mut h, "Use High-Tg FR-4");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Use High-Tg FR-4");
        let Dialog::BoardSetup(edit) = &h.state().dialog else { panic!("setup") };
        assert_eq!(edit.candidate.materials.len(), 2);
        assert!(edit.candidate.materials.iter().all(|m| m.tg.is_empty() && m.dielectric.is_empty()));
        click(&mut h, "Add material");
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Material name");
        assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Tg (°C)").is_none());
        click(&mut h, "Cancel");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Board setup");
        page(&mut h, "Physical stackup");
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Starting stackup").click();
        h.run();
        click(&mut h, "FR-4 · 2 oz copper · 1.6 mm");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Apply board setup");
        let setup = h.state().session.project().unwrap().board_setup().unwrap();
        assert_eq!(setup.stackup().strata[0].thickness_nm(), Some(70000));
        assert_eq!(setup.laminate_thickness_nm(), Some(1600000));
        click(&mut h, "Undo");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}
