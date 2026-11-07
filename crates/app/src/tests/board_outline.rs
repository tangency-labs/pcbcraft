//! Boundary editing exercises the actual modal, canvas and shared project commands.
use super::*;
use egui::accesskit::Role;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, SET_BOARD_OUTLINE, persistence::encode_documents};

/// Start on an empty, addressed two-layer board; the outline must work without any footprints.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({}));
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h
}

/// Replace the selected field through the same keyboard focus path a person uses.
fn field(h: &mut Harness<'_, PcbApp>, name: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, name).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role_and_label(Role::TextInput, name).type_text(value);
    h.run();
}

/// Cancel leaves no geometry/history; mixed units create one exact edit, and removal is undoable.
#[test]
fn outline_form_apply_cancel_remove_and_undo() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Edit board outline");
    field(&mut h, "Width", "1000 mil");
    click(&mut h, "Cancel");
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Edit board outline");
    for (name, value) in [("Left", "-10 mm"), ("Top", "-200 mil"), ("Width", "1000 mil"), ("Height", "20 mm")] {
        field(&mut h, name, value);
    }
    click(&mut h, "Apply outline");
    assert!(matches!(h.state().dialog, Dialog::None), "{:?}", h.state().dialog);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(
        h.state().session.inspect()["project"]["board"]["outline"],
        json!({"shape":"rectangle","min":{"x_nm":-10_000_000,"y_nm":-5_080_000},"max":{"x_nm":15_400_000,"y_nm":14_920_000}})
    );
    let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Edit board outline");
    click(&mut h, "Remove outline");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Rounded mil/mm readouts must not become geometry edits on Apply, including when a different
/// field changes. Exact corner values survive the dialog rather than being rebuilt from labels.
#[test]
fn outline_form_preserves_unedited_nanometers() {
    for unit in [units::Unit::Mil, units::Unit::Mm] {
        let mut h = board();
        h.state_mut().preferences.units = unit;
        let revision = h.state().session.revision();
        h.state_mut().run(
            SET_BOARD_OUTLINE,
            &json!({"outline":{"shape":"rectangle",
            "min":{"x_nm":-20_000_001,"y_nm":-15_000_001},"max":{"x_nm":20_000_002,"y_nm":15_000_002}},"expected_revision":revision}),
        );
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        click(&mut h, "Edit board outline");
        click(&mut h, "Apply outline");
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Edit board outline");
        field(&mut h, "Width", "50 mm");
        click(&mut h, "Apply outline");
        let outline = h.state().session.inspect()["project"]["board"]["outline"].clone();
        assert_eq!(outline["min"], json!({"x_nm":-20_000_001,"y_nm":-15_000_001}));
        assert_eq!(outline["max"], json!({"x_nm":29_999_999,"y_nm":15_000_002}));
    }
}

/// Validation stays in the form; a later revision or reopen expires its authority entirely.
#[test]
fn outline_form_rejects_bad_dimensions_and_stale_openings() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    for width in ["0 mm", "-1 mm", "1 km", "200000 mm"] {
        click(&mut h, "Edit board outline");
        field(&mut h, "Width", width);
        click(&mut h, "Apply outline");
        assert!(matches!(h.state().dialog, Dialog::BoardOutline(_)));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        click(&mut h, "Cancel");
    }
    click(&mut h, "Edit board outline");
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed meanwhile"})).unwrap();
    let concurrent = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), concurrent);
    click(&mut h, "Edit board outline");
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), concurrent);
}

/// Fit includes an empty board's real boundary and respects floating cards. Hiding the semantic
/// outline layer changes only the view, and never edits the boundary, revision or saved bytes.
#[test]
fn outline_canvas_closes_fits_and_respects_layer_visibility() {
    use pcbcraft_engine::{board::setup::LayerPurpose, schematic::Point};
    let mut h = board();
    click(&mut h, "Edit board outline");
    click(&mut h, "Apply outline");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    let canvas = h.get_by_label("PCB canvas").rect();
    let camera = &h.state().views.active.camera;
    let min = camera.screen(Point { x_nm: -20_000_000, y_nm: -15_000_000 }, canvas);
    let max = camera.screen(Point { x_nm: 20_000_000, y_nm: 15_000_000 }, canvas);
    assert!(canvas.contains(min) && canvas.contains(max));
    assert!(max.x - min.x > 200.);
    let paths = |h: &Harness<'_, PcbApp>| {
        h.output()
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape,
        egui::Shape::Path(path) if shape.clip_rect == canvas && path.closed && path.points.len() == 4
            && path.stroke.color == egui::epaint::ColorMode::Solid(Tokens::get().board_ink))
            })
            .count()
    };
    assert_eq!(paths(&h), 1);
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let project = h.state().session.project().unwrap();
    let id = project
        .board_view(project.board_target())
        .unwrap()
        .setup()
        .unwrap()
        .layers()
        .iter()
        .find(|layer| layer.purpose == LayerPurpose::Outline)
        .unwrap()
        .id
        .to_string();
    h.state_mut().views.active.board.hidden_layers.insert(id);
    h.run();
    assert_eq!(paths(&h), 0);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    // A legal setup with no outline display layer must not make an authored boundary disappear.
    let mut setup = before["project"]["board"]["setup"].clone();
    setup["layers"].as_array_mut().unwrap().retain(|layer| layer["purpose"] != "outline");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision}));
    h.run();
    assert_eq!(paths(&h), 1);
}

/// Live MCP uses the same outline transaction and editor history; stale calls cannot remove it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_outline_command_shares_gui_history() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    let board = h.state().session.project().unwrap().board_target().board;
    let outline = json!({"shape":"rectangle","min":{"x_nm":-20_000_000,"y_nm":-15_000_000},"max":{"x_nm":20_000_000,"y_nm":15_000_000}});
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    {
        let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
        let request = json!({"method":"engine.execute","params":{"command":SET_BOARD_OUTLINE,"params":{"board":board,"outline":outline,"expected_revision":revision}}});
        let updated = pcbcraft_mcp::live::call_editor(&mut editor, &request).unwrap();
        assert_eq!(updated["project"]["board"]["outline"], outline);
        assert!(pcbcraft_mcp::live::call_editor(&mut editor, &request).is_err());
    }
    h.run();
    click(&mut h, "Edit board outline");
    click(&mut h, "Apply outline");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}
