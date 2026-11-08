use super::*;
use pcbcraft_engine::{
    net_labels::{self as engine, Style},
    persistence::encode_documents,
    schematic::Point,
};

/// Exercise the real editor with room for labels; panel state changes only test presentation.
fn editor(width: f32) -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(width, 800.));
    new_project(&mut h);
    h.state_mut().parts.open = false;
    h.state_mut().browser.open = false;
    h.run();
    h
}
/// Replace the focused name through actual text input.
fn name(h: &mut Harness<'_, PcbApp>, name: &str) {
    h.get_by_label("Net name").click();
    h.run();
    clear_name(h);
    h.get_by_label("Net name").type_text(name);
    h.run();
}
/// Pick a model-derived label anchor so canvas and toolbar dimensions may evolve independently.
fn at(h: &Harness<'_, PcbApp>, p: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(p, h.get_by_label("Schematic canvas").rect())
}

/// Power placement accepts a preset, previews rotation without editing, and commits once.
/// Hover hotkeys edit/move/delete the label instead of any component underneath its ink.
#[test]
fn power_placement_edit_move_delete_and_undo() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.key_press(Key::P);
    h.run();
    assert!(h.state().parts.open);
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("GND"));
    click(&mut h, "Place");
    assert_eq!(h.state().session.revision(), revision);
    h.key_press(Key::R);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.rotation==90));
    let point = canvas_center(&h);
    click_canvas_at(&mut h, point);
    assert_eq!(h.state().session.revision(), revision + 1);
    let label = h.state().session.project().unwrap().net_labels()[0].clone();
    assert_eq!(label.style, Style::Ground);
    let hover = at(&h, label.at);
    h.hover_at(hover);
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)));
    name(&mut h, "GNDA");
    click(&mut h, "Apply");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "GNDA");
    h.hover_at(hover);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::NetLabel { .. })));
    click_canvas_at(&mut h, hover + egui::vec2(80., 40.));
    let moved = h.state().session.project().unwrap().net_labels()[0].clone();
    assert_ne!(moved.at, label.at);
    let hover = at(&h, moved.at);
    h.hover_at(hover);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    click(&mut h, "Undo");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0], moved);
}

/// Local/global shortcuts and compact caption-menu controls reach the same form. Cancelled
/// properties and cancelled placement leave both encoded bytes and undo history unchanged.
#[test]
fn label_tools_and_cancel_at_both_widths() {
    for width in [1400., 600.] {
        let mut h = editor(width);
        let original = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.key_press(Key::L);
        h.run();
        name(&mut h, "RESET");
        click(&mut h, "Place");
        assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.style==Style::Local));
        h.key_press(Key::Escape);
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::L);
        h.run();
        click(&mut h, "Place");
        assert!(matches!(h.state().views.active.schematic.tool,Some(Tool::NetLabel {ref label,..}) if label.style==Style::Global));
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label("Place menu").scroll_to_me();
        h.run();
        click(&mut h, "Place menu");
        click(&mut h, "Power");
        assert!(h.state().parts.open);
        click(&mut h, "Cancel");
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), original);
    }
}

/// The live adapter discovers and executes named-anchor commands; GUI history and both
/// inspectors use the same names. A form captured before a concurrent edit cannot overwrite it.
#[test]
fn live_commands_inspection_names_and_stale_form() {
    use crate::live_control::LiveEditor;
    use pcbcraft_mcp::live::call_editor;
    let mut h = editor(1400.);
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    let result = call_editor(
        &mut LiveEditor { app: h.state_mut(), ctx: &ctx },
        &json!({"method":"engine.execute","params":{"command":engine::ADD,"params":{"name":"+3V3","style":"power","x_nm":0,"y_nm":0,"expected_revision":revision}}}),
    );
    assert!(result.is_ok(), "{result:?}");
    h.run();
    click(&mut h, "Toggle Net Inspector");
    h.get_by_label("+3V3");
    let label = h.state().session.project().unwrap().net_labels()[0].clone();
    let p = at(&h, label.at);
    h.hover_at(p);
    h.run();
    h.key_press(Key::E);
    h.run();
    h.state_mut().session.apply("project.rename", &json!({"name":"Concurrent"})).unwrap();
    name(&mut h, "+5V");
    click(&mut h, "Apply");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].name, "+3V3");
    assert!(h.state().message.as_deref().unwrap_or_default().contains("changed"));
}

/// Text clicks select a label without opening its form. Selection supports properties and
/// toolbar actions away from the pointer, while clicking empty canvas releases it normally.
#[test]
fn label_selection_routes_toolbar_and_keyboard_without_mutating_data() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(engine::ADD, &json!({"name":"RESET","style":"local","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    let point = at(&h, Point { x_nm: 0, y_nm: 0 });
    click_canvas_at(&mut h, point + egui::vec2(25., -10.));
    assert!(h.state().views.active.schematic.selected_net_label.is_some());
    assert!(matches!(h.state().dialog, Dialog::None));
    let revision = h.state().session.revision();
    h.hover_at(point + egui::vec2(200., 100.));
    h.run();
    h.key_press(Key::E);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NetLabel(_)));
    click(&mut h, "Cancel");
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Rotate selected object");
    assert_eq!(h.state().session.project().unwrap().net_labels()[0].rotation, 90);
    h.get_by_label("Modify menu").scroll_to_me();
    h.run();
    click(&mut h, "Modify menu");
    click(&mut h, "Delete");
    assert!(h.state().session.project().unwrap().net_labels().is_empty());
    click(&mut h, "Undo");
    let point = at(&h, Point { x_nm: 0, y_nm: 0 });
    click_canvas_at(&mut h, point + egui::vec2(25., -10.));
    click_canvas_at(&mut h, point + egui::vec2(200., 100.));
    assert!(h.state().views.active.schematic.selected_net_label.is_none());
}

/// Marker placement uses magnetic contact coordinates even when the focused view's snap
/// spacing would otherwise pull an existing pin off its electrical anchor.
#[test]
fn power_placement_reaches_pins_between_grid_points() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":1_270_000,"y_nm":0,"expected_revision":revision}));
    h.state_mut().views.active.camera.set_snap_nm(2_540_000);
    h.run();
    let pin = h.state().session.project().unwrap().symbols()[0].terminals()[0];
    let point = h.state().session.project().unwrap().terminal_position(pin).unwrap();
    let screen = at(&h, point);
    assert_ne!(h.state().views.active.camera.snap(screen, h.get_by_label("Schematic canvas").rect()), point);
    h.key_press(Key::P);
    h.run();
    click(&mut h, "Place");
    click_canvas_at(&mut h, screen);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.net_labels()[0].at, point);
    let index = project.connectivity().net_index_for(pin).unwrap();
    assert_eq!(project.connectivity().net_name(index), "GND");
}
