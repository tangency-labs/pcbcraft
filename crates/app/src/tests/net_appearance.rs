//! Wire Properties authors a net-wide drawing override through the inline inspector.
use super::*;
use pcbcraft_chrome::theme;
use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, SheetTarget, nets, schematic::Terminal, selection::Selection, sheets};

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

/// Reset changes discards the inline draft, while Apply stores the color on the logical net.
#[test]
fn wire_properties_author_net_appearance_through_one_form() {
    let mut h = harness();
    h.run();
    let before = pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    assert!(matches!(h.state().dialog, Dialog::None));
    click(&mut h, "Set wire color");
    click(&mut h, "Reset changes");
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);

    click(&mut h, "Set wire color");
    click(&mut h, "Apply changes");
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
    assert_eq!(
        serde_json::from_slice::<Value>(&saved[pcbcraft_engine::persistence::ENTRY_POINT]).unwrap()["format"],
        pcbcraft_engine::persistence::CURRENT_PROJECT_FORMAT
    );

    click(&mut h, "Use theme defaults");
    assert_eq!(h.state().session.revision(), revision + 2);
    assert!(h.state().session.project().unwrap().wire_appearances().unwrap().is_empty());
    h.state_mut().run("edit.undo", &Value::Null);
    assert_eq!(pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
}

/// Save both channels through the form, which makes both checkboxes start checked next time.
fn seed_both_channels(h: &mut Harness<'_, PcbApp>) {
    click(h, "Set wire color");
    click(h, "Set wire width");
    click(h, "Apply changes");
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
        ("Set wire width", nets::Appearance { color: Some(theme), width_cp: None }),
        ("Set wire color", nets::Appearance { color: None, width_cp: Some(nets::DEFAULT_WIDTH_CP) }),
    ] {
        let revision = h.state().session.revision();
        click(&mut h, uncheck);
        click(&mut h, "Apply changes");
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
    // A selected or previewed wire also paints a wider contrasting underlay; the wire's own ink
    // is the stroke that carries its appearance color.
    let outline = pcbcraft_chrome::Tokens::get().text;
    h.output().shapes.iter().find_map(|clipped| {
        let (points, width, color) = match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color != outline => {
                (points.as_slice(), stroke.width, egui::epaint::ColorMode::Solid(stroke.color))
            }
            egui::Shape::Path(path) if path.stroke.color != egui::epaint::ColorMode::Solid(outline) => {
                (path.points.as_slice(), path.stroke.width, path.stroke.color.clone())
            }
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

    assert!(matches!(h.state().dialog, Dialog::None));
    click(&mut h, "Set wire color");
    h.get_by_role_and_label(egui::Role::ColorWell, "Wire color").click();
    h.run();
    assert!(egui::Popup::is_any_open(&h.ctx), "the color picker opens");
    for (channel, value) in [("Red", "200"), ("Green", "50"), ("Blue", "100")] {
        set_spin(&mut h, channel, value);
    }
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Set wire width");
    set_spin(&mut h, "Wire stroke width", "50");
    click(&mut h, "Apply changes");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.revision(), revision + 1, "one form apply is one shared command");
    let chosen = [200, 50, 100];
    let appearances = h.state().session.project().unwrap().wire_appearances().unwrap();
    assert_eq!(appearances.get(&wire), Some(&nets::Appearance { color: Some(chosen), width_cp: Some(80) }));
    // Clear the local selection so both panes paint the net color rather than the selection ink.
    for view in h.state_mut().views.all_mut() {
        view.schematic.selection = Default::default();
    }
    h.run();
    let ink = egui::epaint::ColorMode::Solid(egui::Color32::from_rgb(chosen[0], chosen[1], chosen[2]));
    for id in [0, 1] {
        assert_eq!(wire_stroke(&h, id, &wire), Some((0.8, ink.clone())), "pane {id} repainted");
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

/// An empty project harness for the cross-sheet scope test, which authors its own fixture rather
/// than reusing the single-wire harness above.
fn cross_sheet_harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Schematic);
    app.run("project.new", &json!({}));
    app.parts.open = false;
    app.browser.open = false;
    app.properties_open = true;
    Harness::builder().with_size(egui::vec2(1200., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app)
}

/// Apply one revision-checked command through the shared session, stamping the current revision
/// exactly as a person's or an MCP client's call does. A refused command panics so an invalid
/// fixture cannot silently pass.
fn run(h: &mut Harness<'_, PcbApp>, command: &str, mut params: Value) {
    params["expected_revision"] = json!(h.state().session.revision());
    h.state_mut().session.apply(command, &params).unwrap();
}

/// Author a wire between two resistors on `sheet`, named by a label on its first pin. Returns the
/// authored wire ID so a test can say exactly which wire an appearance edit paints.
fn labelled_wire(h: &mut Harness<'_, PcbApp>, sheet: SheetTarget, x_nm: i64, name: &str, style: &str) -> String {
    for offset in [0, 25_400_000] {
        run(h, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":x_nm+offset,"y_nm":0}));
    }
    let pins: Vec<Value> = h
        .state()
        .session
        .project()
        .unwrap()
        .for_sheet(sheet)
        .unwrap()
        .symbols()
        .iter()
        .rev()
        .take(2)
        .map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"}))
        .collect();
    run(h, CONNECT_PINS, json!({"sheet":sheet.id(),"from":pins[1],"to":pins[0]}));
    let sheet_project = h.state().session.project().unwrap().for_sheet(sheet).unwrap();
    let wire = sheet_project.wires().last().unwrap().id();
    let at = sheet_project.terminal_position(Terminal::parse(&pins[0]).unwrap()).unwrap();
    run(h, pcbcraft_engine::net_labels::ADD, json!({"sheet":sheet.id(),"name":name,"style":style,"x_nm":at.x_nm,"y_nm":at.y_nm}));
    wire
}

/// The numeric ID of the named label on `sheet`, used to find its net identity below.
fn label_id(h: &Harness<'_, PcbApp>, sheet: SheetTarget, name: &str) -> u64 {
    h.state().session.project().unwrap().for_sheet(sheet).unwrap().net_labels().iter().find(|label| label.name == name).unwrap().id
}

/// The typed anchor of the current net whose first anchor is one of `labels`. Derived nets have no
/// saved identity until an edit promotes them, so the public inspection view is the right source.
/// A global label's net lists one of its two labels first, hence the set of accepted label IDs.
fn net_anchor_for_labels(h: &Harness<'_, PcbApp>, labels: &[u64]) -> nets::Anchor {
    let registry = h.state().session.project().unwrap().inspect_net_registry().unwrap();
    registry["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|row| {
            let anchor: nets::Anchor = serde_json::from_value(row["anchor"].clone()).ok()?;
            (anchor.kind == nets::AnchorKind::Label && labels.contains(&anchor.object)).then_some(anchor)
        })
        .unwrap()
}

/// Apply a sparse net-wide appearance through the shipped `nets.set_appearance` command, as the
/// Properties form does, targeting a typed derived anchor. The edit promotes the net in the same
/// history step; the command is shared with MCP and refuses stale revisions.
fn set_appearance(h: &mut Harness<'_, PcbApp>, anchor: nets::Anchor, appearance: nets::Appearance) {
    let target = serde_json::to_value(nets::ClassTarget::Derived { anchor }).unwrap();
    run(h, nets::SET_APPEARANCE, json!({"target":target,"appearance":serde_json::to_value(appearance).unwrap()}));
}

/// A net appearance follows its one logical net across sheets joined by a global label, while two
/// sheet-local nets that happen to share a name stay independent. The edit and its Undo show
/// exactly which wires repaint and which stay untouched. This drives the same commands the form
/// and MCP use and checks the engine's wire-to-net paint mapping, not one canvas's rendering.
#[test]
fn net_appearance_scope_follows_shared_net_not_same_named_local_nets() {
    let mut h = cross_sheet_harness();
    let main = h.state().session.project().unwrap().sheet_target();
    run(&mut h, sheets::CREATE_SHEET, json!({"name":"Power"}));
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;

    // "BUS" is one logical net spanning both sheets; "LOCAL" is two distinct sheet-local nets.
    let shared_main = labelled_wire(&mut h, main, 0, "BUS", "global");
    let shared_power = labelled_wire(&mut h, power, 0, "BUS", "global");
    let local_main = labelled_wire(&mut h, main, 10 * 25_400_000, "LOCAL", "local");
    let local_power = labelled_wire(&mut h, power, 10 * 25_400_000, "LOCAL", "local");

    let bus = net_anchor_for_labels(&h, &[label_id(&h, main, "BUS"), label_id(&h, power, "BUS")]);
    let local_a = net_anchor_for_labels(&h, &[label_id(&h, main, "LOCAL")]);
    let local_b = net_anchor_for_labels(&h, &[label_id(&h, power, "LOCAL")]);
    assert_ne!(bus.object, local_a.object);
    assert_ne!(bus.object, local_b.object);
    assert_ne!(local_a.object, local_b.object, "same-named local nets on two sheets stay distinct identities");

    let revision = h.state().session.revision();
    let paint = nets::Appearance { color: Some([21, 42, 63]), width_cp: Some(320) };
    set_appearance(&mut h, bus, paint);
    assert_eq!(h.state().session.revision(), revision + 1, "one command is one shared history step");

    let painted = h.state().session.project().unwrap().wire_appearances().unwrap();
    assert_eq!(painted.len(), 2, "only the shared net's wires paint: {painted:?}");
    assert_eq!(painted.get(&shared_main), Some(&paint), "shared wire on the first sheet");
    assert_eq!(painted.get(&shared_power), Some(&paint), "shared wire on the other sheet");
    assert!(!painted.contains_key(&local_main), "same-named local net on the first sheet stays untouched");
    assert!(!painted.contains_key(&local_power), "same-named local net on the other sheet stays untouched");

    // Styling one of the two same-named local nets paints only its wire, not its name twin.
    let local_paint = nets::Appearance { color: Some([9, 9, 9]), width_cp: None };
    set_appearance(&mut h, local_a, local_paint);
    let painted = h.state().session.project().unwrap().wire_appearances().unwrap();
    assert_eq!(painted.len(), 3);
    assert_eq!(painted.get(&local_main), Some(&local_paint), "the edited local net's wire paints");
    assert!(!painted.contains_key(&local_power), "the other sheet's LOCAL stays untouched");

    // Undo is one step per edit: the shared edit survives the first Undo, then the original,
    // completely untouched state returns on the second.
    h.state_mut().run("edit.undo", &Value::Null);
    let painted = h.state().session.project().unwrap().wire_appearances().unwrap();
    assert_eq!(painted.len(), 2, "the shared edit survives the first Undo");
    assert_eq!(painted.get(&shared_main), Some(&paint));
    assert_eq!(painted.get(&shared_power), Some(&paint));
    assert!(!painted.contains_key(&local_main) && !painted.contains_key(&local_power));

    h.state_mut().run("edit.undo", &Value::Null);
    assert!(h.state().session.project().unwrap().wire_appearances().unwrap().is_empty(), "original state restored");
}
