use super::*;
use crate::{
    PLACE_FOOTPRINT, PLACE_SYMBOL, SET_SILKSCREEN, Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::json;

/// Build a physical placement through public commands, without relying on saved file edits.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let symbol = s.project().unwrap().symbols()[0].id();
    s.apply(
        PLACE_FOOTPRINT,
        &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    s
}

/// Physical settings are one undoable board edit, survive both storage forms, and reject stale,
/// foreign-board or malformed requests without changing geometry, history or document bytes.
#[test]
fn settings_are_atomic_undoable_and_persistent() {
    let mut s = session();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    s.apply(SET_SILKSCREEN, &json!({"settings":Settings::default(),"expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let style = json!({"text_height_nm":1_200_000,"stroke_width_nm":180_000,"pad_clearance_nm":400_000});
    s.apply(SET_SILKSCREEN, &json!({"settings":style,"expected_revision":revision})).unwrap();
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&after[&crate::persistence::test_board_path(&after)]).unwrap().contains("pcbcraft-board/6"));
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    assert_eq!(encode_documents(decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project).unwrap(), after);
    assert_eq!(s.inspect()["project"]["board"]["silkscreen"], style);
    let board = s.project().unwrap().board_view(s.project().unwrap().board_target().unwrap()).unwrap();
    assert_eq!(board.silkscreen().stroke_width_nm, 180_000);
    assert!(board.footprints().next().unwrap().graphics.iter().filter_map(|g| g.stroke_nm).all(|width| width == 180_000));
    let current = s.revision();
    for params in [
        json!({"settings":style,"expected_revision":revision}),
        json!({"settings":style,"board":"00000000-0000-0000-0000-000000000001","expected_revision":current}),
        json!({"settings":{"text_height_nm":1_000_000,"stroke_width_nm":600_000,"pad_clearance_nm":250_000},"expected_revision":current}),
        json!({"settings":{"text_height_nm":0,"stroke_width_nm":150_000,"pad_clearance_nm":250_000},"expected_revision":current}),
    ] {
        assert!(s.apply(SET_SILKSCREEN, &params).is_err());
        assert_eq!(s.revision(), current);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
    assert!(crate::persistence::encode(s.snapshot().unwrap()).is_err());
}

/// Ink keeps the requested physical height; clipping accounts for both line thickness and pad
/// clearance rather than drawing a thin-looking stroke across exposed copper.
#[test]
fn lettering_height_and_silk_to_pad_clearance_are_physical() {
    let style = Settings::default();
    let paths = lettering("H1", 0, style);
    let low = paths.iter().flatten().map(|p| p.y_nm).min().unwrap();
    let high = paths.iter().flatten().map(|p| p.y_nm).max().unwrap();
    assert_eq!(high - low + style.stroke_width_nm, style.text_height_nm);
    let mut pad = super::super::definition::starter("resistor-0402").unwrap().pads[0].clone();
    pad.center = Point { x_nm: 0, y_nm: 0 };
    pad.width_nm = 2_000_000;
    pad.height_nm = 2_000_000;
    pad.rotation = 0;
    let clipped = clipped(vec![vec![Point { x_nm: -3_000_000, y_nm: 0 }, Point { x_nm: 3_000_000, y_nm: 0 }]], &[pad], style);
    assert_eq!(
        clipped,
        vec![
            vec![Point { x_nm: -3_000_000, y_nm: 0 }, Point { x_nm: -1_325_000, y_nm: 0 }],
            vec![Point { x_nm: 1_325_000, y_nm: 0 }, Point { x_nm: 3_000_000, y_nm: 0 }]
        ]
    );
}

/// Q's descender must stay intact above the footprint instead of being clipped away near pad 3,
/// which would change the visible reference from Q1 to O1.
#[test]
fn technical_reference_retains_the_q_tail_above_sot23() {
    let mut s = session();
    s.apply(PLACE_SYMBOL, &json!({"definition":"npn","x_nm":30_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let symbol = s.project().unwrap().symbols().last().unwrap().id();
    s.apply(
        PLACE_FOOTPRINT,
        &json!({"symbol":symbol,"definition":"sot23","mapping":{"1":"1","2":"2","3":"3"},"x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    let project = s.project().unwrap();
    let footprint = project.board_view(project.board_target().unwrap()).unwrap().footprints().find(|f| f.reference == "Q1").unwrap();
    assert!(footprint.graphics.iter().any(|g| g.stroke_nm.is_some() && g.points.len() == 2 && g.points.iter().all(|p| p.y_nm < -1_350_000) && {
        let dx = g.points[1].x_nm - g.points[0].x_nm;
        let dy = g.points[1].y_nm - g.points[0].y_nm;
        dx.abs() > 150_000 && dy.abs() > 100_000 && dx.signum() == dy.signum()
    }));
}
