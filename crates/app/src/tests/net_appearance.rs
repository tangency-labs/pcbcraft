//! Wire Properties opens one revision-bound form that authors a net-wide drawing override.
use super::*;
use pcbcraft_chrome::theme;
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

/// Two panes on the same sheet, with the one named net selected in the focused pane.
fn split_harness() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.run();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    h.get_by_label("Schematic canvas 1");
    h.get_by_label("Schematic canvas 2");
    h
}

/// Resolve a pane by identity rather than by its current focus.
fn view<'a>(h: &'a Harness<'_, PcbApp>, id: u64) -> &'a EditorView {
    h.state().views.view(ViewId(id)).unwrap()
}

/// The canvas rectangle for a pane, used to match that pane's painted shapes.
fn canvas(h: &Harness<'_, PcbApp>, id: u64) -> egui::Rect {
    h.get_by_label(&format!("Schematic canvas {}", id + 1)).rect()
}

/// Focus a pane through its real tab without changing either camera.
fn focus_tab(h: &mut Harness<'_, PcbApp>, id: usize) {
    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    h.get_all_by_label(&path).nth(id).unwrap().click();
    h.run();
}

/// The stroke of the painted `wire` in pane `id`, matched by the wire's projected endpoints so
/// an unrelated symbol stroke of the same width cannot satisfy the assertion.
fn wire_stroke(h: &Harness<'_, PcbApp>, id: u64, wire: &str) -> Option<(f32, egui::epaint::ColorMode)> {
    let rect = canvas(h, id);
    let camera = &view(h, id).camera;
    let project = h.state().session.project().unwrap();
    let route = project.wire_path(project.wires().iter().find(|candidate| candidate.id() == wire)?);
    let (from, to) = (camera.screen(route[0], rect), camera.screen(*route.last().unwrap(), rect));
    h.output().shapes.iter().find_map(|clipped| {
        let (points, width, color) = match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } => (points.as_slice(), stroke.width, egui::epaint::ColorMode::Solid(stroke.color)),
            egui::Shape::Path(path) => (path.points.as_slice(), path.stroke.width, path.stroke.color.clone()),
            _ => return None,
        };
        let touches =
            |a, b| points.first().is_some_and(|point| point.distance(a) < 1.0) && points.last().is_some_and(|point| point.distance(b) < 1.0);
        (touches(from, to) || touches(to, from)).then_some((width, color))
    })
}

/// Type a value into an enabled DragValue through its accessible spin-button handle.
fn set_spin(h: &mut Harness<'_, PcbApp>, label: &str, value: &str) {
    h.get_by_role_and_label(egui::Role::SpinButton, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role_and_label(egui::Role::SpinButton, label).type_text(value);
    h.run();
    h.key_press(Key::Enter);
    h.run();
}

/// An appearance edit made through the form in one pane is stored on the shared net and repaints
/// the same wire in the other pane. Undo issued from that other pane is one shared history step.
/// This exercises the form's real shared command and the normal `views.sync` repaint, not a
/// synthetic shortcut into one canvas.
#[test]
fn net_appearance_edit_in_one_pane_repaints_the_other_pane() {
    let mut h = split_harness();
    let wire = h.state().session.project().unwrap().wires()[0].id();
    let revision = h.state().session.revision();
    for id in [0, 1] {
        assert_eq!(wire_stroke(&h, id, &wire).map(|stroke| stroke.0), Some(theme::SCHEMATIC_SYMBOL_STROKE), "pane {id} before");
    }

    click(&mut h, "Net appearance…");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetAppearance(_)), "the form opens for the focused pane's wire");
    click(&mut h, "Set wire color");
    h.get_by_role_and_label(egui::Role::ColorWell, "Wire color").click();
    h.run();
    assert!(egui::Popup::is_any_open(&h.ctx), "the color picker opens");
    for (channel, value) in [("Red", "200"), ("Green", "50"), ("Blue", "100")] {
        set_spin(&mut h, channel, value);
    }
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Set drawn width");
    set_spin(&mut h, "Wire stroke width", "300");
    click(&mut h, "Apply");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.revision(), revision + 1, "one form apply is one shared command");
    let chosen = [200, 50, 100];
    let appearances = h.state().session.project().unwrap().wire_appearances().unwrap();
    assert_eq!(appearances.get(&wire), Some(&nets::Appearance { color: Some(chosen), width_cp: Some(300) }));
    // Clear the local selection so both panes paint the net color rather than the selection ink.
    for view in h.state_mut().views.all_mut() {
        view.schematic.selection = Default::default();
    }
    h.run();
    let ink = egui::epaint::ColorMode::Solid(egui::Color32::from_rgb(chosen[0], chosen[1], chosen[2]));
    for id in [0, 1] {
        assert_eq!(wire_stroke(&h, id, &wire), Some((3.0, ink.clone())), "pane {id} repainted");
    }

    focus_tab(&mut h, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().session.project().unwrap().wire_appearances().unwrap().get(&wire), None);
    let theme_ink = egui::epaint::ColorMode::Solid(theme::Tokens::get().schematic_wire);
    for id in [0, 1] {
        assert_eq!(wire_stroke(&h, id, &wire), Some((theme::SCHEMATIC_SYMBOL_STROKE, theme_ink.clone())), "pane {id} undone");
    }
}
