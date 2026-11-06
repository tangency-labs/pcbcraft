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
    let definition = project.symbols()[0].definition;
    let ctx = egui::Context::default();
    pcbcraft_chrome::fonts::install(&ctx);

    for step in [5.0, 10.16, 20.0] {
        for turns in 0..4 {
            ctx.begin_pass(egui::RawInput::default());
            let painter = ctx.layer_painter(egui::LayerId::background());
            paint_symbol(&painter, definition, pos2(250.0, 250.0), step, turns, egui::Color32::WHITE, Some(("D1", "LED")));
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
