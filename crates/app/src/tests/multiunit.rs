//! Drive the same pinned-unit placement command from the real catalog on another sheet.
use super::*;
use crate::editor_view::DocumentTarget;
use pcbcraft_engine::{library::sources, persistence::encode_documents, sheets::CREATE_SHEET};

/// Start with one unit on Main and an empty second sheet addressed by the active view.
fn fixture(width: f32) -> (Harness<'static, PcbApp>, pcbcraft_engine::SheetTarget) {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    app.run(
        sources::LOAD,
        &json!({"source":"units","documents":{
            "library.json":include_str!("../../../../examples/native-parts/library.json"),
            "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
        }}),
    );
    assert!(app.message.is_none(), "{:?}", app.message);
    let definition = app.session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    let revision = app.session.revision();
    app.place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    app.run(CREATE_SHEET, &json!({"name":"Second","expected_revision":app.session.revision()}));
    let target = app.session.project().unwrap().sheets().nth(1).unwrap().target;
    app.views.active.open_document(DocumentTarget::Sheet(target), &app.session);
    let mut harness =
        Harness::builder().with_size(egui::vec2(width, 900.0)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    harness.run();
    (harness, target)
}

/// The catalog lists pinned unplaced units on a sheet that has no existing drawing of the part.
#[test]
fn catalog_places_remaining_unit_on_another_sheet_without_duplicate_part() {
    for width in [1400.0, 700.0] {
        let (mut h, target) = fixture(width);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Existing part units");
        click(&mut h, "U1 B · Illustrative dual amplifier");
        assert!(matches!(&h.state().views.active.schematic.tool,Some(Tool::PlaceSymbol { component:Some(_),unit,.. }) if unit == "b"));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        let canvas = h.get_by_label("Schematic canvas").rect();
        let at = canvas.center() + egui::vec2(40., 40.);
        h.hover_at(at);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
        h.run();
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let project = h.state().session.project().unwrap();
        assert_eq!(project.components().count(), 1);
        let second = project.for_sheet(target).unwrap();
        assert_eq!(second.symbols().len(), 1);
        assert_eq!(second.symbols()[0].unit_id(), "b");
        assert!(h.state().views.active.schematic.tool.is_none());
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="U1 B")));
        h.state_mut().run("edit.undo", &Value::Null);
        h.run();
        let restored = h.state().session.project().unwrap().for_sheet(target).unwrap();
        assert!(restored.symbols().is_empty());
        assert_eq!(restored.components().count(), 1);
        h.state_mut().run("edit.redo", &Value::Null);
        h.run();
        assert_eq!(h.state().session.project().unwrap().components().count(), 1);
    }
}
