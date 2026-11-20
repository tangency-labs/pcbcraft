//! Wire Properties opens one revision-bound form that authors a net-wide drawing override.
use super::*;
use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, nets, selection::Selection};

fn harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Schematic);
    app.run("project.new", &json!({}));
    app.parts.open = false;
    app.browser.open = false;
    app.properties_open = true;
    for x in [0, 25_400_000] {
        app.run(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let pins: Vec<Value> = app.session.project().unwrap().symbols().iter().map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
    app.run(CONNECT_PINS, &json!({"from":pins[0],"to":pins[1],"expected_revision":app.session.revision()}));
    let wire = app.session.project().unwrap().wires()[0].id();
    app.views.active.schematic.selection = Selection::one_wire(wire);
    Harness::builder().with_size(egui::vec2(1200., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app)
}

/// A color chosen in the form is stored against the promoted net identity, not the wire, and
/// Cancel leaves the document and its revision untouched.
#[test]
fn wire_properties_author_net_appearance_through_one_form() {
    let mut h = harness();
    h.run();
    let before = pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Net appearance…");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetAppearance(_)));
    click(&mut h, "Cancel");
    h.run();
    assert_eq!(pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);

    click(&mut h, "Net appearance…");
    h.run();
    click(&mut h, "Set wire color");
    h.run();
    click(&mut h, "Apply");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    let project = h.state().session.project().unwrap();
    let appearances = project.wire_appearances().unwrap();
    assert_eq!(appearances.len(), 1);
    let theme_color = pcbcraft_chrome::theme::Tokens::get().schematic_wire;
    assert_eq!(
        appearances.values().next(),
        Some(&nets::Appearance { color: Some([theme_color.r(), theme_color.g(), theme_color.b()]), width_cp: None })
    );
    let saved = pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&saved[pcbcraft_engine::persistence::ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/52");
}

/// Save both channels through the form, which makes both checkboxes start checked next time.
fn seed_both_channels(h: &mut Harness<'_, PcbApp>) {
    click(h, "Net appearance…");
    click(h, "Set wire color");
    click(h, "Set drawn width");
    click(h, "Apply");
}

/// Unchecking one independent channel clears only that saved override, the checked channel keeps
/// its value, and one Undo restores both. This relies on the engine's `clear` marker rather than
/// preserving the unchecked saved value of a partial patch.
#[test]
fn unchecking_a_channel_clears_only_that_override_and_one_undo_restores_both() {
    let mut h = harness();
    h.run();
    let theme_color = pcbcraft_chrome::theme::Tokens::get().schematic_wire;
    let theme = [theme_color.r(), theme_color.g(), theme_color.b()];
    let both = nets::Appearance { color: Some(theme), width_cp: Some(nets::DEFAULT_WIDTH_CP) };

    seed_both_channels(&mut h);
    assert_eq!(h.state().session.project().unwrap().wire_appearances().unwrap().values().next(), Some(&both));

    for (uncheck, expected) in [
        ("Set drawn width", nets::Appearance { color: Some(theme), width_cp: None }),
        ("Set wire color", nets::Appearance { color: None, width_cp: Some(nets::DEFAULT_WIDTH_CP) }),
    ] {
        let revision = h.state().session.revision();
        click(&mut h, "Net appearance…");
        click(&mut h, uncheck);
        click(&mut h, "Apply");
        assert_eq!(h.state().session.revision(), revision + 1, "one form apply is one shared command");
        assert_eq!(h.state().session.project().unwrap().wire_appearances().unwrap().values().next(), Some(&expected));

        h.state_mut().run("edit.undo", &Value::Null);
        h.run();
        assert_eq!(h.state().session.project().unwrap().wire_appearances().unwrap().values().next(), Some(&both));
    }
}
