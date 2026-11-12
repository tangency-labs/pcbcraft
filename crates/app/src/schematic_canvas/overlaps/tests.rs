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
    let normal = bounds(unit, 0, symbol.position).unwrap();
    let rotated = bounds(unit, 1, symbol.position).unwrap();
    assert_eq!(normal.size(), vec2(rotated.height(), rotated.width()));
    let mut pins_only = unit.clone();
    pins_only.drawing.clear();
    assert!(bounds(&pins_only, 0, symbol.position).is_none());
    let mut cache = Cache::default();
    let first = cache.get(&project, session.revision());
    let again = cache.get(&project, session.revision());
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(first.overlaps.len(), 2);
    assert_eq!(first.groups[0].len(), 2);
    session
        .apply(pcbcraft_engine::MOVE_SYMBOL, &json!({"symbol":symbol.id(),"x_nm":25_400_000,"y_nm":0,"expected_revision":session.revision()}))
        .unwrap();
    let next = cache.get(&session.snapshot().unwrap(), session.revision());
    assert!(!Arc::ptr_eq(&first, &next));
    assert!(next.overlaps.is_empty());
    let documents = pcbcraft_engine::persistence::encode_documents(project.clone()).unwrap();
    let reopened = pcbcraft_engine::persistence::decode_documents(&documents).unwrap();
    let restored = cache.get(&reopened, session.revision());
    assert!(!Arc::ptr_eq(&next, &restored));
    assert_eq!(restored.overlaps.len(), 2);
}
