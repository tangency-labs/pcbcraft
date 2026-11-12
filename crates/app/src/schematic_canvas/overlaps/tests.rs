use super::*;
use pcbcraft_engine::Session;
use serde_json::json;

/// Cell pruning agrees with exhaustive positive-area bounds checks, including edge-only
/// contact, negative cells, a large fallback body and many coincident bodies.
#[test]
fn drawing_candidates_preserve_intersections_and_touching_boundaries() {
    let mut entries = vec![
        ("a".into(), Rect::from_min_max(pos2(-2., -2.), pos2(2., 2.))),
        ("touch".into(), Rect::from_min_max(pos2(2., -2.), pos2(6., 2.))),
        ("separate".into(), Rect::from_min_max(pos2(40., 30.), pos2(42., 31.))),
        ("large".into(), Rect::from_min_max(pos2(-1000., -1000.), pos2(-500., -500.))),
    ];
    entries.extend((0..256).map(|i| (format!("stack-{i}"), Rect::from_min_max(pos2(100., 100.), pos2(104., 104.)))));
    let index = Index::new(entries);
    assert_eq!(index.groups.len(), 1);
    assert_eq!(index.groups[0].len(), 256);
    for (i, (_, rect)) in index.entries.iter().enumerate() {
        let expected = index.entries.iter().enumerate().any(|(j, (_, other))| i != j && rect.intersect(*other).is_positive());
        assert_eq!(index.overlaps.contains(&i), expected, "body {i}");
    }
    assert!(index.any(Rect::from_center_size(pos2(-700., -700.), vec2(1., 1.)), None));
    assert!(!index.any(Rect::from_center_size(pos2(-300., -300.), vec2(1., 1.)), None));
}

/// Drawing bounds follow rotations and placement, while empty drawing bodies do not turn
/// integrated pins into overlap obstacles. Cache invalidation follows immutable sheet geometry.
#[test]
fn bounds_and_cached_warnings_follow_geometry_without_document_edits() {
    let ctx = egui::Context::default();
    pcbcraft_chrome::fonts::install(&ctx);
    ctx.begin_pass(egui::RawInput::default());
    let painter = ctx.layer_painter(egui::LayerId::background());
    let step = 10.;
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    for _ in 0..2 {
        session
            .apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()}))
            .unwrap();
    }
    let project = session.snapshot().unwrap();
    let symbol = &project.symbols()[0];
    let unit = symbol.unit_definition();
    let normal = bounds(&painter, unit, 0, symbol.position, step).unwrap();
    let rotated = bounds(&painter, unit, 1, symbol.position, step).unwrap();
    assert_eq!(normal.size(), vec2(rotated.height(), rotated.width()));
    let mut pins_only = unit.clone();
    pins_only.drawing.clear();
    assert!(bounds(&painter, &pins_only, 0, symbol.position, step).is_none());
    let mut cache = Cache::default();
    let first = cache.get(&project, session.revision(), &painter, step);
    let again = cache.get(&project, session.revision(), &painter, step);
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(first.overlaps.len(), 2);
    assert_eq!(first.groups[0].len(), 2);
    session
        .apply(pcbcraft_engine::MOVE_SYMBOL, &json!({"symbol":symbol.id(),"x_nm":25_400_000,"y_nm":0,"expected_revision":session.revision()}))
        .unwrap();
    let next = cache.get(&session.snapshot().unwrap(), session.revision(), &painter, step);
    assert!(!Arc::ptr_eq(&first, &next));
    assert!(next.overlaps.is_empty());
    let documents = pcbcraft_engine::persistence::encode_documents(project.clone()).unwrap();
    let reopened = pcbcraft_engine::persistence::decode_documents(&documents).unwrap();
    let restored = cache.get(&reopened, session.revision(), &painter, step);
    assert!(!Arc::ptr_eq(&next, &restored));
    assert_eq!(restored.overlaps.len(), 2);
    let zoomed = cache.get(&reopened, session.revision(), &painter, step * 2.);
    assert!(!Arc::ptr_eq(&restored, &zoomed));
    assert!(Arc::ptr_eq(&zoomed, &cache.get(&reopened, session.revision(), &painter, step * 2.)));
    ctx.end_pass().textures_delta.clear();
    ctx.set_pixels_per_point(2.);
    ctx.begin_pass(egui::RawInput::default());
    let scaled = cache.get(&reopened, session.revision(), &ctx.layer_painter(egui::LayerId::background()), step * 2.);
    assert!(!Arc::ptr_eq(&zoomed, &scaled));
    ctx.end_pass().textures_delta.clear();
}

/// Native drawing text uses the same glyph extents as painting, including Unicode,
/// upright quarter turns, both font clamps and display scales. A distant text edge alone can
/// trigger committed and preview warnings without changing non-text bounds or low-zoom ink.
#[test]
fn drawing_text_extents_match_painting_and_warn_beyond_the_anchor() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let mut unit = session.snapshot().unwrap().symbols()[0].unit_definition().clone();
    let ctx = egui::Context::default();
    pcbcraft_chrome::fonts::install(&ctx);
    let at = Point { x_nm: 7_620_000, y_nm: -2_540_000 };
    let text = "WWWWWWWWWWWWW Ωµ";
    for scale in [1., 2.] {
        ctx.set_pixels_per_point(scale);
        for step in [4., 5., 10.16, 40.] {
            for turns in 0..4 {
                for align in [TextAlign::Start, TextAlign::Middle, TextAlign::End] {
                    unit.drawing = vec![Shape::Text(at, text.into(), align)];
                    ctx.begin_pass(egui::RawInput::default());
                    let painter = ctx.layer_painter(egui::LayerId::background());
                    let world = bounds(&painter, &unit, turns, Point { x_nm: 0, y_nm: 0 }, step).unwrap();
                    let center = pos2(400., 400.);
                    paint_symbol(&painter, &unit, center, step, turns, Stroke::new(1., egui::Color32::WHITE));
                    let mut output = ctx.end_pass();
                    output.textures_delta.clear();
                    let rendered = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(shape_text) if shape_text.galley.text() == text => Some(shape_text.visual_bounding_rect()),
                            _ => None,
                        })
                        .fold(Rect::NOTHING, Rect::union);
                    if step < theme::SCHEMATIC_MIN_GRID_SPACING {
                        assert!(!rendered.is_finite());
                        assert!(world.width() <= 0.051 && world.height() <= 0.051);
                        continue;
                    }
                    assert!(rendered.is_positive());
                    let measured = Rect::from_min_max(center + world.min.to_vec2() * step, center + world.max.to_vec2() * step);
                    assert!(measured.expand(0.001).contains_rect(rendered), "{align:?}, turns {turns}, step {step}, scale {scale}");
                    // Place a small neighboring body near the far text edge, away from its anchor.
                    let (anchor_x, _) = turn(grid(at).0, grid(at).1, turns);
                    let anchor_x = center.x + anchor_x * step;
                    let far = if (rendered.right() - anchor_x).abs() > (rendered.left() - anchor_x).abs() {
                        rendered.right_center()
                    } else {
                        rendered.left_center()
                    };
                    let target = Rect::from_center_size(((far - center) / step).to_pos2(), vec2(0.1, 0.1));
                    assert!(!turn_rect(drawing_bounds(&unit).expand(0.025), turns).intersects(target));
                    let committed = Index::new(vec![("text".into(), world), ("neighbor".into(), target)]);
                    assert_eq!(committed.overlaps.len(), 2);
                    let preview = Index::new(vec![("neighbor".into(), target)]);
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let mut camera = Camera::default();
                        camera.set_zoom(step / camera.unit_step());
                        preview.preview(ui, &camera, ui.max_rect(), &unit, turns, Point { x_nm: 0, y_nm: 0 });
                    });
                    output.textures_delta.clear();
                    assert!(
                        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Symbol overlap"))
                    );
                }
            }
        }
    }
}
