use super::*;
use crate::{
    INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_OUTLINE, SET_SILKSCREEN,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::json;

/// Exercise the boundary independently of board setup and alongside all existing board features.
fn session(configured: bool) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    if configured {
        s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let symbol = s.project().unwrap().symbols()[0].id();
        s.apply(
            PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"resistor-0603","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
        )
        .unwrap();
        s.apply(
            SET_SILKSCREEN,
            &json!({"settings":{"text_height_nm":1_200_000,"stroke_width_nm":180_000,"pad_clearance_nm":300_000},"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    s
}

/// Original geometry stays within the boundary but is not the boundary's source of identity.
fn rectangle() -> Value {
    json!({"shape":"rectangle","min":{"x_nm":-20_000_001,"y_nm":-15_000_001},"max":{"x_nm":20_000_002,"y_nm":15_000_002}})
}

/// Setting, clearing and undoing an outline retain every other document and old no-op bytes;
/// folders and archives both preserve the new boundary and reject lossy single-file export.
#[test]
fn outline_is_board_local_undoable_and_persistent() {
    for configured in [false, true] {
        let mut s = session(configured);
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let revision = s.revision();
        let target = s.project().unwrap().board_target();
        s.apply(SET_BOARD_OUTLINE, &json!({"board":target.board,"outline":rectangle(),"expected_revision":revision})).unwrap();
        assert_eq!(s.revision(), revision + 1);
        let after = encode_documents(s.snapshot().unwrap()).unwrap();
        for (path, bytes) in &before {
            if path != "board.json" {
                assert_eq!(after[path], *bytes);
            }
        }
        assert_eq!(s.inspect()["project"]["board"]["id"], target.board.to_string());
        assert_eq!(s.inspect()["project"]["board"]["outline"], rectangle());
        assert_eq!(serde_json::to_value(s.project().unwrap().board_view(target).unwrap().outline()).unwrap(), rectangle());
        assert_eq!(serde_json::from_slice::<Value>(&after["board.json"]).unwrap()["format"], "pcbcraft-board/8");
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
        assert_eq!(encode_documents(decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project).unwrap(), after);
        assert!(crate::persistence::encode(s.snapshot().unwrap()).unwrap_err().contains("outlines"));
        let revision = s.revision();
        s.apply(SET_BOARD_OUTLINE, &json!({"outline":rectangle(),"expected_revision":revision})).unwrap();
        assert_eq!(s.revision(), revision);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
        s.apply(SET_BOARD_OUTLINE, &json!({"outline":null,"expected_revision":s.revision()})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        let revision = s.revision();
        s.apply(SET_BOARD_OUTLINE, &json!({"outline":null,"expected_revision":revision})).unwrap();
        assert_eq!(s.revision(), revision);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
    }
}

/// Rejected revisions, board identities and geometry never change bytes or consume redo history.
#[test]
fn invalid_or_stale_outline_is_atomic() {
    let mut s = session(true);
    let old_revision = s.revision();
    s.apply(SET_BOARD_OUTLINE, &json!({"outline":rectangle(),"expected_revision":old_revision})).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    let before = s.inspect();
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let mut candidates = vec![
        json!({"outline":rectangle(),"expected_revision":old_revision}),
        json!({"board":"00000000-0000-0000-0000-000000000001","outline":rectangle(),"expected_revision":revision}),
        json!({"expected_revision":revision}),
    ];
    for outline in [
        json!({"shape":"rectangle","min":{"x_nm":0,"y_nm":0},"max":{"x_nm":0,"y_nm":1}}),
        json!({"shape":"rectangle","min":{"x_nm":1,"y_nm":0},"max":{"x_nm":0,"y_nm":1}}),
        json!({"shape":"rectangle","min":{"x_nm":0,"y_nm":0},"max":{"x_nm":1,"y_nm":-1}}),
        json!({"shape":"rectangle","min":{"x_nm":i64::MIN,"y_nm":0},"max":{"x_nm":i64::MAX,"y_nm":1}}),
        json!({"shape":"rectangle","min":{"x_nm":0.5,"y_nm":0},"max":{"x_nm":1,"y_nm":1}}),
        json!({"shape":"polygon","min":{"x_nm":0,"y_nm":0},"max":{"x_nm":1,"y_nm":1}}),
        json!({"shape":"rectangle","min":{"x_nm":0,"y_nm":0},"max":{"x_nm":1,"y_nm":1},"cutouts":[]}),
        json!([]),
    ] {
        candidates.push(json!({"outline":outline,"expected_revision":revision}));
    }
    for params in candidates {
        assert!(s.apply(SET_BOARD_OUTLINE, &params).is_err(), "{params}");
        assert_eq!(s.inspect(), before);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    }
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(s.inspect()["project"]["board"]["outline"], rectangle());
}

/// Version gates refuse unknown or null boundaries instead of losing them on a later save.
#[test]
fn outline_storage_rejects_incompatible_formats_and_geometry() {
    let mut s = session(false);
    s.apply(SET_BOARD_OUTLINE, &json!({"outline":rectangle(),"expected_revision":s.revision()})).unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let board: Value = serde_json::from_slice(&original["board.json"]).unwrap();
    let mut candidates = Vec::new();
    for version in 1..=7 {
        let mut candidate = board.clone();
        candidate["format"] = json!(format!("pcbcraft-board/{version}"));
        candidates.push(candidate);
    }
    let mut missing = board.clone();
    missing.as_object_mut().unwrap().remove("outline");
    candidates.push(missing);
    let mut null = board.clone();
    null["outline"] = Value::Null;
    candidates.push(null);
    let mut reversed = board;
    reversed["outline"]["max"]["x_nm"] = json!(-30_000_000);
    candidates.push(reversed);
    for candidate in candidates {
        let mut documents = original.clone();
        documents.insert("board.json".into(), serde_json::to_vec(&candidate).unwrap());
        assert!(decode_documents(&documents).unwrap_err().contains("board.json"));
    }
}

/// Main's version-7 assignments remain byte-stable on open; adding an outline advances only
/// board data to version 8 and retains every assigned package, mapping and placement offline.
#[test]
fn outlines_preserve_version_seven_assignments_and_placements() {
    use crate::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT};
    for placed in [false, true] {
        let mut s = session(false);
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
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let board: Value = serde_json::from_slice(&before["board.json"]).unwrap();
        assert_eq!(board["format"], "pcbcraft-board/7");
        s.open_snapshot(decode_documents(&before).unwrap());
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply(SET_BOARD_OUTLINE, &json!({"outline":rectangle(),"expected_revision":s.revision()})).unwrap();
        let after = encode_documents(s.snapshot().unwrap()).unwrap();
        let outlined: Value = serde_json::from_slice(&after["board.json"]).unwrap();
        assert_eq!(outlined["format"], "pcbcraft-board/8");
        assert_eq!(outlined["assignments"], board["assignments"]);
        assert_eq!(outlined["footprints"], board["footprints"]);
        for (path, bytes) in &before {
            if path != "board.json" {
                assert_eq!(&after[path], bytes);
            }
        }
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
        assert_eq!(encode_documents(decode_packed(&encode_packed(s.snapshot().unwrap()).unwrap()).unwrap().project).unwrap(), after);
        let mut writer = crate::persistence::DocumentsEncoder::new(s.snapshot().unwrap());
        let mut streamed = None;
        for _ in 0..100 {
            if let Some(documents) = writer.step(1).unwrap() {
                streamed = Some(documents);
                break;
            }
        }
        assert_eq!(streamed, Some(after.clone()));
        s.apply(SET_BOARD_OUTLINE, &json!({"outline":null,"expected_revision":s.revision()})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
    }
}
