use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, persistence::encode_documents};

/// Keep the target clear of the project Browser and Properties at ordinary and compact sizes.
fn editor(width: f32, dark: bool) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.preferences.theme = if dark { crate::ThemeChoice::Dark } else { crate::ThemeChoice::Light };
    app.run("project.new", &json!({}));
    app.parts.open = false;
    app.properties_open = false;
    app.run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Overlapping placement stays allowed, warns in the actual preview and remains marked after
/// placement. Choosing a hidden symbol is read-only and subsequent dragging moves that symbol.
#[test]
fn symbol_overlap_warns_and_the_chooser_can_recover_a_hidden_symbol() {
    for (width, dark, repeat) in [(1400., true, true), (1400., false, false), (700., true, false), (700., false, true)] {
        let mut h = editor(width, dark);
        h.state_mut().parts.set_repeat(repeat);
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Resistor");
        click(&mut h, "Place");
        let canvas = h.get_by_label("Schematic canvas").rect();
        let point = h.state().views.active.camera.screen(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }, canvas);
        h.hover_at(point);
        h.run();
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Symbol overlap")));
        click_canvas_at(&mut h, point);
        assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
        assert!(h.query_all_by_label("Overlapping symbols").count() > 0);
        if repeat {
            h.key_press(Key::Escape);
            h.run();
        }
        let before = h.state().session.snapshot().unwrap();
        let bytes = encode_documents(before.clone()).unwrap();
        let first = before.symbols()[0].id();
        let second = before.symbols()[1].id();
        let first_label = format!("Select {}", before.symbols()[0].display_reference(&before.reference(&before.symbols()[0])));
        let second_label = format!("Select {}", before.symbols()[1].display_reference(&before.reference(&before.symbols()[1])));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
        let menu = h.get_by_label("Select symbol").rect();
        h.hover_at(menu.center());
        h.run_steps(20);
        h.run();
        h.get_by_label(&second_label);
        click(&mut h, &first_label);
        assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some(first.as_str()));
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        let destination = point + egui::vec2(100., 50.);
        h.drag_at(point);
        h.run();
        h.hover_at(destination);
        h.run();
        h.drop_at(destination);
        h.run();
        let project = h.state().session.snapshot().unwrap();
        assert_ne!(project.symbols().iter().find(|symbol| symbol.id() == first).unwrap().position, before.symbols()[0].position);
        assert_eq!(project.symbols().iter().find(|symbol| symbol.id() == second).unwrap().position, before.symbols()[1].position);
        assert_eq!(h.query_all_by_label("Overlapping symbols").count(), 0);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert!(h.query_all_by_label("Overlapping symbols").count() > 0);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
}

/// Identical command-created bodies share one marker. A revision change invalidates the
/// captured chooser instead of leaving stale references actionable over the current document.
#[test]
fn coincident_symbols_coalesce_markers_and_stale_choices_are_not_reused() {
    let mut h = editor(1400., true);
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    assert_eq!(h.query_all_by_label("Overlapping symbols").count(), 1);
    let canvas = h.get_by_label("Schematic canvas").rect();
    let point = h.state().views.active.camera.screen(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }, canvas);
    h.hover_at(point);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    let row = h.get_by_label("Select symbol").rect();
    h.hover_at(row.center());
    h.run_steps(20);
    h.run();
    h.get_by_label("Select R1");
    h.get_by_label("Select R2");
    h.state_mut().run("project.rename", &json!({"name":"Changed while choosing"}));
    let after = h.state().session.snapshot().unwrap();
    h.run();
    assert!(h.query_by_label("Select symbol").is_none());
    assert!(h.query_by_label("Select R1").is_none());
    assert!(std::sync::Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
}
