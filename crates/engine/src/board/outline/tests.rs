use super::*;
use crate::{
    SET_DRAWING, Session,
    persistence::{decode_documents, encode_documents},
};
use serde_json::{Value, json};

/// The earlier local format remains byte-stable until an actual graphic edit materializes it.
#[test]
fn legacy_rectangle_migrates_on_graphic_edit_without_losing_geometry() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let mut project = s.snapshot().unwrap();
    std::sync::Arc::make_mut(&mut std::sync::Arc::make_mut(&mut project).board).outline =
        Some(Outline::rectangle(Point { x_nm: -20_000_001, y_nm: -15_000_001 }, Point { x_nm: 20_000_002, y_nm: 15_000_002 }).unwrap());
    let before = encode_documents(project).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&before["board.json"]).unwrap()["format"], "pcbcraft-board/8");
    s.open_snapshot(decode_documents(&before).unwrap());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let view = s.project().unwrap().boards().next().unwrap();
    let mut graphic = view.drawings()[0].clone();
    let revision = s.revision();
    s.apply(SET_DRAWING, &json!({"drawing":graphic.id,"points":graphic.points,"closed":true,"expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    graphic.points[1].x_nm += 1_000_000;
    s.apply(SET_DRAWING, &json!({"drawing":graphic.id,"points":graphic.points,"closed":true,"expected_revision":s.revision()})).unwrap();
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&after["board.json"]).unwrap()["format"], "pcbcraft-board/9");
    assert_eq!(s.project().unwrap().boards().next().unwrap().drawings()[0], graphic);
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}
