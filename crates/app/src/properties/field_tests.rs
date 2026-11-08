//! Real controls and pointer gestures cover instance properties without moving electrical geometry.
use super::*;
use crate::tests::{click, harness, resistor_at_origin};
use egui::{Key, Modifiers, Pos2};
use egui_kittest::{Harness, kittest::Queryable};
use pcbcraft_engine::{
    persistence::{decode_documents, encode_documents},
    selection::Selection,
};

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
fn text_center(h: &Harness<'_, PcbApp>, text: &str) -> Pos2 {
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(field) if field.galley.job.text == text && canvas.contains(field.pos) => Some(field.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap()
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
    click(&mut h, "Fields");
    type_field(&mut h, "Manufacturer", "Example");
    type_field(&mut h, "Manufacturer part number", "ABC-123");
    type_field(&mut h, "Datasheet", "https://example.com/part.pdf");
    click(&mut h, "Add field");
    type_field(&mut h, "Field name", "Supplier code");
    type_field(&mut h, "Field value", "XYZ-789");
    click(&mut h, "Attributes");
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
    click(&mut h, "Fields");
    type_field(&mut h, "Manufacturer", "discarded");
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::E);
    h.run();
    click(&mut h, "Fields");
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
    click(&mut h, "Text appearance");
    click(&mut h, "Reference");
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
    click(&mut h, "Text appearance");
    click(&mut h, "Reset appearance");
    click(&mut h, "Show");
    click(&mut h, "OK");
    assert!(!h.state().session.project().unwrap().symbols()[0].field_layout["reference"].visible);
    assert!(h.output().shapes.iter().all(|s| !matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="R1")));
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
