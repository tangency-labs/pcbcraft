use super::*;
use pcbcraft_engine::Session;
use serde_json::json;

/// The rendered LED labels stay outside its strokes at every quarter turn and at
/// several detail zoom levels, including when the arrows point down after a half turn.
#[test]
fn rotated_led_labels_clear_the_drawing() {
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "LED labels"})).unwrap();
    session.execute("schematic.place_symbol", &json!({"definition": "led", "x_nm": 0, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
    let project = session.snapshot().unwrap();
    let definition = project.symbols()[0].definition.clone();
    let ctx = egui::Context::default();
    pcbcraft_chrome::fonts::install(&ctx);

    for step in [5.0, 10.16, 20.0] {
        for turns in 0..4 {
            session
                .execute(
                    pcbcraft_engine::SET_SYMBOL_FIELDS,
                    &json!({"symbol":"symbol-2","reference":"D1","value":"LED","rotation":u16::from(turns)*90,"expected_revision":session.revision()}),
                )
                .unwrap();
            let project = session.snapshot().unwrap();
            let symbol = &project.symbols()[0];
            ctx.begin_pass(egui::RawInput::default());
            let painter = ctx.layer_painter(egui::LayerId::background());
            paint_symbol(
                &painter,
                definition.default_unit(),
                pos2(250.0, 250.0),
                step,
                turns,
                Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, egui::Color32::WHITE),
            );
            fields::paint_labels(
                &painter,
                symbol,
                fields::Content { reference: "D1", value: "LED", metadata: &project.component(symbol).unwrap().metadata },
                pos2(250., 250.),
                step,
                egui::Color32::WHITE,
            );
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            let drawing = output
                .shapes
                .iter()
                .filter(|shape| !matches!(shape.shape, egui::Shape::Text(_)))
                .fold(Rect::NOTHING, |bounds, shape| bounds.union(shape.shape.visual_bounding_rect()));
            let labels: Vec<_> = output.shapes.iter().filter(|shape| matches!(shape.shape, egui::Shape::Text(_))).collect();
            assert_eq!(labels.len(), 2);
            assert!(drawing.is_positive());
            for label in labels {
                assert!(
                    !label.shape.visual_bounding_rect().intersects(drawing),
                    "LED label overlaps its drawing at {turns} quarter turns and {step} points per grid step"
                );
            }
        }
    }
}

/// Pin labels affect Fit, but must not enlarge body selection into empty wire space.
#[test]
fn integrated_pin_labels_do_not_enlarge_the_symbol_body_hitbox() {
    use pcbcraft_engine::authoring::{Item, Kind, Primitive};
    let mut item = Item::new(Kind::Symbol).unwrap();
    let before = item.symbol().unwrap();
    if let Primitive::Pin { name, .. } = item.objects.get_mut(&1).unwrap() {
        *name = Some("LONG_PIN_NAME_WITH_THIRTY_CHARS".into());
    }
    let after = item.symbol().unwrap();
    for turns in 0..4 {
        assert_eq!(body(before.default_unit(), turns), body(after.default_unit(), turns));
    }
    assert!(grid_bounds(after.default_unit(), 0).width() > grid_bounds(before.default_unit(), 0).width());
}

/// Generated stroke lettering follows the same zoom cutoff as ordinary schematic labels;
/// leads and body ink remain present at low zoom.
#[test]
fn integrated_pin_text_obeys_the_schematic_detail_threshold() {
    use pcbcraft_engine::authoring::{Item, Kind, Primitive};
    let item = Item::new(Kind::Symbol).unwrap();
    let visible = item.symbol().unwrap();
    let mut hidden = item.clone();
    for p in hidden.objects.values_mut() {
        if let Primitive::Pin { appearance: Some(a), .. } = p {
            a.show_name = false;
            a.show_number = false;
        }
    }
    let hidden = hidden.symbol().unwrap();
    let ctx = egui::Context::default();
    pcbcraft_chrome::fonts::install(&ctx);
    let count = |definition: &SymbolDefinition, step| {
        ctx.begin_pass(egui::RawInput::default());
        paint_symbol(
            &ctx.layer_painter(egui::LayerId::background()),
            definition.default_unit(),
            pos2(100., 100.),
            step,
            0,
            Stroke::new(1., egui::Color32::WHITE),
        );
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        output.shapes.len()
    };
    let low = theme::SCHEMATIC_MIN_GRID_SPACING - 0.1;
    assert!(count(&visible, low) > 0);
    assert_eq!(count(&visible, low), count(&hidden, low));
    assert!(count(&visible, theme::SCHEMATIC_MIN_GRID_SPACING) > count(&hidden, theme::SCHEMATIC_MIN_GRID_SPACING));
}
