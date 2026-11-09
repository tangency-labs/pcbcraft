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
    session.apply("project.new", &json!({"create_board":true,})).unwrap();
    session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
    session
        .apply(
            PLACE_FOOTPRINT,
            &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":2}),
        )
        .unwrap();
    let original = session.inspect();
    let original_bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    let board = session.project().unwrap().board_target().unwrap();
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
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
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
            let footprint = restored.board_view(restored.board_target().unwrap()).unwrap().footprints().next().unwrap();
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

/// Batch deletion validates every identity before mutation, retains physical assignments and
/// schematic data, and restores exact placements/indices through undo and offline codecs.
#[test]
fn deleting_placements_is_atomic_and_preserves_components_and_packages() {
    use crate::{DELETE_FOOTPRINTS, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL};
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    for (i, definition) in ["resistor-0402", "header-1x02-254", "generic-through-hole-2"].into_iter().enumerate() {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":i as i64*20_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let symbol = s.project().unwrap().symbols().last().unwrap().id();
        let mut params = json!({"symbol":symbol,"definition":definition,"x_nm":i as i64*20_000_000,"y_nm":0,"expected_revision":s.revision()});
        if i == 2 {
            params["pin_1_pad"] = json!(2);
        } else {
            params["mapping"] = json!({"1":"2","2":"1"});
        }
        s.apply(PLACE_FOOTPRINT, &params).unwrap();
    }
    let original = s.inspect();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let footprints = original["project"]["board"]["footprints"].as_array().unwrap();
    let ids: Vec<_> = footprints.iter().map(|f| f["id"].clone()).collect();
    let board = s.project().unwrap().board_target().unwrap().board;
    let revision = s.revision();
    for invalid in [json!([]), json!([ids[0], ids[0]]), json!([ids[0], "footprint-999999"]), json!(["footprint-01"]), json!([null])] {
        assert!(s.apply(DELETE_FOOTPRINTS, &json!({"footprints":invalid,"expected_revision":revision})).is_err());
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        assert_eq!(s.revision(), revision);
    }
    for params in [
        json!({"board":"00000000-0000-4000-8000-000000000099","footprints":[ids[0]],"expected_revision":revision}),
        json!({"footprints":[ids[0]],"expected_revision":revision-1}),
    ] {
        assert!(s.apply(DELETE_FOOTPRINTS, &params).is_err());
        assert_eq!(s.inspect(), original);
    }
    s.apply(DELETE_FOOTPRINTS, &json!({"board":board,"footprints":[ids[0],ids[2]],"expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let after = s.inspect();
    assert_eq!(after["project"]["schematic"], original["project"]["schematic"]);
    assert_eq!(after["project"]["components"], original["project"]["components"]);
    assert_eq!(after["project"]["board"]["footprints"], json!([footprints[1]]));
    assert_eq!(s.project().unwrap().board.by_component.len(), 1);
    let removed = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&removed).unwrap()).unwrap(), removed);
    let assignments = s.project().unwrap().footprint_assignments();
    let retained = assignments.iter().find(|a| a["component"] == footprints[0]["component"]).unwrap();
    assert_eq!(retained["placed"], false);
    assert_eq!(retained["mapping"], footprints[0]["mapping"]);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), removed);
    s.open_snapshot(decode_documents(&removed).unwrap());
    s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"board":board,"component":retained["component"],"x_nm":0,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    assert_eq!(s.project().unwrap().board.footprints().len(), 2);
}

/// Every mode round-trips through its MCP name, and only the distribute modes need three.
#[test]
fn align_modes_round_trip_their_names() {
    for mode in super::AlignMode::ALL {
        assert_eq!(super::AlignMode::parse(mode.name()), Some(mode));
        assert_eq!(mode.minimum(), if mode.name().starts_with("distribute") { 3 } else { 2 });
    }
    assert_eq!(super::ALIGN_MODES.map(super::AlignMode::parse).map(Option::unwrap), super::AlignMode::ALL);
    assert_eq!(super::AlignMode::parse("distribute"), None);
}

/// Edge, center and distribute modes move boxes exactly; distribution keeps the outer items and
/// leaves equal gaps.
#[test]
fn alignment_moves_boxes_to_shared_edges_and_equal_gaps() {
    let boxes = [[0, 0, 10, 10], [30, 5, 34, 25], [12, -4, 20, 2]];
    assert_eq!(super::alignment(super::AlignMode::Left, &boxes), vec![(0, 0), (-30, 0), (-12, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Right, &boxes), vec![(24, 0), (0, 0), (14, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Top, &boxes), vec![(0, -4), (0, -9), (0, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Bottom, &boxes), vec![(0, 15), (0, 0), (0, 23)]);
    assert_eq!(super::alignment(super::AlignMode::CenterX, &boxes), vec![(12, 0), (-15, 0), (1, 0)]);
    let spread = super::alignment(super::AlignMode::DistributeX, &boxes);
    let placed: Vec<_> = boxes.iter().zip(&spread).map(|(b, (dx, _))| [b[0] + dx, b[2] + dx]).collect();
    assert_eq!(placed, vec![[0, 10], [30, 34], [16, 24]], "gaps of 6: 34 of span minus 22 of sizes, in two gaps");
}

/// The command aligns real footprints in one undoable edit, is a no-op when already aligned,
/// and refuses too few, repeated or unknown footprints and unknown modes.
#[test]
fn align_footprints_is_one_checked_edit() {
    use crate::board::track::tests::{routed_fixture, view};
    use crate::persistence::encode_documents;
    let mut s = routed_fixture();
    let ids: Vec<String> = view(&s).footprints().map(|f| f.id.clone()).collect();
    assert_eq!(ids.len(), 3);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let nudge = |s: &mut Session, dx: i64, dy: i64| {
        let (id, at, rotation) = view(s).footprints().next().map(|f| (f.id.clone(), f.position, f.rotation)).unwrap();
        s.apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":at.x_nm + dx,"y_nm":at.y_nm + dy,"rotation":rotation,"expected_revision":s.revision()}),
        )
        .unwrap();
    };
    nudge(&mut s, 0, 3_000_000);
    let revision = s.revision();
    s.apply(super::ALIGN_FOOTPRINTS, &json!({"footprints":ids,"mode":"top","expected_revision":revision})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let tops: Vec<_> = view(&s).footprints().map(|f| super::bounds(&f)[1]).collect();
    assert!(tops.windows(2).all(|pair| pair[0] == pair[1]), "{tops:?}");
    s.apply(super::ALIGN_FOOTPRINTS, &json!({"footprints":ids,"mode":"top","expected_revision":s.revision()})).unwrap();
    assert_eq!(s.revision(), revision + 1, "already aligned is not an edit");
    // The fixture is evenly spaced, so nudge one footprint before distributing.
    nudge(&mut s, 3_000_000, 0);
    let nudged_revision = s.revision();
    s.apply(super::ALIGN_FOOTPRINTS, &json!({"footprints":ids,"mode":"distribute_x","expected_revision":s.revision()})).unwrap();
    assert_eq!(s.revision(), nudged_revision + 1, "distributing is one edit");
    let mut spans: Vec<_> = view(&s).footprints().map(|f| super::bounds(&f)).map(|b| (b[0], b[2])).collect();
    spans.sort();
    assert_eq!(spans[1].0 - spans[0].1, spans[2].0 - spans[1].1, "equal gaps: {spans:?}");
    let revision = s.revision();
    for params in [
        json!({"footprints":[ids[0]],"mode":"left"}),
        json!({"footprints":[ids[0], ids[1]],"mode":"distribute_y"}),
        json!({"footprints":[ids[0], ids[0]],"mode":"left"}),
        json!({"footprints":[ids[0], "footprint-999"],"mode":"left"}),
        json!({"footprints":[ids[0], ids[1]],"mode":"sideways"}),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(revision);
        assert!(s.apply(super::ALIGN_FOOTPRINTS, &params).is_err(), "{params}");
    }
    assert_eq!(s.revision(), revision);
    for _ in 0..4 {
        s.apply("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before, "each nudge and alignment is one undo step");
}
