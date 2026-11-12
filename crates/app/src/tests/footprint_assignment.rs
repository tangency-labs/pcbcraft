use super::*;
use pcbcraft_engine::{
    PLACE_RESISTOR,
    persistence::{decode_documents, encode_documents},
};

/// Start with a generic resistor, keeping native IDs from inspection rather than view ordering.
fn component_harness() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({"create_board":true})).unwrap();
    h.state_mut().session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
    h.run();
    h
}

/// Choose a package through the actual UI; every pad remains explicit rather than guessed.
fn choose_resistor(h: &mut Harness<'_, PcbApp>) {
    click(h, "starter / Resistor 0402 (1005 metric) · r4");
    h.get_by_label("Terminal 1").scroll_to_me();
    h.run();
    click(h, "Terminal 1");
    click(h, "Pad 2");
    h.get_by_label("Terminal 2").scroll_to_me();
    h.run();
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
        h.state()
            .session
            .project()
            .unwrap()
            .board_view(h.state().session.project().unwrap().board_target().unwrap())
            .unwrap()
            .footprints()
            .next()
            .is_none()
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
    crate::tests::assert_same_content(&decode_documents(&saved).unwrap(), &h.state().session.snapshot().unwrap());
}

/// Invalid maps remain editable, Close is inert, and an external command expires an open form
/// so it cannot apply to a newer project revision.
#[test]
fn footprint_assignment_invalid_cancel_and_stale_dialog_preserve_state() {
    let mut h = component_harness();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    assert!(
        h.output().shapes.iter().any(|clipped| matches!(&clipped.shape,
        egui::Shape::Text(text) if (text.galley.job.text.contains("Map every terminal and pad exactly once") || text.galley.job.text.contains("must map terminal IDs to pad IDs"))
            && clipped.clip_rect.contains_rect(text.visual_bounding_rect()))),
        "mapping error must be visible after validation"
    );
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
/// Escape closes mapping choices before the draft, and the title close discards no saved work.
#[test]
fn short_window_can_scroll_to_placement_and_close() {
    let mut h = component_harness();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    choose_resistor(&mut h);
    click(&mut h, "Assign");
    click(&mut h, "Close");
    let app = h.state_mut();
    let target = app.session.project().unwrap().board_target().unwrap();
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
    h.get_by_label("Terminal 1").scroll_to_me();
    h.run();
    click(&mut h, "Terminal 1");
    h.key_press(Key::Escape);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    click(&mut h, "Place on board");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"].as_array().unwrap().len(), 1);
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    let before = h.state().session.inspect();
    click(&mut h, "Close Assign Footprint");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.inspect(), before);
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
    crate::tests::assert_same_content(&decode_documents(&assigned).unwrap(), &h.state().session.snapshot().unwrap());
}

/// Library, token search and physical filters narrow the list without changing the project.
/// Preview navigation changes the rendered copper, while Fit restores its original framing.
#[test]
fn footprint_browser_filters_and_preview_leave_assignment_untouched() {
    let mut h = component_harness();
    h.set_size(egui::vec2(1440., 1000.));
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "starter");
    click(&mut h, "Search footprints");
    h.get_by_label("Search footprints").type_text("resistor 0603");
    h.run();
    h.get_by_label("starter / Resistor 0603 (1608 metric) · r4");
    assert!(h.query_by_label("starter / Resistor 0402 (1005 metric) · r4").is_none());
    click(&mut h, "starter / Resistor 0603 (1608 metric) · r4");
    let row = h.get_by_label("starter / Resistor 0603 (1608 metric) · r4").rect();
    let rendered = h
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with("Resistor 0603 (1608 metric)\n") => Some(text),
            _ => None,
        })
        .unwrap();
    assert!(rendered.pos.x - row.left() < 20., "result name must stay left aligned");
    assert!(rendered.galley.job.sections[0].format.font_id.size > rendered.galley.job.sections[1].format.font_id.size);
    let preview = h.get_by_label("Footprint preview").rect();
    assert!(preview.width() >= 360. && preview.height() >= 300., "{preview:?}");
    let copper_bounds = |h: &Harness<'_, PcbApp>| {
        h.output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Path(path)
                    if path.fill == pcbcraft_chrome::board::copper_color(Some(true)) && preview.contains(path.visual_bounding_rect().center()) =>
                {
                    Some(path.visual_bounding_rect())
                }
                _ => None,
            })
            .reduce(|a, b| a.union(b))
            .expect("preview copper")
    };
    let fitted = copper_bounds(&h);
    click(&mut h, "Zoom footprint preview in");
    assert!((copper_bounds(&h).width() / fitted.width() - 1.25).abs() < 0.02);
    let start = preview.center();
    h.drag_at(start);
    h.run();
    h.hover_at(start + egui::vec2(25., 15.));
    h.run();
    h.drop_at(start + egui::vec2(25., 15.));
    h.run();
    assert!(copper_bounds(&h).center().distance(fitted.center()) > 20.);
    click(&mut h, "Fit");
    assert!(copper_bounds(&h).center().distance(fitted.center()) < 0.1);
    assert!((copper_bounds(&h).width() - fitted.width()).abs() < 0.1);
    click(&mut h, "Mounting type");
    click(&mut h, "Through hole");
    h.get_by_label("No footprints match");
    click(&mut h, "Clear filters");
    h.get_by_label("starter / Resistor 0402 (1005 metric) · r4");
    h.get_by_label("starter / SOT-23 (TO-236AB) · r4");
    click(&mut h, "Match pin count");
    assert!(h.query_by_label("starter / SOT-23 (TO-236AB) · r4").is_none());
    click(&mut h, "All libraries");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Close");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A different preview cannot place an existing assignment; assigning still requires a valid map.
#[test]
fn browsing_another_package_cannot_silently_place_or_replace_the_saved_assignment() {
    let mut h = component_harness();
    h.set_size(egui::vec2(1440., 1000.));
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    click(&mut h, "Close");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let target = h.state().session.project().unwrap().board_target().unwrap();
    let app = h.state_mut();
    app.views.active.open_document(crate::editor_view::DocumentTarget::Board(target), &app.session);
    app.begin_footprint_assignment(None);
    h.run();
    click(&mut h, "starter / Resistor 0603 (1608 metric) · r4");
    assert!(h.get_by_label("Place on board").accesskit_node().is_disabled());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Terminal 1");
    click(&mut h, "Pad 2");
    click(&mut h, "Assign");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    click(&mut h, "Close");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A large project uses a virtual scrolling component list with reference/value search.
/// Symbol-originated assignment stays bound to its selected component without a global picker.
#[test]
fn component_browser_searches_large_projects_and_contextual_assignment_stays_focused() {
    let mut h = component_harness();
    for index in 1..64 {
        let revision = h.state().session.revision();
        h.state_mut().session.apply(PLACE_RESISTOR, &json!({"x_nm": index * 2_540_000, "y_nm":0,"expected_revision":revision})).unwrap();
    }
    h.set_size(egui::vec2(1440., 1000.));
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "Find component");
    h.get_by_label("Find component").type_text("R64");
    h.run();
    click(&mut h, "R64 · R");
    h.get_by_label("R64");
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    click(&mut h, "Close");
    let project = h.state().session.project().unwrap();
    let component = project.components().find(|c| c.reference() == "R64").unwrap().id();
    let assignments = project.footprint_assignments();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0]["component"], component);
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    assert!(h.query_by_label("Find component").is_none());
    h.get_by_label("R64");
    h.get_by_label("Assigned · not placed");
    click(&mut h, "Close");
}

/// The narrow browser scrolls the full preview into view, rather than trapping it in a tiny
/// nested scrolling area. Assignment controls remain reachable below the preview.
#[test]
fn narrow_footprint_browser_can_show_the_preview_and_mapping() {
    let mut h = component_harness();
    h.set_size(egui::vec2(600., 850.));
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    h.get_by_label("starter / Resistor 0402 (1005 metric) · r4").scroll_to_me();
    h.run();
    click(&mut h, "starter / Resistor 0402 (1005 metric) · r4");
    h.get_by_label("Footprint preview").scroll_to_me();
    h.run();
    let rect = h.get_by_label("Footprint preview").rect();
    assert!(rect.top() >= h.get_by_label("Assign Footprint").rect().bottom(), "{rect:?}");
    assert!(rect.bottom() < h.get_by_label("Close").rect().top(), "{rect:?}");
    h.get_by_label("Match numbers").scroll_to_me();
    h.run();
    click(&mut h, "Match numbers");
    click(&mut h, "Assign");
    h.get_by_label("Assigned · not placed");
    click(&mut h, "Close");
}

/// A symbol-only instance can choose an external native footprint through the same explicit mapping form.
#[test]
fn external_native_footprint_is_assignable_and_pinned_from_the_chooser() {
    let mut h = component_harness();
    let documents = pcbcraft_engine::library::sources::Documents::from([
        ("library.json".into(), include_str!("../../../../examples/native-parts/library.json").into()),
        ("footprints/resistor.json".into(), include_str!("../../../../examples/native-parts/footprints/resistor-0603.json").into()),
    ]);
    h.state_mut().session.execute("library.load_source", &json!({"source":"external-footprint","documents":documents})).unwrap();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "parts-demo");
    click(&mut h, "parts-demo / Resistor 0603 (1608 metric) · r1");
    for (terminal, pad) in [("Terminal 1", "Pad 1"), ("Terminal 2", "Pad 2")] {
        h.get_by_label(terminal).scroll_to_me();
        h.run();
        click(&mut h, terminal);
        click(&mut h, pad);
    }
    click(&mut h, "Assign");
    assert_eq!(h.state().session.project().unwrap().footprint_assignments()[0]["library"], "parts-demo");
    click(&mut h, "Close");
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().session.execute("library.remove_source", &json!({"source":"external-footprint"})).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.footprint_assignments()[0]["mapping"], json!({"1":"1","2":"2"}));
}

/// The generic chooser edits complete pad sets and NC intent, retaining them on an offline reopen.
#[test]
fn generic_chooser_preserves_multi_pad_and_unconnected_assignments() {
    let mut h = component_harness();
    let mut footprint: Value = serde_json::from_str(include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")).unwrap();
    for number in ["3", "4"] {
        let mut pad = footprint["pads"][0].clone();
        pad["number"] = json!(number);
        pad["center"] = json!({"x_nm":if number == "3" {-2_000_000} else {2_000_000},"y_nm":3_000_000});
        footprint["pads"].as_array_mut().unwrap().push(pad);
    }
    h.state_mut()
        .session
        .execute(
            "library.load_source",
            &json!({"source":"multipad","documents":{
                "library.json":include_str!("../../../../examples/native-parts/library.json"),
                "footprints/four.json":footprint.to_string()
            }}),
        )
        .unwrap();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "Match pin count");
    click(&mut h, "parts-demo / Resistor 0603 (1608 metric) · r1");
    for (terminal, pads) in [("Terminal 1", vec!["Pad 1", "Pad 3"]), ("Terminal 2", vec!["Pad 2"]), ("Unconnected pads", vec!["Pad 4"])] {
        h.get_by_label(terminal).scroll_to_me();
        h.run();
        click(&mut h, terminal);
        for pad in pads {
            click(&mut h, pad);
        }
        h.key_press(Key::Escape);
        h.run();
    }
    click(&mut h, "Assign");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let assignment = h.state().session.project().unwrap().footprint_assignments()[0].clone();
    assert_eq!(assignment["mapping"], json!({"1":["1","3"],"2":"2"}));
    assert_eq!(assignment["unconnected_pads"], json!(["4"]));
    click(&mut h, "Close");
    let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().session.execute("library.remove_source", &json!({"source":"multipad"})).unwrap();
    h.state_mut().session.open_snapshot(decode_documents(&saved).unwrap());
    h.run();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    click(&mut h, "Assign");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    click(&mut h, "Close");
    let app = h.state_mut();
    let target = app.session.project().unwrap().board_target().unwrap();
    app.views.active.open_document(crate::editor_view::DocumentTarget::Board(target), &app.session);
    app.begin_footprint_assignment(None);
    h.run();
    click(&mut h, "Place on board");
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"][0]["unconnected_pads"], json!(["4"]));
}
