use super::*;
use pcbcraft_engine::{
    PLACE_RESISTOR,
    persistence::{decode_documents, encode_documents},
};

/// Start with a generic resistor, keeping native IDs from inspection rather than view ordering.
fn component_harness() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({})).unwrap();
    h.state_mut().session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
    h.run();
    h
}

/// Choose a package through the actual UI; every pad remains explicit rather than guessed.
fn choose_resistor(h: &mut Harness<'_, PcbApp>) {
    click(h, "Package");
    click(h, "starter / Resistor 0402 (1005 metric) · r4");
    click(h, "Terminal 1");
    click(h, "Pad 2");
    click(h, "Terminal 2");
    click(h, "Pad 1");
}

/// Assignment from capture survives reopen without creating copper; board placement and undo
/// use the saved mapping and share the same project history as the schematic.
#[test]
fn schematic_assignment_then_board_placement_survives_offline_reopen() {
    let mut h = component_harness();
    let point = canvas_center(&h);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Assign Footprint…");
    choose_resistor(&mut h);
    click(&mut h, "Assign");
    assert!(
        h.state().session.project().unwrap().board_view(h.state().session.project().unwrap().board_target()).unwrap().footprints().next().is_none()
    );
    h.get_by_label("Assigned · not placed");
    click(&mut h, "Close");
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().session.open_snapshot(decode_documents(&saved).unwrap());
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Assign and place component footprints");
    h.run();
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)), "dialog state {:?}; message {:?}", h.state().dialog, h.state().message);
    click(&mut h, "Place on board");
    assert!(matches!(h.state().dialog, Dialog::None));
    let inspected = h.state().session.inspect();
    assert_eq!(inspected["project"]["board"]["footprints"][0]["mapping"], json!({"1":"2","2":"1"}));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
}

/// Invalid maps remain editable, Close is inert, and an external command expires an open form
/// so it cannot apply to a newer project revision.
#[test]
fn footprint_assignment_invalid_cancel_and_stale_dialog_preserve_state() {
    let mut h = component_harness();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "Package");
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Assign");
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Close");
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().message.as_ref().is_some_and(|m| m.contains("View or project changed")));
    assert!(h.state().session.project().unwrap().footprint_assignments().is_empty());
}

/// A short window keeps the form inside the viewport and allows scrolling to placement and
/// Close, even when the complete package form is taller than the available content area.
#[test]
fn short_window_can_scroll_to_placement_and_close() {
    let mut h = component_harness();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    choose_resistor(&mut h);
    click(&mut h, "Assign");
    click(&mut h, "Close");
    let app = h.state_mut();
    let target = app.session.project().unwrap().board_target();
    app.views.active.open_document(crate::editor_view::DocumentTarget::Board(target), &app.session);
    h.set_size(egui::vec2(420.0, 320.0));
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    for label in ["Close", "Place on board"] {
        h.get_by_label(label).scroll_to_me();
        h.run();
        let rect = h.get_by_label(label).rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 320.0, "{label}: {rect:?}");
        assert!(rect.top() >= h.get_by_label("Assign Footprint").rect().bottom(), "form content overlaps its fixed title");
    }
    click(&mut h, "Place on board");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"].as_array().unwrap().len(), 1);
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    h.get_by_label("Close").scroll_to_me();
    h.run();
    click(&mut h, "Close");
    assert!(matches!(h.state().dialog, Dialog::None));
}

/// The live editor refuses assignment while its modal owns input, then shares the assignment,
/// placement and GUI undo transactions once the modal closes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_assignment_uses_modal_ownership_and_shared_history() {
    use pcbcraft_engine::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT};
    let mut h = component_harness();
    let component = h.state().session.inspect()["project"]["components"][0]["id"].clone();
    let request = json!({"method":"engine.execute","params":{"command":ASSIGN_FOOTPRINT,"params":{
        "component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,
        "mapping":{"1":"2","2":"1"},"expected_revision":h.state().session.revision()}}});
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    let ctx = h.ctx.clone();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert!(pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Close");
    pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap();
    let assigned = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let place = json!({"method":"engine.execute","params":{"command":PLACE_ASSIGNED_FOOTPRINT,"params":{
        "component":component,"x_nm":0,"y_nm":0,"expected_revision":h.state().session.revision()}}});
    pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &place).unwrap();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), assigned);
}
