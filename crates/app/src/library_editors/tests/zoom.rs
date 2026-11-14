use super::*;
use pcbcraft_engine::authoring::Primitive;
use pcbcraft_engine::schematic::Point;

/// Exercise the actual authoring canvas with its normal chrome and input ownership.
fn editor(kind: Kind) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.new_library_item(kind);
    let mut harness = Harness::builder().with_size(egui::vec2(1200., 850.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    harness.run();
    harness
}

/// Both editors must exceed the old schematic ceiling through real wheel and pinch events,
/// preserving the model point under the pointer and leaving authored history untouched.
#[test]
fn definition_detail_zoom_accepts_wheel_and_pinch_without_moving_the_anchor() {
    for kind in [Kind::Footprint, Kind::Symbol] {
        let mut h = editor(kind);
        let id = h.state().library_editors.active.unwrap();
        let before = h.state().library_editors.documents[&id].session.snapshot();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().camera.set_zoom(8.);
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        let camera = &h.state().library_editors.documents[&id].camera;
        let pointer = camera.screen(camera.center(), area) + egui::vec2(37., -21.);
        let anchor = camera.unsnapped(pointer, area);
        h.hover_at(pointer);
        h.run();
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0., 4.),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        assert!(h.state().library_editors.documents[&id].camera.zoom() > 8., "{kind:?} wheel stopped at the old schematic ceiling");
        for _ in 0..5 {
            h.event(egui::Event::Zoom(4.));
            h.run();
        }
        let doc = &h.state().library_editors.documents[&id];
        assert!(doc.camera.zoom() > 64., "{kind:?} stopped at the former PCB detail ceiling");
        assert_eq!(doc.camera.zoom(), crate::canvas_camera::MAX_ZOOM);
        let after = doc.camera.unsnapped(pointer, area);
        assert!((after.x_nm - anchor.x_nm).abs() <= 100 && (after.y_nm - anchor.y_nm).abs() <= 100);
        assert!(Arc::ptr_eq(&before, &doc.session.snapshot()));
        assert_eq!(doc.session.revision(), 0);
        assert!(h.state().session.project().is_none());
    }
}

/// Fit should use the real tiny footprint extents, with its margin remaining in screen space.
#[test]
fn tiny_footprint_fit_fills_the_clear_canvas_and_explicit_zoom_uses_the_same_limit() {
    let mut app = PcbApp::default();
    let footprint = app.session.libraries().footprint("basic-components", "resistor-0402", 4).unwrap();
    let id = app.library_editors.open(Item::from_footprint(&footprint).unwrap(), false).unwrap();
    app.place_library_document(&Context::default(), id, pcbcraft_chrome::workspace::Placement::Here).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let doc = &h.state().library_editors.documents[&id];
    assert!(doc.camera.zoom() > 8., "tiny footprint Fit stayed at {}", doc.camera.zoom());
    let before = doc.session.snapshot();
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "app.library_view", &json!({"document":id,"zoom_percent":102400})).unwrap();
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].camera.zoom(), 1024.);
    assert!(h.state_mut().run_window_command(&ctx, "app.library_view", &json!({"document":id,"zoom_percent":102401})).is_err());
    assert_eq!(h.state().library_editors.documents[&id].camera.zoom(), 1024.);
    h.state_mut().run_window_command(&ctx, "app.library_view", &json!({"document":id,"fit":true})).unwrap();
    h.run();
    assert!(h.state().library_editors.documents[&id].camera.zoom() > 8.);
    assert!(Arc::ptr_eq(&before, &h.state().library_editors.documents[&id].session.snapshot()));
}

/// Magnified pad picking and a fine-grid move must use the same inverse camera transform as ink.
#[test]
fn magnified_footprint_pad_selection_and_fine_move_round_trip() {
    let mut h = editor(Kind::Footprint);
    let id = h.state().library_editors.active.unwrap();
    let before = h.state().library_editors.documents[&id].session.snapshot();
    let (object, original) = before.objects.iter().find(|(_, primitive)| matches!(primitive, Primitive::Pad { .. })).unwrap();
    let (object, original) = (*object, original.anchor());
    {
        let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
        doc.camera.set_zoom(512.);
        doc.camera.set_snap_nm(25_000);
        doc.camera.center_on(original);
    }
    h.run();
    let area = h.get_by_label("Library canvas").rect();
    let at = h.state().library_editors.documents[&id].camera.screen(original, area);
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].selected, Some(object));
    crate::tests::click(&mut h, "Move");
    let target = Point { x_nm: original.x_nm + 25_000, y_nm: original.y_nm - 25_000 };
    let at = h.state().library_editors.documents[&id].camera.screen(target, area);
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects[&object].anchor(), target);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
}
