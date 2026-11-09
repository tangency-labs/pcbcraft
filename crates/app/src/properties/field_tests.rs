//! Real controls and pointer gestures cover instance properties without moving electrical geometry.
use super::*;
use crate::tests::{harness, resistor_at_origin};
use egui::{Key, Modifiers, Pos2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    persistence::{decode_documents, encode_documents},
    selection::Selection,
};

/// Scroll controls into the shared form body before clicking, leaving the fixed footer visible.
fn click(h: &mut Harness<'_, PcbApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run();
    h.get_by_label(label).click();
    h.run();
}

/// Keep field drags away from panels while testing the real schematic canvas.
fn fixture() -> Harness<'static, PcbApp> {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.state_mut().browser.open = false;
    h.state_mut().properties_open = false;
    h.set_size(egui::vec2(1200., 1000.));
    h.run();
    h
}

/// Locate actual painted text, rather than assuming a fixed label offset or camera zoom.
fn field_ink(h: &Harness<'_, PcbApp>, text: &str) -> egui::epaint::TextShape {
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(field) if field.galley.job.text == text && canvas.contains(field.pos) => Some(field.clone()),
            _ => None,
        })
        .unwrap()
}

/// Pick the visual center of the same glyphs that the canvas paints.
fn text_center(h: &Harness<'_, PcbApp>, text: &str) -> Pos2 {
    field_ink(h, text).visual_bounding_rect().center()
}

/// Recover the legacy side anchor from rendered ink without depending on field-layout state.
/// On a vertical symbol Reference attaches at its lower left and Value at its upper left.
fn vertical_field_anchor(h: &Harness<'_, PcbApp>, text: &str, reference: bool) -> Pos2 {
    let ink = field_ink(h, text);
    let corner = if reference { egui::vec2(0., ink.galley.size().y) } else { egui::Vec2::ZERO };
    ink.pos + egui::emath::Rot2::from_angle(ink.angle) * corner
}

/// Replace input through accessibility; every character must stay owned by the form.
fn type_field(h: &mut Harness<'_, PcbApp>, name: &str, text: &str) {
    h.get_by_label(name).scroll_to_me();
    h.run();
    h.get_by_label(name).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.get_by_label(name).type_text(text);
    h.run();
}

/// Reference and value drags commit independently, preview outside history and never move pins.
#[test]
fn dragging_reference_and_value_preserves_the_symbol_and_undoes_once() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":25_400_000,"y_nm":0,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
    );
    h.run();
    for (text, id) in [("R1", "reference"), ("R", "value")] {
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        let start = text_center(&h, text);
        let end = start + egui::vec2(101.6, 50.8);
        h.drag_at(start);
        h.run();
        h.hover_at(end);
        h.run();
        assert!(matches!(h.state().views.active.schematic.tool, Some(crate::schematic_canvas::Tool::Field { .. })));
        assert_eq!(h.state().session.revision(), revision);
        assert_ne!(text_center(&h, text), start, "drag preview follows pointer");
        h.drop_at(end);
        h.run();
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(after.symbols()[0].field_layout[id].offset.is_some());
        assert_eq!(after.symbols()[0].position, before.symbols()[0].position);
        assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
        assert!(before.same_connectivity(&after));
        assert_eq!(
            before.wires().iter().map(|wire| before.wire_path(wire)).collect::<Vec<_>>(),
            after.wires().iter().map(|wire| after.wire_path(wire)).collect::<Vec<_>>()
        );
        let docs = encode_documents(after.clone()).unwrap();
        assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Redo");
        assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    }
}

/// Cancel, outside release, focus loss and a concurrent revision must discard previews.
#[test]
fn field_drag_cancellation_never_mutates_the_document() {
    for mode in ["escape", "outside", "focus", "stale"] {
        let mut h = fixture();
        let start = text_center(&h, "R1");
        h.drag_at(start);
        h.run();
        h.hover_at(start + egui::vec2(100., 60.));
        h.run();
        let before = h.state().session.inspect();
        let revision = h.state().session.revision();
        match mode {
            "escape" => h.key_press(Key::Escape),
            "outside" => {}
            "focus" => h.event(egui::Event::WindowFocused(false)),
            _ => h
                .state_mut()
                .run(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R7","value":"R","expected_revision":revision})),
        }
        h.run();
        let expected = if mode == "stale" { h.state().session.inspect() } else { before };
        h.drop_at(if mode == "outside" { egui::pos2(-20., -20.) } else { start + egui::vec2(100., 60.) });
        h.run();
        assert_eq!(h.state().session.inspect(), expected, "{mode}");
        assert!(h.state().session.project().unwrap().symbols()[0].field_layout.is_empty());
    }
}

/// Fields, text appearance and attributes apply as one instance transaction; Cancel and invalid
/// duplicate field names leave history, other instances and the library definition unchanged.
#[test]
fn component_fields_and_attributes_edit_through_the_expanded_form() {
    let mut h = fixture();
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Manufacturer", "Example");
    type_field(&mut h, "Manufacturer part number", "ABC-123");
    type_field(&mut h, "Datasheet", "https://example.com/part.pdf");
    click(&mut h, "Add field");
    type_field(&mut h, "Field name", "Supplier code");
    type_field(&mut h, "Supplier code", "XYZ-789");
    click(&mut h, "Do not populate");
    click(&mut h, "Exclude from bill of materials");
    let revision = h.state().session.revision();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "OK");
    assert_eq!(h.state().session.revision(), revision + 1);
    let project = h.state().session.snapshot().unwrap();
    let component = project.component(&project.symbols()[0]).unwrap();
    assert_eq!(component.metadata.fields["manufacturer"].value, "Example");
    assert!(component.metadata.fields.values().any(|f| f.name == "Supplier code" && f.value == "XYZ-789"));
    assert!(component.metadata.attributes.do_not_populate && component.metadata.attributes.exclude_from_bom);
    assert_eq!(project.symbols()[0].pins(), before.symbols()[0].pins());
    let docs = encode_documents(project.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Redo");
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Manufacturer", "discarded");
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Field name", "Reference");
    click(&mut h, "OK");
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
}

/// Text visibility, explicit physical offsets, size and reset are edited independently of body X/Y.
#[test]
fn field_appearance_controls_preserve_physical_positions_and_reset_to_auto() {
    let mut h = fixture();
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.key_press(Key::E);
    h.run();
    if h.query_by_label("Text height").is_none() {
        click(&mut h, "Text appearance");
    }
    click(&mut h, "Automatic position");
    type_field(&mut h, "Text X", "5.08 mm");
    type_field(&mut h, "Text Y", "-100 mil");
    type_field(&mut h, "Text height", "1 mm");
    click(&mut h, "Show name");
    click(&mut h, "OK");
    let project = h.state().session.snapshot().unwrap();
    let layout = &project.symbols()[0].field_layout["reference"];
    assert_eq!(layout.offset, Some(Point { x_nm: 5_080_000, y_nm: -2_540_000 }));
    assert_eq!(layout.height_nm, 1_000_000);
    assert!(layout.show_name);
    text_center(&h, "Reference: R1");
    h.key_press(Key::E);
    h.run();
    if h.query_by_label("Text height").is_none() {
        click(&mut h, "Text appearance");
    }
    click(&mut h, "Reset appearance");
    click(&mut h, "Show Reference");
    click(&mut h, "OK");
    assert!(!h.state().session.project().unwrap().symbols()[0].field_layout["reference"].visible);
    assert!(h.output().shapes.iter().all(|s| !matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="R1")));
}

/// Appearance changes and R over reference/value text retain the automatic side anchor on
/// vertical symbols. Resetting a saved layout must restore the exact legacy text placement.
#[test]
fn vertical_symbols_keep_automatic_field_alignment_through_appearance_edits() {
    use pcbcraft_engine::schematic::fields::Layout;
    for rotation in [90, 270] {
        let mut h = fixture();
        let rev = h.state().session.revision();
        h.state_mut()
            .session
            .apply(
                pcbcraft_engine::SET_SYMBOL_FIELDS,
                &json!({"symbol":"symbol-2","reference":"R1","value":"R","rotation":rotation,"expected_revision":rev}),
            )
            .unwrap();
        h.run();
        let reference = vertical_field_anchor(&h, "R1", true);
        let value = vertical_field_anchor(&h, "R", false);
        let original = h.state().session.snapshot().unwrap();
        for (text, field, expected, is_reference) in [("R1", "reference", reference, true), ("R", "value", value, false)] {
            h.hover_at(text_center(&h, text));
            h.run();
            h.key_press(Key::R);
            h.run();
            let project = h.state().session.project().unwrap();
            assert_eq!(project.symbols()[0].field_layout[field].rotation, 90);
            assert!(project.symbols()[0].field_layout[field].offset.is_none());
            assert!(vertical_field_anchor(&h, text, is_reference).distance(expected) < 0.01, "{rotation} {field}");
            click(&mut h, "Undo");
        }
        let rev = h.state().session.revision();
        let layout = Layout { show_name: true, height_nm: 2_540_000, align: pcbcraft_engine::schematic::TextAlign::End, ..Default::default() };
        h.state_mut()
            .session
            .apply(
                pcbcraft_engine::SET_SYMBOL_FIELDS,
                &json!({"symbol":"symbol-2","reference":"R1","value":"R","field_layout":{"reference":layout,"value":layout},"expected_revision":rev}),
            )
            .unwrap();
        h.run();
        assert!(vertical_field_anchor(&h, "Reference: R1", true).distance(reference) < 0.01);
        assert!(vertical_field_anchor(&h, "Value: R", false).distance(value) < 0.01);
        h.state_mut().select(Selection::one_symbol("symbol-2".into()));
        h.hover_at(h.get_by_label("Schematic canvas").rect().left_top() + egui::vec2(20., 20.));
        h.run();
        h.key_press(Key::E);
        h.run();
        if h.query_by_label("Reset appearance").is_none() {
            click(&mut h, "Text appearance");
        }
        click(&mut h, "Reset appearance");
        click(&mut h, "OK");
        assert!(vertical_field_anchor(&h, "R1", true).distance(reference) < 0.01);
        let project = h.state().session.project().unwrap();
        assert_eq!(project.symbols()[0].pins(), original.symbols()[0].pins());
        assert!(project.same_connectivity(&original));
    }
}

/// A label over the body remains independently draggable in either pane, and a disabled symbol
/// filter prevents the same gesture from acquiring an electrical editing target.
#[test]
fn overlapping_field_drags_belong_to_the_focused_split_pane() {
    let mut h = fixture();
    h.set_size(egui::vec2(1500., 1000.));
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R1","value":"R",
        "field_layout":{"reference":pcbcraft_engine::schematic::fields::Layout{offset:Some(Point{x_nm:0,y_nm:0}),..Default::default()}},"expected_revision":revision}));
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    for pane in [0, 1] {
        let canvas = h.get_by_label(&format!("Schematic canvas {}", pane + 1)).rect();
        let start = h
            .output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == "R1" && canvas.contains(t.pos) => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap();
        let before = h.state().session.snapshot().unwrap();
        h.drag_at(start);
        h.run();
        h.hover_at(start + egui::vec2(35., 50.));
        h.run();
        h.drop_at(start + egui::vec2(35., 50.));
        h.run();
        assert_eq!(h.state().views.active.id(), crate::editor_view::ViewId(pane));
        let after = h.state().session.snapshot().unwrap();
        assert_ne!(after.symbols()[0].field_layout, before.symbols()[0].field_layout);
        assert_eq!(after.symbols()[0].position, before.symbols()[0].position);
    }
    h.state_mut().views.active.filters.symbols = false;
    h.run();
    let canvas = h.get_by_label("Schematic canvas 2").rect();
    let start = h
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "R1" && canvas.contains(text.pos) => Some(text.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    let before = h.state().session.snapshot().unwrap();
    h.drag_at(start);
    h.run();
    h.hover_at(start + egui::vec2(40., 35.));
    h.run();
    assert!(!matches!(h.state().views.active.schematic.tool, Some(crate::schematic_canvas::Tool::Field { .. })));
    h.drop_at(start + egui::vec2(40., 35.));
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Default and explicitly positioned labels keep a fixed physical ratio to the symbol at zoom.
#[test]
fn symbol_text_scales_with_the_canvas_instead_of_staying_at_screen_size() {
    let mut h = fixture();
    let mut sizes = Vec::new();
    for zoom in [0.75, 1., 4.] {
        h.state_mut().views.active.camera.set_zoom(zoom);
        h.run();
        let step = h.state().views.active.camera.unit_step();
        let size = h
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "R1" => Some(text.galley.job.sections[0].format.font_id.size),
                _ => None,
            })
            .unwrap();
        assert!((size / step - 1.).abs() < 0.01, "{zoom}: {size}/{step}");
        sizes.push(size);
    }
    assert!((sizes[2] / sizes[1] - 4.).abs() < 0.01);
}

/// Send two complete primary clicks within the platform double-click interval.
fn double_click(h: &mut Harness<'_, PcbApp>, point: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(point);
    h.run();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.input_mut().time = Some(h.ctx.input(|i| i.time) + 0.03);
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers });
            h.run_steps(1);
        }
    }
    h.input_mut().time = None;
    h.run();
}

/// Bodies and visible fields open the same revision-bound properties form without editing the
/// document. Nearby pin hit radii must not turn a small capacitor's first body click into a wire.
#[test]
fn double_click_routes_bodies_to_properties_and_fields_to_inline_text() {
    for (definition, turns, zoom) in [("resistor", 0, 1.), ("capacitor", 0, 0.5), ("capacitor", 90, 0.5), ("led", 90, 1.), ("npn", 0, 1.)] {
        let mut h = fixture();
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::DELETE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":revision}));
        let revision = h.state().session.revision();
        h.state_mut()
            .run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":definition,"rotation":turns,"x_nm":0,"y_nm":0,"expected_revision":revision}));
        h.state_mut().views.active.camera.set_zoom(zoom);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let point = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, h.get_by_label("Schematic canvas").rect());
        double_click(&mut h, point, Modifiers::NONE);
        assert!(matches!(&h.state().dialog,Dialog::EditFields(edit) if edit.definition.id==definition), "{definition} {turns} {zoom}");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Cancel");
    }
    let mut h = fixture();
    let point = text_center(&h, "R1");
    let before = h.state().session.snapshot().unwrap();
    double_click(&mut h, point, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::None));
    h.get_by_label("Edit Reference");
    assert!(h.state().views.active.schematic.text_edit.is_some());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Modified clicks and disabled symbol picking retain selection semantics rather than opening
/// properties, and explicit wire tools keep their own double-click gesture.
#[test]
fn double_click_properties_respects_filters_modifiers_and_active_tools() {
    for mode in ["filter", "shift", "wire"] {
        let mut h = fixture();
        let point = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, h.get_by_label("Schematic canvas").rect());
        if mode == "filter" {
            h.state_mut().views.active.filters.symbols = false;
            h.run();
        }
        if mode == "wire" {
            h.key_press(Key::W);
            h.run();
        }
        double_click(&mut h, point, if mode == "shift" { Modifiers::SHIFT } else { Modifiers::NONE });
        assert!(matches!(h.state().dialog, Dialog::None), "{mode}");
        assert!(h.state().views.active.schematic.text_edit.is_none(), "{mode}");
    }
}

/// Rotating hovered text updates only its physical appearance, including a custom visible field.
/// Existing pins/wires, automatic anchors and undo remain independent of the symbol's rotation.
#[test]
fn visible_field_rotation_keeps_symbol_geometry_wiring_and_other_text() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":25_400_000,"y_nm":0,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":revision}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R1","value":"R",
        "metadata":{"fields":{"00000000-0000-4000-8000-000000000001":{"name":"Note","value":"Visible note"}},"attributes":pcbcraft_engine::schematic::fields::Attributes::default()},
        "field_layout":{"00000000-0000-4000-8000-000000000001":{"offset":{"x_nm":0,"y_nm":6_350_000},"visible":true,"show_name":false,"rotation":0,"align":"middle","height_nm":1_270_000}},
        "expected_revision":revision}));
    h.run();
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    for (text, field) in [("R1", "reference"), ("R", "value"), ("Visible note", "00000000-0000-4000-8000-000000000001")] {
        let before = h.state().session.snapshot().unwrap();
        let start = text_center(&h, text);
        h.hover_at(start);
        h.run();
        h.key_press(Key::R);
        h.run();
        let after = h.state().session.snapshot().unwrap();
        let original = &before.symbols()[0];
        let symbol = &after.symbols()[0];
        assert_eq!(symbol.field_layout[field].rotation, 90);
        assert_eq!(symbol.field_layout[field].offset, original.field_layout.get(field).and_then(|l| l.offset));
        assert_eq!(symbol.position, original.position);
        assert_eq!(symbol.pins(), original.pins());
        assert_eq!(symbol.turns(), original.turns());
        assert!(before.same_connectivity(&after));
        assert_eq!(
            before.wires().iter().map(|w| before.wire_path(w)).collect::<Vec<_>>(),
            after.wires().iter().map(|w| after.wire_path(w)).collect::<Vec<_>>()
        );
        let mut expected = original.field_layout.clone();
        expected.entry(field.into()).or_default().rotation = 90;
        assert_eq!(symbol.field_layout, expected);
        let docs = encode_documents(after.clone()).unwrap();
        assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// R while dragging rotates the preview; drop publishes position and angle in one undo step,
/// while Escape discards both and leaves connectivity untouched.
#[test]
fn dragging_text_can_rotate_without_committing_until_drop() {
    for cancel in [true, false] {
        let mut h = fixture();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        let start = text_center(&h, "R1");
        let end = start + egui::vec2(50., 50.);
        h.drag_at(start);
        h.run();
        h.hover_at(end);
        h.run();
        h.key_press(Key::R);
        h.run();
        assert_eq!(h.state().session.revision(), revision);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text=="R1" && (text.angle + std::f32::consts::FRAC_PI_2).abs()<0.001)));
        if cancel {
            h.key_press(Key::Escape);
            h.run();
        }
        h.drop_at(end);
        h.run();
        if cancel {
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        } else {
            let after = h.state().session.snapshot().unwrap();
            assert_eq!(h.state().session.revision(), revision + 1);
            assert_eq!(after.symbols()[0].field_layout["reference"].rotation, 90);
            assert!(after.symbols()[0].field_layout["reference"].offset.is_some());
            assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Inspect host output on the key-down frame before the release frame replaces it.
fn datasheet_urls(h: &mut Harness<'_, PcbApp>) -> Vec<String> {
    h.event(egui::Event::Key { key: Key::D, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    let urls = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
            _ => None,
        })
        .collect();
    h.event(egui::Event::Key { key: Key::D, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run();
    urls
}

/// Opening URL fields is a host action and does not apply drafts. D uses the saved datasheet
/// for a hovered/selected component and is suppressed while a form or active tool owns input.
#[test]
fn field_links_and_datasheet_shortcut_open_without_editing_the_project() {
    let mut h = fixture();
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Datasheet", "https://example.com/data.pdf");
    let before = h.state().session.snapshot().unwrap();
    h.get_by_label("Open Datasheet link").scroll_to_me();
    h.run();
    h.get_by_label("Open Datasheet link").click();
    h.step();
    assert!(
        h.output().platform_output.commands.iter().any(|c| matches!(c,egui::OutputCommand::OpenUrl(url) if url.url=="https://example.com/data.pdf"))
    );
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Add link");
    assert!(h.get_by_label("Open Link 1 link").accesskit_node().is_disabled());
    type_field(&mut h, "Field name", "Documentation");
    type_field(&mut h, "Documentation", "https://example.com/guide");
    h.get_by_label("Open Documentation link").click();
    h.step();
    assert!(
        h.output().platform_output.commands.iter().any(|c| matches!(c,egui::OutputCommand::OpenUrl(url) if url.url=="https://example.com/guide"))
    );
    click(&mut h, "Add link");
    type_field(&mut h, "Link 1", "https://example.com/supplier");
    h.get_by_label("Open Link 1 link").click();
    h.step();
    assert!(
        h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(url) if url.url=="https://example.com/supplier"))
    );
    click(&mut h, "OK");
    let saved = h.state().session.snapshot().unwrap();
    let metadata = &saved.component(&saved.symbols()[0]).unwrap().metadata;
    assert_eq!(metadata.fields.values().filter(|field| pcbcraft_chrome::properties::web_link(&field.value).is_some()).count(), 3);
    let docs = encode_documents(saved.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    for hovered in [true, false] {
        let pointer = if hovered { text_center(&h, "R1") } else { h.get_by_label("Schematic canvas").rect().left_bottom() - egui::vec2(-30., 30.) };
        h.hover_at(pointer);
        h.run();
        assert_eq!(datasheet_urls(&mut h), ["https://example.com/data.pdf"]);
        assert!(Arc::ptr_eq(&saved, &h.state().session.snapshot().unwrap()));
    }
    h.key_press(Key::W);
    h.run();
    assert!(datasheet_urls(&mut h).is_empty());
    h.key_press(Key::Escape);
    h.run();
    h.state_mut().views.active.filters.symbols = false;
    assert!(datasheet_urls(&mut h).is_empty());
    h.state_mut().views.active.filters.symbols = true;
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Datasheet", "javascript:alert(1)");
    assert!(h.get_by_label("Open Datasheet link").accesskit_node().is_disabled());
    assert!(datasheet_urls(&mut h).is_empty());
    click(&mut h, "OK");
    assert!(datasheet_urls(&mut h).is_empty());
    assert!(h.state().message.as_ref().is_some_and(|m| m.contains("datasheet link")));
}

/// The main form shares Preferences sections, aligns the footprint action with field values
/// and keeps placement/attribute headings level while stacking them in a narrow host.
#[test]
fn property_sections_and_value_columns_stay_aligned() {
    for size in [egui::vec2(1440., 1000.), egui::vec2(600., 850.)] {
        let mut h = fixture();
        h.set_size(size);
        h.state_mut().select(Selection::one_symbol("symbol-2".into()));
        h.key_press(Key::E);
        h.run();
        let reference = h.get_by_label("Reference").rect();
        let footprint = h.get_by_label("Choose footprint…").rect();
        assert!((reference.left() - footprint.left()).abs() < 1.);
        assert!((reference.right() - footprint.right()).abs() < 1.);
        let placement = h.get_by_label("PLACEMENT").rect();
        let attributes = h.get_by_label("ATTRIBUTES").rect();
        if size.x > 1000. {
            assert!((placement.top() - attributes.top()).abs() < 1.);
        } else {
            assert!(attributes.top() > placement.top());
        }
    }
}

/// Property actions open the existing library-source manager or an independent symbol copy.
/// Unapplied edits disable those exits, and stale forms cannot fork another selected definition.
#[test]
fn library_property_actions_keep_pinned_geometry_and_pending_edits_safe() {
    let mut h = fixture();
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    let before = h.state().session.snapshot().unwrap();
    h.key_press(Key::E);
    h.run();
    type_field(&mut h, "Value", "pending");
    click(&mut h, "Library");
    for label in ["Edit symbol copy…", "Manage libraries…"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::E);
    h.run();
    click(&mut h, "Library");
    click(&mut h, "Manage libraries…");
    assert!(matches!(h.state().dialog, Dialog::LibrarySources));
    click(&mut h, "Close Library sources");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::E);
    h.run();
    click(&mut h, "Library");
    h.state_mut().session.apply("project.rename", &json!({"name":"Concurrent"})).unwrap();
    h.run();
    click(&mut h, "Edit symbol copy…");
    assert!(h.state().library_editors.active_kind().is_none());
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    click(&mut h, "Cancel");
    h.key_press(Key::E);
    h.run();
    click(&mut h, "Library");
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Edit symbol copy…");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().library_editors.active_kind(), Some(pcbcraft_engine::authoring::Kind::Symbol));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Enter publishes the addressed reference/value once, preserving electrical geometry and every
/// other field. The old glyph is replaced by a canvas text box, without a dialog or new viewport.
#[test]
fn inline_symbol_fields_commit_once_and_keep_their_owner() {
    for (old, edited, label) in [("R1", "R42", "Edit Reference"), ("R", "4.7k", "Edit Value")] {
        let mut h = fixture();
        let before = h.state().session.snapshot().unwrap();
        let point = text_center(&h, old);
        double_click(&mut h, point, Modifiers::NONE);
        assert!(matches!(h.state().dialog, Dialog::None));
        let rect = h.get_by_label(label).rect();
        assert!(h.get_by_label("Schematic canvas").rect().contains_rect(rect));
        assert!(rect.expand(30.).contains(point));
        assert_eq!(
            h.output().shapes.iter().filter(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text==old)).count(),
            1,
            "the editor must replace the canvas text"
        );
        type_field(&mut h, label, edited);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let rev = h.state().session.revision();
        h.key_press(Key::Enter);
        h.run();
        assert!(h.state().views.active.schematic.text_edit.is_none());
        assert_eq!(h.state().session.revision(), rev + 1);
        let after = h.state().session.snapshot().unwrap();
        let component = after.component(&after.symbols()[0]).unwrap();
        if label == "Edit Reference" {
            assert_eq!(component.reference(), edited);
            assert_eq!(component.value(), "R");
        } else {
            assert_eq!(component.value(), edited);
            assert_eq!(component.reference(), "R1");
        }
        assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
        assert_eq!(after.symbols()[0].field_layout, before.symbols()[0].field_layout);
        assert!(after.same_connectivity(&before));
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Redo");
        assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    }
}

/// E edits visible custom text without selecting or opening its component. Blank optional text
/// remains a named metadata field, and native persistence retains both content and appearance.
#[test]
fn inline_custom_fields_use_the_shared_metadata_command() {
    let mut h = fixture();
    let id = "00000000-0000-4000-8000-000000000001";
    let rev = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::SET_SYMBOL_FIELDS,&json!({"symbol":"symbol-2","reference":"R1","value":"R","metadata":{"fields":{id:{"name":"Note","value":"Visible note"}},"attributes":pcbcraft_engine::schematic::fields::Attributes::default()},"field_layout":{id:pcbcraft_engine::schematic::fields::Layout{offset:Some(Point{x_nm:0,y_nm:6_350_000}),..Default::default()}},"expected_revision":rev})).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let point = text_center(&h, "Visible note");
    h.hover_at(point);
    h.run();
    h.key_press(Key::E);
    h.run();
    h.get_by_label("Edit Note");
    assert!(matches!(h.state().dialog, Dialog::None));
    type_field(&mut h, "Edit Note", "Changed note");
    h.key_press(Key::Enter);
    h.run();
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(after.component(&after.symbols()[0]).unwrap().metadata.fields[id].value, "Changed note");
    assert_eq!(after.symbols()[0].field_layout, before.symbols()[0].field_layout);
    let docs = encode_documents(after).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
}

/// Escape, focus loss, outside click, a new opening or a competing revision discard drafts.
/// The outside release is consumed rather than selecting a node or starting a wire.
#[test]
fn inline_drafts_cancel_without_misrouting_canvas_input() {
    for mode in ["escape", "focus", "outside", "stale", "reopen"] {
        let mut h = fixture();
        let point = text_center(&h, "R1");
        double_click(&mut h, point, Modifiers::NONE);
        type_field(&mut h, "Edit Reference", "R42");
        match mode {
            "escape" => h.key_press(Key::Escape),
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "outside" => {
                let project = h.state().session.project().unwrap();
                let terminal = project.symbols()[0].terminals()[0];
                let point = project.terminal_position(terminal).unwrap();
                let pointer = h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect());
                assert!(!h.get_by_label("Edit Reference").rect().contains(pointer));
                h.hover_at(pointer);
                h.run();
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                    h.run();
                }
            }
            "reopen" => {
                let snapshot = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(snapshot);
            }
            _ => {
                h.state_mut().session.apply("project.rename", &json!({"name":"Concurrent"})).unwrap();
            }
        }
        let before = h.state().session.snapshot().unwrap();
        h.run();
        assert!(h.state().views.active.schematic.text_edit.is_none(), "{mode}");
        assert!(h.state().views.active.schematic.tool.is_none(), "{mode}");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "{mode}");
    }
}

/// Native command validation rejects invalid strings in place and leaves the draft available
/// for correction; accepting unchanged text creates no history entry.
#[test]
fn invalid_inline_text_stays_editable_without_partial_changes() {
    let mut h = fixture();
    let before = h.state().session.snapshot().unwrap();
    let point = text_center(&h, "R1");
    double_click(&mut h, point, Modifiers::NONE);
    type_field(&mut h, "Edit Reference", "");
    h.key_press(Key::Enter);
    h.run();
    h.get_by_label("Edit Reference");
    assert!(h.state().views.active.schematic.text_edit.is_some());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    type_field(&mut h, "Edit Reference", "R1");
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().views.active.schematic.text_edit.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// The live command path respects inline input ownership; after cancellation the same revision
/// remains usable and executes through the normal shared field command.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_text_owns_live_command_input_until_finished() {
    use pcbcraft_mcp::live::Editor;
    let mut h = fixture();
    assert_eq!(crate::window_commands::inspect_editor(h.state())["text_editing"], false);
    let point = text_center(&h, "R1");
    double_click(&mut h, point, Modifiers::NONE);
    let before = h.state().session.snapshot().unwrap();
    let ctx = h.ctx.clone();
    assert_eq!(crate::window_commands::inspect_editor(h.state())["text_editing"], true);
    assert!(crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }.refusal("project.rename").is_some());
    let request = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::SET_SYMBOL_FIELDS,"params":{"symbol":"symbol-2","reference":"R1","value":"new","expected_revision":h.state().session.revision()}}});
    assert!(pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).is_err());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(crate::window_commands::inspect_editor(h.state())["text_editing"], false);
    assert!(crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }.refusal("project.rename").is_none());
    pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap();
    assert_eq!(h.state().session.project().unwrap().component(&h.state().session.project().unwrap().symbols()[0]).unwrap().value(), "new");
}

/// In-place editors render and commit inside the focused split, and changing panes cancels a
/// pending draft rather than moving the editor or applying it to another view's selection.
#[test]
fn inline_text_belongs_to_its_split_canvas() {
    let mut h = fixture();
    h.set_size(egui::vec2(1500., 1000.));
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let canvas = h.get_by_label("Schematic canvas 2").rect();
    let point = h
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(text) if text.galley.job.text == "R1" && canvas.contains(text.visual_bounding_rect().center()) => {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap();
    double_click(&mut h, point, Modifiers::NONE);
    assert_eq!(h.state().views.active.id(), crate::ViewId(1));
    assert!(canvas.contains_rect(h.get_by_label("Edit Reference").rect()));
    type_field(&mut h, "Edit Reference", "R42");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().session.project().unwrap().components().next().unwrap().reference(), "R42");
    click(&mut h, "Undo");
    double_click(&mut h, point, Modifiers::NONE);
    type_field(&mut h, "Edit Reference", "discarded");
    let before = h.state().session.snapshot().unwrap();
    let other = h.get_by_label("Schematic canvas 1").rect().center();
    crate::tests::click_canvas_at(&mut h, other);
    assert!(h.state().views.view(crate::ViewId(1)).unwrap().schematic.text_edit.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Native power Values use ordinary field drag/rotation and inline editing. The terminal
/// keeps its wires while the Value continues to name the actual global electrical net.
#[test]
fn native_power_value_uses_common_field_tools_and_inline_editing() {
    let mut h = fixture();
    let d = h.state().session.libraries().latest_symbol("power", "gnd").unwrap();
    let rev = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_SYMBOL,&json!({"library":"power","definition":"gnd","definition_revision":d.revision(),"definition_digest":pcbcraft_engine::library::sources::symbol_digest(&d).unwrap(),"x_nm":12*pcbcraft_engine::schematic::GRID_NM,"y_nm":0,"expected_revision":rev}));
    let p = h.state().session.snapshot().unwrap();
    let power = p.symbols()[1].clone();
    let pin = power.terminals()[0];
    let rev = h.state().session.revision();
    h.state_mut()
        .run(pcbcraft_engine::CONNECT_PINS, &json!({"from":p.symbols()[0].terminals()[1].inspect(),"to":pin.inspect(),"expected_revision":rev}));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let start = text_center(&h, "GND");
    let to = start + egui::vec2(80., 60.);
    h.drag_at(start);
    h.run();
    h.hover_at(to);
    h.run();
    h.key_press(Key::R);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::schematic_canvas::Tool::Field { .. })));
    h.drop_at(to);
    h.run();
    let moved = h.state().session.snapshot().unwrap();
    assert_eq!(moved.symbols()[1].position, power.position);
    assert_eq!(moved.symbols()[1].turns(), power.turns());
    assert_eq!(moved.symbols()[1].field_layout["value"].rotation, 90);
    assert_eq!(moved.wire_path(&moved.wires()[0]), before.wire_path(&before.wires()[0]));
    let point = text_center(&h, "GND");
    double_click(&mut h, point, Modifiers::NONE);
    assert!(
        matches!(&h.state().views.active.schematic.text_edit,Some(edit) if matches!(&edit.target,crate::inline_text::Target::Symbol{symbol,field} if *symbol==power.id() && field=="value"))
    );
    type_field(&mut h, "Edit Value", "AGND");
    h.key_press(Key::Enter);
    h.run();
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(after.component(&after.symbols()[1]).unwrap().value(), "AGND");
    assert_eq!(after.connectivity().net_name(after.connectivity().net_index_for(pin).unwrap()), "AGND");
    assert_eq!(after.wire_path(&after.wires()[0]), before.wire_path(&before.wires()[0]));
    assert!(after.net_labels().is_empty());
    let docs = encode_documents(after).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&moved, &h.state().session.snapshot().unwrap());
    click(&mut h, "Undo");
    crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
}
