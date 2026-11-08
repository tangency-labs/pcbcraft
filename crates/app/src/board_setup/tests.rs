use super::*;
use crate::tests::click;
use egui_kittest::{Harness, kittest::Queryable};
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

/// Replace one real field through ordinary editing keys, retaining its accessible name.
fn replace(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(label).type_text(text);
    h.run();
}

/// Setup creation is an explicit undoable edit; opening/cancelling does not dirty the project.
#[test]
fn board_setup_creation_cancel_and_undo_use_real_controls() {
    for (width, count) in [(1200.0, 2), (700.0, 4)] {
        let mut h = harness(width);
        let before = h.state().session.inspect();
        click(&mut h, "Board setup");
        click(&mut h, "Cancel");
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Board setup…");
        click(&mut h, &format!("{count} copper layers"));
        click(&mut h, "Create board setup");
        assert!(matches!(h.state().dialog, Dialog::None));
        let setup = h.state().session.project().unwrap().board_setup().unwrap();
        assert_eq!(setup.layers().iter().filter(|l| l.purpose == LayerPurpose::Copper).count(), count);
        assert!(setup.stackup().strata.iter().all(|s| s.thickness_nm().is_none()));
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
    click(&mut h, "4 copper layers");
    click(&mut h, "Create board setup");
    click(&mut h, "Board setup");
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 520.0));
    assert!(viewport.contains_rect(h.get_by_label("Apply board setup").rect()));
    assert!(viewport.contains_rect(h.get_by_label("Cancel").rect()));
    assert!(h.get_by_label("Layer name Top").rect().width() > 200.0);
    click(&mut h, "Stackup");
    assert!(h.get_by_label("Top thickness").rect().width() > 100.0);
    assert!(viewport.contains_rect(h.get_by_label("Apply board setup").rect()));
}

/// Invalid names cannot commit, while one valid name/thickness edit retains all IDs and is one undo step.
#[test]
fn board_setup_names_and_stackup_validate_before_atomic_apply() {
    let mut h = harness(1200.0);
    click(&mut h, "Board setup");
    click(&mut h, "Create board setup");
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
    click(&mut h, "Stackup");
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
            click(&mut h, "Create board setup");
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
        assert!(h.query_by_label("Add layer").is_none());
        if initialized {
            replace(&mut h, "Layer name Top", "Signal");
            click(&mut h, "Apply board setup");
            assert!(matches!(h.state().dialog, Dialog::None));
            assert!(h.state().session.project().unwrap().board_setup().unwrap().layers().iter().any(|l| l.name == "Signal"));
        } else {
            h.get_by_label("4 copper layers");
            click(&mut h, "4 copper layers");
            click(&mut h, "Create board setup");
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
    click(&mut h, "Create board setup");
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
    s.apply("project.new", &json!({})).unwrap();
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
