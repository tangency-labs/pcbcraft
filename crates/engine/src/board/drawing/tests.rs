use super::*;
use crate::{
    ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};

/// Exercise technical layers on both configured boards and legacy boards with assigned parts.
fn session(configured: bool, placed: bool) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    if configured {
        s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let component = s.inspect()["project"]["components"][0]["id"].clone();
    let definition = crate::board::definition::starter("resistor-0402").unwrap();
    s.apply(
        ASSIGN_FOOTPRINT,
        &json!({"component":component,"library":definition.library,
        "definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"2","2":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    if placed {
        s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":5_000_000,"y_nm":-2_000_000,"expected_revision":s.revision()}))
            .unwrap();
    }
    s
}

/// Commands address the layer UUID, never its display name or position.
fn edge(s: &Session) -> Uuid {
    s.project().unwrap().boards().next().unwrap().layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id
}

fn points() -> Value {
    json!([{"x_nm":-20_000_001,"y_nm":-15_000_001},{"x_nm":20_000_002,"y_nm":-15_000_001},{"x_nm":15_000_002,"y_nm":15_000_002}])
}

/// Open and closed geometry survives folders/archives, preserves assignments, and shares history.
#[test]
fn edge_geometry_is_layer_addressed_persistent_and_atomic() {
    for configured in [false, true] {
        for placed in [false, true] {
            let mut s = session(configured, placed);
            let before = encode_documents(s.snapshot().unwrap()).unwrap();
            let old: Value = serde_json::from_slice(&before["board.json"]).unwrap();
            let layer = edge(&s);
            s.apply(ADD_DRAWING, &json!({"layer":layer,"points":points(),"closed":false,"expected_revision":s.revision()})).unwrap();
            let graphic = s.project().unwrap().boards().next().unwrap().drawings()[0].clone();
            assert_eq!(graphic.layer, layer);
            assert_eq!(s.project().unwrap().boards().next().unwrap().setup().is_some(), configured);
            let after = encode_documents(s.snapshot().unwrap()).unwrap();
            let board: Value = serde_json::from_slice(&after["board.json"]).unwrap();
            assert_eq!(board["format"], "pcbcraft-board/9");
            assert_eq!(board["assignments"], old["assignments"]);
            assert_eq!(board["footprints"], old["footprints"]);
            for (path, bytes) in &before {
                if path != "board.json" {
                    assert_eq!(&after[path], bytes);
                }
            }
            assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
            assert_eq!(encode_documents(decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project).unwrap(), after);
            let revision = s.revision();
            s.apply(SET_DRAWING, &json!({"drawing":graphic.id,"points":graphic.points,"closed":false,"expected_revision":revision})).unwrap();
            assert_eq!(s.revision(), revision);
            s.apply(SET_DRAWING, &json!({"drawing":graphic.id,"points":graphic.points,"closed":true,"expected_revision":revision})).unwrap();
            s.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
            s.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
            s.apply("edit.redo", &json!({})).unwrap();
            s.apply(DELETE_DRAWING, &json!({"drawing":graphic.id,"expected_revision":s.revision()})).unwrap();
            assert!(s.project().unwrap().boards().next().unwrap().drawings().is_empty());
            assert!(crate::persistence::encode(s.snapshot().unwrap()).is_err());
        }
    }
}

/// Unknown/copper layers, stale edits and invalid references are refused without touching history.
#[test]
fn invalid_geometry_and_storage_are_rejected() {
    let mut s = session(true, false);
    let before = s.inspect();
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let layer = edge(&s);
    let copper = s.project().unwrap().boards().next().unwrap().layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Copper).unwrap().id;
    let base = json!({"layer":layer,"points":points(),"closed":false,"expected_revision":s.revision()});
    for (key, value) in [
        ("layer", json!(Uuid::from_u128(1))),
        ("layer", json!(copper)),
        ("expected_revision", json!(0)),
        ("points", json!([{"x_nm":0,"y_nm":0},{"x_nm":0,"y_nm":0}])),
        ("board", json!(Uuid::from_u128(1))),
    ] {
        let mut bad = base.clone();
        bad[key] = value;
        assert!(s.apply(ADD_DRAWING, &bad).is_err(), "{bad}");
        assert_eq!(s.inspect(), before);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    }
    s.apply(ADD_DRAWING, &base).unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    for wrong_format in [false, true] {
        let mut changed = original.clone();
        let mut board: Value = serde_json::from_slice(&changed["board.json"]).unwrap();
        if wrong_format {
            board["format"] = json!("pcbcraft-board/7");
        } else {
            board["graphics"]["items"][0]["layer"] = json!(Uuid::from_u128(1));
        }
        changed.insert("board.json".into(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&changed).is_err());
    }
}

/// Adding copper later on an empty board preserves the technical layer's existing identity.
#[test]
fn later_stackup_setup_preserves_edge_layer() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    let layer = edge(&s);
    s.apply(ADD_DRAWING, &json!({"layer":layer,"points":points(),"closed":true,"expected_revision":s.revision()})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    assert_eq!(edge(&s), layer);
    assert_eq!(s.project().unwrap().boards().next().unwrap().drawings()[0].layer, layer);
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
}

/// Technical-layer widths survive native/clipboard codecs alongside upstream reference
/// overrides, while Undo restores the old format and malformed stroke values are atomic.
#[test]
fn physical_graphics_preserve_widths_reference_overrides_and_history() {
    let mut s = session(true, true);
    let view = s.project().unwrap().boards().next().unwrap();
    let footprint = view.footprints().next().unwrap().id.clone();
    let board = view.target.board;
    let layer = view.layers().into_iter().find(|l| l.purpose == LayerPurpose::Silkscreen).unwrap().id;
    s.apply(
        crate::board::reference::SET_REFERENCE_TEXT,
        &json!({"board":board,"footprint":footprint,"appearance":{"text":"Input"},"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&before["board.json"]).unwrap()["format"], "pcbcraft-board/15");
    for width in [0, 9_999, 10_000_001] {
        assert!(
            s.apply(ADD_DRAWING, &json!({"layer":layer,"points":points(),"closed":false,"stroke_nm":width,"expected_revision":s.revision()}))
                .is_err()
        );
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    s.apply(ADD_DRAWING, &json!({"layer":layer,"points":points(),"closed":false,"stroke_nm":250_000,"expected_revision":s.revision()})).unwrap();
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&after["board.json"]).unwrap()["format"], "pcbcraft-board/16");
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    let view = s.project().unwrap().boards().next().unwrap();
    let id = view.drawings()[0].id;
    let fragment = crate::board::clipboard::BoardFragment::capture(
        s.project().unwrap(),
        &crate::board::clipboard::BoardSelection { drawings: [id].into(), ..Default::default() },
        false,
    )
    .unwrap();
    let text = fragment.to_text().unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["format"], "pcbcraft-clipboard/2");
    assert_eq!(value["drawings"][0]["stroke_nm"], 250_000);
    assert_eq!(crate::board::clipboard::BoardFragment::from_text(&text).unwrap().to_text().unwrap(), text);
    let mut malformed = after.clone();
    let mut value: Value = serde_json::from_slice(&malformed["board.json"]).unwrap();
    value["format"] = json!("pcbcraft-board/15");
    malformed.insert("board.json".into(), serde_json::to_vec(&value).unwrap());
    assert!(decode_documents(&malformed).is_err());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// Each visible manufacturing surface receives only its own explicit board artwork;
/// the outline and opposite surfaces remain byte-identical.
#[test]
fn technical_graphics_reach_their_fabrication_layers() {
    let mut s = session(true, false);
    s.apply(ADD_DRAWING, &json!({"layer":edge(&s),"points":points(),"closed":true,"expected_revision":s.revision()})).unwrap();
    for (purpose, label) in [(LayerPurpose::Silkscreen, "Silkscreen_Top"), (LayerPurpose::Mask, "Mask_Top"), (LayerPurpose::Paste, "Paste_Top")] {
        let view = s.project().unwrap().boards().next().unwrap();
        let layer = view.layers().into_iter().find(|l| l.purpose == purpose && l.side == Some(super::super::setup::Side::Front)).unwrap().id;
        let before = view.fabrication_outputs().unwrap();
        s.apply(ADD_DRAWING,&json!({"layer":layer,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1_000_000,"y_nm":0}],"closed":false,"stroke_nm":250_000,"expected_revision":s.revision()})).unwrap();
        let after = s.project().unwrap().boards().next().unwrap().fabrication_outputs().unwrap();
        let changed: Vec<_> = before.iter().zip(&after).filter(|(a, b)| a.contents != b.contents).map(|(_, b)| b.name.as_str()).collect();
        assert_eq!(changed.len(), 1, "{label}: {changed:?}");
        assert!(changed[0].contains(label));
        let saved = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    }
}
