use super::*;
use crate::{
    net_labels::Style,
    persistence::{decode_documents, encode_documents},
};
use std::sync::Arc;

/// Prepare and paste through shared commands so original text never bypasses native validation.
#[test]
fn plain_net_names_have_explicit_scope_spacing_suffix_and_one_undo_step() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Text target"})).unwrap();
    let before = s.snapshot().unwrap();
    let result = s
        .execute(
            LABELS,
            &json!({"text":"VCC\r\nDATA\r\nGND\r\n","style":"global","spacing_nm":2_540_000,"net_name_suffix":"_B","expected_revision":s.revision()}),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let text = result["clipboard"].as_str().unwrap();
    assert_eq!(Fragment::from_text(text).unwrap().counts(), (0, 0, 3));
    s.apply(crate::clipboard::PASTE, &json!({"text":text,"x_nm":5_080_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.net_labels().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["VCC_B", "DATA_B", "GND_B"]);
    assert!(p.net_labels().iter().all(|l| l.style == Style::Global));
    assert_eq!(p.net_labels()[1].at, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(bytes, encode_documents(decode_documents(&bytes).unwrap()).unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Header/unit checks, nanometer rounding and optional inversion produce deterministic geometry.
#[test]
fn numeric_tables_preserve_dimensions_and_normalize_closed_endpoints() {
    let text = "x_mm,y_mm\r\n\"0\",0\r\n25.4,0\r\n25.4,12.7\r\n0,0\r\n";
    let p = coordinate_points(text, CoordinateUnits::Mm, true, true).unwrap();
    assert_eq!(p, [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 25_400_000, y_nm: 0 }, Point { x_nm: 25_400_000, y_nm: -12_700_000 }]);
    assert_eq!(
        coordinate_points("x_mil\ty_mil\n0\t0\n1000\t500", CoordinateUnits::Mil, false, false).unwrap()[1],
        Point { x_nm: 25_400_000, y_nm: 12_700_000 }
    );
    assert_eq!(coordinate_points("0,0\n0.000001,1e-6", CoordinateUnits::Mm, false, false).unwrap()[1], Point { x_nm: 1, y_nm: 1 });
    assert!(coordinate_points(text, CoordinateUnits::Mil, true, false).is_err());
}

/// Outline preparation changes no destination state/setup and pastes through the ordinary
/// board fragment contract, with one undo operation and native round-trip geometry.
#[test]
fn coordinate_preparation_and_paste_keep_destination_setup() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":s.revision()})).unwrap();
    let before = s.snapshot().unwrap();
    let result = s.execute(OUTLINE, &json!({"text":"0,0\n10,0\n10,5","units":"mm","closed":true,"expected_revision":s.revision()})).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let text = result["clipboard"].as_str().unwrap();
    assert_eq!(BoardFragment::from_text(text).unwrap().counts(), (0, 1, 0, 0));
    s.apply(crate::board::clipboard::PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap();
    let b = p.boards().next().unwrap();
    assert_eq!(b.setup(), before.boards().next().unwrap().setup());
    assert_eq!(b.drawings()[0].points[2], Point { x_nm: 10_000_000, y_nm: 5_000_000 });
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(bytes, encode_documents(decode_documents(&bytes).unwrap()).unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Malformed/resource-exhausting text is refused as a complete batch, including native EDA data,
/// formulas, mixed/missing columns, nonfinite numbers, duplicate vertices and excessive geometry.
#[test]
fn invalid_external_text_never_changes_the_destination() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let before = s.snapshot().unwrap();
    for text in [
        "",
        "0,0\nNaN,0",
        "0,0\ninf,0",
        "0,0\n1e100,0",
        "0,0\n=1+1,0",
        "0,0,0\n1,1,1",
        "0,0\n\n1,1",
        "0,0\n0,0",
        "0,0\n1,1,",
        "{\"format\":\"pcbcraft-clipboard/99\"}",
        "<svg/>",
        "(structured EDA clipboard)",
    ] {
        assert!(s.execute(OUTLINE, &json!({"text":text,"units":"mm","closed":false,"expected_revision":s.revision()})).is_err(), "{text}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    for text in ["", "NET\n\nOTHER", "X\tY", "{\"format\":\"pcbcraft-clipboard/99\"}"] {
        assert!(s.execute(LABELS, &json!({"text":text,"style":"local","spacing_nm":2_540_000,"expected_revision":s.revision()})).is_err());
    }
    let huge = "X".repeat(MAX_TEXT_BYTES + 1);
    assert!(s.execute(LABELS, &json!({"text":huge,"style":"local","spacing_nm":2_540_000,"expected_revision":s.revision()})).is_err());
    assert!(coordinate_points(&(0..258).map(|i| format!("{i},0")).collect::<Vec<_>>().join("\n"), CoordinateUnits::Mm, false, false).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Preparation is project-independent and read-only even through apply; stale requests are
/// rejected before conversion and cannot consume IDs or change save/history state.
#[test]
fn preparation_availability_and_revisions_are_explicit() {
    let mut s = Session::default();
    assert!(s.available(LABELS));
    assert!(s.available(OUTLINE));
    let params = json!({"text":"A\nB","style":"local","spacing_nm":2_540_000,"expected_revision":0});
    assert_eq!(s.apply(LABELS, &params).unwrap(), 0);
    assert!(s.project().is_none());
    assert!(s.execute(LABELS, &json!({"text":"A","style":"local","spacing_nm":2_540_000,"expected_revision":1})).is_err());
    assert_eq!(s.revision(), 0);
}
