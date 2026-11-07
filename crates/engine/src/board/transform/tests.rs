use super::*;
use crate::{
    PLACE_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Transform preserves mappings and schematic geometry, updates resolved pads, and shares history.
#[test]
fn transforms_are_atomic_revision_checked_and_persistent() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
    session
        .apply(
            PLACE_FOOTPRINT,
            &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":2}),
        )
        .unwrap();
    let original = session.inspect();
    let original_bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    let board = session.project().unwrap().board_target();
    let transform = json!({"board":board.board,"footprint":"footprint-3","x_nm":2_540_000,"y_nm":-1_270_000,"rotation":90,"expected_revision":3});
    session.apply(SET_FOOTPRINT_TRANSFORM, &transform).unwrap();
    let changed = session.inspect();
    let footprint = &changed["project"]["board"]["footprints"][0];
    assert_eq!(footprint["rotation"], 90);
    assert_eq!(footprint["pins"][0]["x_nm"], 2_540_000);
    assert_eq!(footprint["pins"][0]["y_nm"], 0);
    assert_eq!(footprint["pins"][0]["terminal"], original["project"]["board"]["footprints"][0]["pins"][0]["terminal"]);
    assert_eq!(changed["project"]["schematic"], original["project"]["schematic"]);
    assert!(session.apply(SET_FOOTPRINT_TRANSFORM, &transform).is_err());
    assert_eq!(session.inspect(), changed);
    let bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&bytes["board.json"]).unwrap().contains("pcbcraft-board/4"));
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    assert_eq!(
        bytes.iter().filter(|(path, _)| *path != "board.json").collect::<Vec<_>>(),
        original_bytes.iter().filter(|(path, _)| *path != "board.json").collect::<Vec<_>>()
    );
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original_bytes);
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    let mut noop = transform.clone();
    noop["expected_revision"] = json!(session.revision());
    let revision = session.revision();
    session.apply(SET_FOOTPRINT_TRANSFORM, &noop).unwrap();
    assert_eq!(session.revision(), revision);
    for (key, value) in [
        ("rotation", json!(360)),
        ("x_nm", json!(i64::MAX)),
        ("board", json!("00000000-0000-0000-0000-000000000001")),
        ("footprint", json!("footprint-99")),
    ] {
        let mut invalid = noop.clone();
        invalid[key] = value;
        assert!(session.apply(SET_FOOTPRINT_TRANSFORM, &invalid).is_err());
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
        assert_eq!(session.revision(), revision);
    }
}

/// Configured boards retain stackup and exact pad identities through all orientations; corrupt
/// rotations and attempts to hide orientation under an old format are rejected on load.
#[test]
fn rotated_board_format_preserves_setup_and_rejects_invalid_orientation() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":1})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":2})).unwrap();
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    s.apply(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":3}))
        .unwrap();
    let before = s.inspect();
    let footprint = before["project"]["board"]["footprints"][0]["id"].clone();
    let setup = before["project"]["board"]["setup"].clone();
    let terminal = s.project().unwrap().symbols()[0].terminals()[0];
    let pad = s.project().unwrap().pad_for_terminal(terminal).unwrap();
    for rotation in [45, 90, 180, 270, 0] {
        s.apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":footprint,"x_nm":-2_540_000,"y_nm":1_270_000,"rotation":rotation,"expected_revision":s.revision()}),
        )
        .unwrap();
        let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
        let restored = decode_documents(&bytes).unwrap();
        if rotation == 45 {
            let footprint = restored.board_view(restored.board_target()).unwrap().footprints().next().unwrap();
            assert_eq!(footprint.pads[0].center, crate::schematic::Point { x_nm: -3_438_026, y_nm: 2_168_026 });
        }
        assert_eq!(encode_documents(restored.clone()).unwrap(), bytes);
        assert_eq!(restored.pad_for_terminal(terminal), Some(pad));
        assert_eq!(restored.terminal_for_pad(pad), Some(terminal));
        let mut reopened = Session::default();
        reopened.open_snapshot(restored);
        assert_eq!(reopened.inspect()["project"]["board"]["setup"], setup);
        if rotation == 0 {
            continue;
        }
        assert!(crate::persistence::encode(s.snapshot().unwrap()).is_err());
        for value in [json!(45.5), json!(-90), json!(360), json!(null), json!("90")] {
            let mut bad = bytes.clone();
            let mut board: Value = serde_json::from_slice(&bad["board.json"]).unwrap();
            board["footprints"][0]["rotation"] = value;
            bad.insert("board.json".into(), serde_json::to_vec(&board).unwrap());
            assert!(decode_documents(&bad).is_err());
        }
        let mut old = bytes.clone();
        let mut board: Value = serde_json::from_slice(&old["board.json"]).unwrap();
        board["format"] = json!("pcbcraft-board/2");
        old.insert("board.json".into(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&old).unwrap_err().contains("format 4"));
    }
}
