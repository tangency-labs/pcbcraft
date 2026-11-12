//! Placed-symbol X/Y reflection drives the same shared command from keys, menus and the ribbon.
use super::*;
use pcbcraft_engine::{
    PLACE_SYMBOL, SET_SYMBOL_FIELDS,
    schematic::{MirrorAxis, Point},
};

/// Keep floating panels away from the real canvas while exercising the shipped shortcuts.
fn editor() -> Harness<'static, PcbApp> {
    let mut h = harness();
    new_project(&mut h);
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().browser.open = false;
    h.state_mut().properties_open = false;
    h.run();
    h
}

/// Place one resistor and return its stable placement ID.
fn place_resistor(h: &mut Harness<'_, PcbApp>) -> String {
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.state().session.project().unwrap().symbols()[0].id()
}

/// Resolve the current view's transform instead of relying on a fixed camera scale.
fn screen(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect())
}

/// Open the real canvas context menu with secondary input.
fn right_click(h: &mut Harness<'_, PcbApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Every quarter turn keeps both keys on the shared command, fields stay upright, and Undo/Redo
/// restore the exact before/after snapshots.
#[test]
fn x_and_y_mirror_hovered_symbols_at_every_rotation() {
    for turn in 0..4 {
        for (key, axis) in [(Key::X, MirrorAxis::Horizontal), (Key::Y, MirrorAxis::Vertical)] {
            let mut h = editor();
            let symbol = place_resistor(&mut h);
            let revision = h.state().session.revision();
            h.state_mut().run(
                SET_SYMBOL_FIELDS,
                &json!({"symbol":symbol,"reference":"R7","value":"4.7k","rotation":u16::from(turn)*90,
                    "field_layout":{
                        "reference":{"offset":{"x_nm":-2_540_000,"y_nm":-6_350_000},"visible":true,"show_name":false,"rotation":0,"align":"start","height_nm":1_270_000},
                        "value":{"offset":{"x_nm":2_540_000,"y_nm":6_350_000},"visible":true,"show_name":false,"rotation":0,"align":"end","height_nm":1_270_000}
                    },
                    "expected_revision":revision}),
            );
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let at = screen(&h, before.symbols()[0].position);
            h.hover_at(at);
            h.run();
            h.key_press(key);
            h.run();
            assert!(h.state().message.is_none(), "{axis:?} at {turn} turns: {:?}", h.state().message);

            let (expected_turns, expected_mirrored) = pcbcraft_engine::schematic::mirror_pose(turn, false, axis);
            let placement = &h.state().session.project().unwrap().symbols()[0];
            assert_eq!((placement.turns(), placement.is_mirrored()), (expected_turns, expected_mirrored));
            let texts: Vec<_> = h
                .output()
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            for label in ["R7", "4.7k"] {
                assert!(texts.iter().any(|text| text.galley.job.text == label && text.angle == 0.), "{axis:?} at {turn} turns keeps {label} legible");
            }

            let mirrored = h.state().session.snapshot().unwrap();
            click(&mut h, "Undo");
            assert_same_content(&before, &h.state().session.snapshot().unwrap());
            click(&mut h, "Redo");
            assert_same_content(&mirrored, &h.state().session.snapshot().unwrap());
        }
    }
}

/// The key, context menu and ribbon all reach the same edit, including the selected-symbol
/// fallback when the pointer is not over the drawing.
#[test]
fn selected_symbols_mirror_from_keys_menus_and_the_ribbon() {
    let mut h = editor();
    let symbol = place_resistor(&mut h);
    let at = screen(&h, h.state().session.project().unwrap().symbols()[0].position);
    click_canvas_at(&mut h, at);
    assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some(symbol.as_str()));

    let canvas = h.get_by_label("Schematic canvas").rect();
    h.hover_at(canvas.left_top() + egui::vec2(40., 40.));
    h.run();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    let placement = &h.state().session.project().unwrap().symbols()[0];
    assert!(placement.is_mirrored());
    assert_eq!(placement.turns(), 0);
    assert_eq!(h.state().session.revision(), revision + 1);

    right_click(&mut h, at);
    click(&mut h, "Mirror Vertically");
    let placement = &h.state().session.project().unwrap().symbols()[0];
    assert!(!placement.is_mirrored());
    assert_eq!(placement.turns(), 2);

    click_tab(&mut h, "Capture");
    click(&mut h, "Mirror selected symbol top to bottom (Y)");
    let placement = &h.state().session.project().unwrap().symbols()[0];
    assert!(placement.is_mirrored());
    assert_eq!(placement.turns(), 0);

    let mirrored = h.state().session.snapshot().unwrap();
    click(&mut h, "Undo");
    click(&mut h, "Undo");
    click(&mut h, "Undo");
    let restored = &h.state().session.project().unwrap().symbols()[0];
    assert!(!restored.is_mirrored());
    assert_eq!(restored.turns(), 0);
    click(&mut h, "Redo");
    click(&mut h, "Redo");
    click(&mut h, "Redo");
    assert_same_content(&mirrored, &h.state().session.snapshot().unwrap());
}

/// With no target, X/Y publish nothing; Escape after a completed edit removes no authored state.
#[test]
fn mirror_keys_without_a_target_or_after_escape_change_nothing() {
    let mut h = editor();
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.hover_at(canvas.center());
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    for key in [Key::X, Key::Y] {
        h.key_press(key);
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().session.revision(), revision);
    }

    place_resistor(&mut h);
    let at = screen(&h, Point { x_nm: 0, y_nm: 0 });
    h.hover_at(at);
    h.run();
    h.key_press(Key::X);
    h.run();
    let mirrored = h.state().session.snapshot().unwrap();
    h.key_press(Key::Escape);
    h.run();
    assert_same_content(&mirrored, &h.state().session.snapshot().unwrap());
}
