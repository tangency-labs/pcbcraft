use super::*;
use crate::{
    PLACE_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM,
    error_codes::ErrorCode,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Bad reference edits preserve data and history, and repeating an accepted offset is a no-op.
#[test]
fn reference_moves_validate_targets_and_preserve_physical_placement() {
    use crate::board::track::tests::{routed_fixture, view};
    let mut session = routed_fixture();
    let footprint = view(&session).footprints().next().unwrap();
    let board = view(&session).target.board;
    let id = footprint.id.clone();
    let pads = footprint.pads;
    let origin = footprint.position;
    let revision = session.revision();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let params = json!({"board":board,"footprint":id,"offset":{"x_nm":1_000_000,"y_nm":-1_000_000},"expected_revision":revision});
    for (key, value) in [
        ("board", json!("00000000-0000-0000-0000-000000000001")),
        ("footprint", json!("footprint-99999")),
        ("expected_revision", json!(revision - 1)),
        ("offset", json!({"x_nm":100_000_001,"y_nm":0})),
        ("offset", json!({"x_nm":0,"y_nm":-100_000_001})),
        ("offset", json!({"x_nm":1.5,"y_nm":0})),
        ("offset", json!({"x_nm":1,"y_nm":0,"pixels":1})),
    ] {
        let mut invalid = params.clone();
        invalid[key] = value;
        assert!(session.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &invalid).is_err());
        assert_eq!(session.revision(), revision);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
    }
    session.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &params).unwrap();
    let moved = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert_eq!(moved.pads, pads);
    assert_eq!(moved.position, origin);
    let mut repeated = params;
    let revision = session.revision();
    repeated["expected_revision"] = json!(revision);
    session.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &repeated).unwrap();
    assert_eq!(session.revision(), revision);
}

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
    assert!(std::str::from_utf8(&bytes[&crate::persistence::test_board_path(&bytes)]).unwrap().contains("pcbcraft-board/4"));
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    assert_eq!(
        bytes.iter().filter(|(path, _)| !path.ends_with("board.json")).collect::<Vec<_>>(),
        original_bytes.iter().filter(|(path, _)| !path.ends_with("board.json")).collect::<Vec<_>>()
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
            let mut board: Value = serde_json::from_slice(&bad[&crate::persistence::test_board_path(&bad)]).unwrap();
            board["footprints"][0]["rotation"] = value;
            bad.insert(crate::persistence::test_board_path(&bad), serde_json::to_vec(&board).unwrap());
            assert!(decode_documents(&bad).is_err());
        }
        let mut old = bytes.clone();
        let mut board: Value = serde_json::from_slice(&old[&crate::persistence::test_board_path(&old)]).unwrap();
        board["format"] = json!("pcbcraft-board/2");
        old.insert(crate::persistence::test_board_path(&old), serde_json::to_vec(&board).unwrap());
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
    assert_eq!(super::alignment(super::AlignMode::Left, &boxes, None).unwrap(), vec![(0, 0), (-30, 0), (-12, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Right, &boxes, None).unwrap(), vec![(24, 0), (0, 0), (14, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Top, &boxes, None).unwrap(), vec![(0, -4), (0, -9), (0, 0)]);
    assert_eq!(super::alignment(super::AlignMode::Bottom, &boxes, None).unwrap(), vec![(0, 15), (0, 0), (0, 23)]);
    assert_eq!(super::alignment(super::AlignMode::CenterX, &boxes, None).unwrap(), vec![(12, 0), (-15, 0), (1, 0)]);
    let spread = super::alignment(super::AlignMode::DistributeX, &boxes, None).unwrap();
    let placed: Vec<_> = boxes.iter().zip(&spread).map(|(b, (dx, _))| [b[0] + dx, b[2] + dx]).collect();
    assert_eq!(placed, vec![[0, 10], [30, 34], [16, 24]], "gaps of 6: 34 of span minus 22 of sizes, in two gaps");
}

/// Distribution anchors physical edges even when overlapping boxes have a different center order.
#[test]
fn distribution_keeps_physical_outer_edges_fixed() {
    for spans in [[[0, 20], [1, 3], [30, 34]], [[0, 4], [10, 30], [27, 29]]] {
        let boxes = spans.map(|[a, b]| [a, a, b, b]);
        for (mode, axis) in [(super::AlignMode::DistributeX, 0), (super::AlignMode::DistributeY, 1)] {
            let deltas = super::alignment(mode, &boxes, None).unwrap();
            assert_eq!(deltas[0], (0, 0), "leading physical anchor: {mode:?}, {spans:?}");
            let trailing = if spans[1][1] > spans[2][1] { 1 } else { 2 };
            assert_eq!(deltas[trailing], (0, 0), "trailing physical anchor: {mode:?}, {spans:?}");
            let mut placed: Vec<_> = boxes
                .iter()
                .zip(&deltas)
                .map(|(b, (dx, dy))| {
                    let delta = if axis == 0 { *dx } else { *dy };
                    (b[axis] + delta, b[axis + 2] + delta)
                })
                .collect();
            placed.sort();
            assert_eq!(placed[0].0, spans[0][0]);
            assert_eq!(placed.last().unwrap().1, spans[trailing][1]);
            assert_eq!(placed[1].0 - placed[0].1, placed[2].0 - placed[1].1);
        }
    }
}

/// Infeasible overlap is refused rather than moving an outer anchor or expanding the span.
#[test]
fn distribution_refuses_overlap_that_cannot_preserve_its_span() {
    let boxes = [[0, 0, 1, 1], [10, 10, 90, 90], [20, 20, 80, 80], [99, 99, 100, 100]];
    for mode in [super::AlignMode::DistributeX, super::AlignMode::DistributeY] {
        let error = super::alignment(mode, &boxes, None).unwrap_err();
        assert!(error.to_string().contains("keeping the outer footprints"));
    }
    use crate::board::track::tests::{routed_fixture, view};
    let mut s = routed_fixture();
    let footprints: Vec<_> = view(&s).footprints().collect();
    let ids: Vec<_> = footprints.iter().map(|f| f.id.clone()).collect();
    let origin = footprints[0].position;
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":ids[1],"x_nm":origin.x_nm,"y_nm":origin.y_nm,"rotation":footprints[1].rotation,"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let result = s.apply(super::ALIGN_FOOTPRINTS, &json!({"footprints":ids,"mode":"distribute_x","expected_revision":revision}));
    assert!(result.is_err(), "both footprints defining the left edge must remain fixed");
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Center alignment rounds consistently across zero and does not move aligned mixed-width boxes
/// on a second application, on either axis and at positive or negative positions.
#[test]
fn center_alignment_is_idempotent_across_zero() {
    for offset in [-20, 0, 20] {
        let boxes = [[-3 + offset, -3 + offset, offset, offset], [offset, offset, 10 + offset, 10 + offset]];
        for (mode, axis) in [(super::AlignMode::CenterX, 0), (super::AlignMode::CenterY, 1)] {
            let moved: Vec<_> = boxes
                .iter()
                .zip(super::alignment(mode, &boxes, None).unwrap())
                .map(|(b, (dx, dy))| [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy])
                .collect();
            let centers: Vec<_> = moved.iter().map(|b| (b[axis] + b[axis + 2]).div_euclid(2)).collect();
            assert_eq!(centers, vec![3 + offset; 2], "{mode:?} at offset {offset}: {moved:?}");
            assert_eq!(super::alignment(mode, &moved, None).unwrap(), vec![(0, 0); 2], "repeating {mode:?} must not create another edit");
        }
    }
}

/// An offset odd-diameter opening extends the alignment edge, not just the displayed outline.
#[test]
fn alignment_uses_the_outer_edge_of_a_non_plated_hole() {
    use crate::board::{
        definition::CircularNpth,
        track::tests::{routed_fixture, view},
    };
    let session = routed_fixture();
    let board = view(&session);
    let mut footprint = board.footprints().next().unwrap();
    let plain = super::bounds(&footprint);
    let center = Point { x_nm: plain[2] + 1_000_000, y_nm: footprint.position.y_nm };
    footprint.non_plated_holes.push(CircularNpth { center, diameter_nm: 600_001 });
    let mechanical = super::bounds(&footprint);
    assert_eq!(mechanical[2], center.x_nm + 300_001);
    assert_eq!(super::alignment(AlignMode::Right, &[plain, mechanical]).unwrap(), vec![(1_300_001, 0), (0, 0)]);
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

/// Author connector placements through the production commands, with one rotated back-side part.
fn pitch_connectors(positions: &[Point]) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    for (i, point) in positions.iter().enumerate() {
        session.apply(PLACE_RESISTOR, &json!({"x_nm":i as i64*20_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        let symbol = session.project().unwrap().symbols().last().unwrap().id();
        session
            .apply(
                PLACE_FOOTPRINT,
                &json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},
                    "x_nm":point.x_nm,"y_nm":point.y_nm,"expected_revision":session.revision()}),
            )
            .unwrap();
    }
    let first = crate::board::track::tests::view(&session).footprints().next().unwrap();
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":first.id,"x_nm":first.position.x_nm,"y_nm":first.position.y_nm,
            "rotation":90,"side":"back","expected_revision":session.revision()}),
        )
        .unwrap();
    session
}

/// Five connectors land on exact 5.08 mm origin pitch on either axis. The leading origin stays
/// fixed, request order breaks coordinate ties, and perpendicular positions and poses survive.
#[test]
fn set_pitch_distributes_five_connectors_and_round_trips_one_edit() {
    use crate::board::track::tests::view;
    const PITCH: i64 = 5_080_000;
    for (mode, horizontal) in [("distribute_x", true), ("distribute_y", false)] {
        let point = |axis, cross| {
            if horizontal { Point { x_nm: axis, y_nm: cross } } else { Point { x_nm: cross, y_nm: axis } }
        };
        let origins = [11_000_000, -4_000_000, -4_000_000, 18_000_000, 6_000_000];
        let cross = |i: usize| i as i64 * 8_000_000;
        let mut s = pitch_connectors(&std::array::from_fn::<_, 5, _>(|i| point(origins[i], cross(i))));
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let placements = |session: &Session| view(session).footprints().map(|f| (f.id, f.position, f.rotation, f.bottom)).collect::<Vec<_>>();
        let original = placements(&s);
        let ids = [2, 4, 0, 1, 3].map(|i| &original[i].0);
        let revision = s.revision();
        let mut params = json!({"board":view(&s).target.board,"footprints":ids,"mode":mode,"pitch_nm":PITCH,"expected_revision":revision});
        s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
        let result = placements(&s);
        for (rank, i) in [2, 1, 4, 0, 3].into_iter().enumerate() {
            let (id, _, rotation, bottom) = &original[i];
            assert_eq!(result[i], (id.clone(), point(-4_000_000 + rank as i64 * PITCH, cross(i)), *rotation, *bottom), "{mode}, footprint {id}");
        }
        assert_eq!(s.revision(), revision + 1);
        let after = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
        params["expected_revision"] = json!(s.revision());
        s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
        assert_eq!(s.revision(), revision + 1, "repeating pitch creates no edit");
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
    }
}

/// Check the stable refusal code and message without changes to the snapshot, revision or history.
fn assert_alignment_refused(s: &mut Session, params: &Value, code: ErrorCode, reason: &str) {
    let before = s.snapshot().unwrap();
    let history = (s.revision(), s.undo.len(), s.redo.len());
    let error = s.execute(ALIGN_FOOTPRINTS, params).unwrap_err();
    assert_eq!(error.code(), code, "{error} for {params}");
    assert!(error.to_string().contains(reason), "expected {reason:?}, got {error:?} for {params}");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!((s.revision(), s.undo.len(), s.redo.len()), history);
}

/// Invalid values, wrong modes and invalid placement targets are refused; one nanometer and the
/// full placement span are accepted. A valid pitch with an out-of-range destination is refused.
#[test]
fn set_pitch_rejects_invalid_inputs_and_out_of_range_targets_atomically() {
    use crate::board::track::tests::view;
    let mut s = pitch_connectors(&[Point { x_nm: -MAX_CENTER_NM, y_nm: 0 }, Point { x_nm: 0, y_nm: 20_000_000 }]);
    let ids: Vec<_> = view(&s).footprints().map(|f| f.id).collect();
    let rev = s.revision();
    let mut params = json!({"footprints":ids,"mode":"distribute_x","pitch_nm":1,"expected_revision":rev});
    for (key, value, reason) in [
        ("pitch_nm", json!(0), "requires pitch_nm"),
        ("pitch_nm", json!(-1), "requires pitch_nm"),
        ("pitch_nm", json!(1.5), "requires pitch_nm"),
        ("pitch_nm", json!(null), "requires pitch_nm"),
        ("pitch_nm", json!(MAX_FOOTPRINT_PITCH_NM + 1), "requires pitch_nm"),
        ("mode", json!("top"), "pitch_nm requires"),
        ("footprints", json!([ids[0]]), "at least 2"),
        ("footprints", json!([ids[0], ids[0]]), "unique placed footprint IDs"),
    ] {
        let mut invalid = params.clone();
        invalid[key] = value;
        assert_alignment_refused(&mut s, &invalid, ErrorCode::InvalidParams, reason);
    }
    let mut unknown = params.clone();
    unknown["footprints"] = json!([ids[0], "footprint-999"]);
    assert_alignment_refused(&mut s, &unknown, ErrorCode::NotFound, "unknown footprint ID");
    unknown["expected_revision"] = json!(rev - 1);
    assert_alignment_refused(&mut s, &unknown, ErrorCode::StaleRevision, "project changed");
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(view(&s).footprint(&ids[1]).unwrap().position.x_nm, -MAX_CENTER_NM + 1);
    params["expected_revision"] = json!(s.revision());
    params["pitch_nm"] = json!(MAX_FOOTPRINT_PITCH_NM);
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(view(&s).footprint(&ids[1]).unwrap().position.x_nm, MAX_CENTER_NM);

    params["expected_revision"] = json!(s.revision());
    params["mode"] = json!("distribute_y");
    params["pitch_nm"] = json!(MAX_CENTER_NM + 1);
    assert_alignment_refused(&mut s, &params, ErrorCode::InvalidParams, "outside the coordinate range");
    params["pitch_nm"] = json!(MAX_CENTER_NM);
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(view(&s).footprint(&ids[1]).unwrap().position.y_nm, MAX_CENTER_NM);
}

/// Pitch uses the same fixed-copper protection and carries an unlocked track in the same edit.
#[test]
fn set_pitch_refuses_locked_attached_copper_then_carries_it_unlocked() {
    use crate::board::track::tests::{copper_layers, route, routed_fixture, view};
    let mut s = routed_fixture();
    let footprints: Vec<_> = view(&s).footprints().collect();
    let (mover, anchor) = (footprints[0].id.clone(), footprints[2].id.clone());
    let pad = footprints[0].pads[0].center;
    let free = Point { x_nm: pad.x_nm, y_nm: pad.y_nm + 3_000_000 };
    let layer = copper_layers(&s)[0];
    route(&mut s, layer, &[pad, free]).unwrap();
    let track = view(&s).tracks()[0].id;
    let board = s.project().unwrap().board_target().unwrap().board;
    let mut lock = json!({"board":board,"tracks":[track],"locked":true,"expected_revision":s.revision()});
    s.apply(crate::SET_COPPER_LOCKED, &lock).unwrap();
    let mut params = json!({"footprints":[mover, anchor],"mode":"distribute_x","pitch_nm":22_000_000,"expected_revision":s.revision()});
    assert_alignment_refused(&mut s, &params, ErrorCode::InvalidParams, &format!("Track {track} is locked"));
    lock["expected_revision"] = json!(s.revision());
    lock["locked"] = json!(false);
    s.apply(crate::SET_COPPER_LOCKED, &lock).unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    params["expected_revision"] = json!(revision);
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    let carried = &view(&s).tracks()[0];
    assert_eq!((carried.id, carried.end), (track, free));
    assert_eq!(carried.start, Point { x_nm: pad.x_nm + 6_000_000, y_nm: pad.y_nm });
    assert_eq!(s.revision(), revision + 1);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Landing a moved conductive pad on a pad of another accepted net refuses the entire edit.
#[test]
fn set_pitch_refuses_new_shorts_atomically() {
    use crate::board::track::tests::{routed_fixture, view};
    let mut s = routed_fixture();
    let footprints: Vec<_> = view(&s).footprints().collect();
    let (obstacle, mover, anchor) = (&footprints[0], &footprints[1], &footprints[2]);
    let obstacle_pad = obstacle.pads.iter().find(|p| p.net.is_some()).unwrap();
    let mover_pad = mover.pads.iter().find(|p| p.net.is_some() && p.net != obstacle_pad.net).unwrap();
    assert_eq!(obstacle_pad.center.y_nm, mover_pad.center.y_nm);
    let pitch = mover.position.x_nm + obstacle_pad.center.x_nm - mover_pad.center.x_nm - anchor.position.x_nm;
    let params = json!({"footprints":[mover.id, anchor.id],"mode":"distribute_x","pitch_nm":pitch,"expected_revision":s.revision()});
    assert_alignment_refused(&mut s, &params, ErrorCode::InvalidParams, "would connect nets");
}

/// Author a one-terminal part and a footprint with one plated mounting drill, then place it
/// through the library and placement commands. Do not stand in a via or a non-plated hole.
fn place_plated_mounting_hole(s: &mut Session) -> String {
    use crate::authoring::{Action, Item, Kind, Primitive};
    use crate::board::definition::{CopperBinding, PadShape};
    let mut symbol = crate::authoring::Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    symbol.execute(symbol.revision(), Action::Delete { objects: vec![3] }).unwrap();
    let mut footprint = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    footprint.execute(footprint.revision(), Action::Delete { objects: vec![2, 3] }).unwrap();
    let Primitive::Pad { mut pad } = footprint.snapshot().objects[&1].clone() else { panic!("pad") };
    pad.center = Point { x_nm: 700_000, y_nm: 0 };
    pad.width_nm = 5_000_000;
    pad.height_nm = 5_000_000;
    pad.shape = PadShape::Circle;
    pad.corner_radius_nm = 0;
    pad.copper = CopperBinding::AllCopper;
    pad.drill_nm = Some(3_200_000);
    footprint.execute(footprint.revision(), Action::Replace { object: 1, primitive: Primitive::Pad { pad } }).unwrap();
    let definition = footprint.snapshot().footprint().unwrap();
    let symbol_def = symbol.snapshot().symbol().unwrap();
    let manifest = json!({"format":crate::library::LIBRARY_FORMAT,"id":definition.library,"name":"Test fixture","license":definition.license});
    s.execute(
        "library.load_source",
        &json!({"source":"mounting-hole-fixture","documents":{
            "library.json":manifest.to_string(),
            "symbols/hole.json":symbol.snapshot().export_native().unwrap().to_string(),
            "footprints/hole.json":footprint.snapshot().export_native().unwrap().to_string(),
        }}),
    )
    .unwrap();
    let entry = s
        .libraries()
        .search(&symbol_def.id, Some(crate::library::sources::Kind::Symbol))
        .into_iter()
        .find(|(source, _)| *source == "mounting-hole-fixture")
        .unwrap()
        .1
        .inspect();
    s.apply(
        crate::PLACE_SYMBOL,
        &json!({"library":symbol_def.library,"definition":symbol_def.id,"definition_revision":symbol_def.revision(),
        "definition_digest":entry["digest"],"x_nm":100_000_000,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    let component = s.project().unwrap().components().last().unwrap().id();
    s.apply(
        crate::ASSIGN_FOOTPRINT,
        &json!({"component":component,"library":definition.library,"definition":definition.id,
        "definition_revision":definition.revision,"mapping":{"1":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":17_000_000,"y_nm":-10_000_000,"expected_revision":s.revision()}))
        .unwrap();
    crate::board::track::tests::view(s).footprint_ids().last().unwrap()
}

/// Three placements align to the physical edge of a locked, rotated bottom-side mounting
/// hole even when it is in the selection. Its placement and plated drill stay fixed.
#[test]
fn align_to_locked_plated_mounting_hole_keeps_the_reference_fixed() {
    use crate::board::track::tests::view;
    let mut fixture =
        pitch_connectors(&[Point { x_nm: -10_000_000, y_nm: 0 }, Point { x_nm: -12_000_000, y_nm: 12_000_000 }, Point { x_nm: 0, y_nm: 24_000_000 }]);
    let reference = place_plated_mounting_hole(&mut fixture);
    let position = view(&fixture).footprint(&reference).unwrap().position;
    fixture
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":reference,"x_nm":position.x_nm,"y_nm":position.y_nm,
                "rotation":90,"side":"back","expected_revision":fixture.revision()}),
        )
        .unwrap();
    let original = fixture.snapshot().unwrap();
    let ids: Vec<_> = view(&fixture).footprint_ids().collect();
    assert_eq!(ids.len(), 4);
    let mut locked = original.as_ref().clone();
    // The persisted lock has no public authoring command yet; only fixture metadata is injected.
    Arc::make_mut(&mut locked.board).footprints.last_mut().unwrap().locked = true;
    let initial = encode_documents(Arc::new(locked)).unwrap();
    let mut s = Session::default();
    s.open_snapshot(decode_documents(&initial).unwrap());
    let hole = view(&s).footprint(&reference).unwrap();
    assert!(hole.locked);
    assert_eq!(hole.pads.len(), 1);
    assert!(hole.pads[0].through_hole);
    assert_eq!(hole.pads[0].drill_nm, Some(3_200_000));
    assert!(hole.bottom);
    assert_eq!(hole.rotation, 90);
    assert_eq!(hole.pads[0].center, Point { x_nm: 17_000_000, y_nm: -9_300_000 });
    // At 90 degrees the local 0.7 mm offset runs along Y, not X. The 5 mm land
    // therefore starts 2.5 mm left of the placement origin, irrespective of drill size.
    let left = 14_500_000;
    assert_eq!(bounds(&hole)[0], left);
    let (hole_at, hole_pads) = (hole.position, hole.pads);
    let revision = s.revision();
    let mut params =
        json!({"board":view(&s).target.board,"footprints":ids,"mode":"left","reference_footprint":reference,"expected_revision":revision});
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(s.revision(), revision + 1);
    for id in &ids[..3] {
        assert_eq!(contour_bounds(&view(&s).footprint(id).unwrap())[0], left);
    }
    let fixed = view(&s).footprint(&reference).unwrap();
    assert_eq!(fixed.position, hole_at);
    assert_eq!(fixed.pads, hole_pads);
    assert_eq!(fixed.rotation, 90);
    assert!(fixed.bottom && fixed.locked);
    let aligned = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&aligned).unwrap()).unwrap(), aligned);
    params["expected_revision"] = json!(s.revision());
    params["footprints"] = json!(ids[..3]);
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(s.revision(), revision + 1);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), initial);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), aligned);
}

/// The 45° plated land starts one radius left of its center, not at the corner of its
/// rotated rectangle. The same footprint keeps its historical selection-only envelope.
#[test]
fn reference_alignment_uses_rotated_round_land_extent() {
    use crate::board::track::tests::view;
    let mut s = pitch_connectors(&[Point { x_nm: 32_000_000, y_nm: 12_000_000 }]);
    let mover = view(&s).footprint_ids().next().unwrap();
    let reference = place_plated_mounting_hole(&mut s);
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":reference,"x_nm":17_000_000,"y_nm":-10_000_000,"rotation":45,"side":"back","expected_revision":s.revision()}),
    )
    .unwrap();
    let hole = view(&s).footprint(&reference).unwrap();
    let left = hole.pads[0].center.x_nm - 2_500_000;
    let legacy_left = hole.pads[0].center.x_nm - 3_535_534;
    assert_eq!(bounds(&hole)[0], legacy_left);
    assert_eq!(contour_bounds(&hole)[0], left);
    let hole_at = hole.position;
    let original = s.snapshot().unwrap();
    let p = view(&s).footprint(&mover).unwrap();
    let new_x = p.position.x_nm + left - contour_bounds(&p)[0];
    let y = p.position.y_nm;
    let mut params = json!({"footprints":[mover],"mode":"left","reference_footprint":reference,"expected_revision":s.revision()});
    s.apply(ALIGN_FOOTPRINTS, &params).unwrap();
    assert_eq!(view(&s).footprint(&mover).unwrap().position, Point { x_nm: new_x, y_nm: y });
    assert_eq!(view(&s).footprint(&reference).unwrap().position, hole_at);
    let revision = s.revision();
    params["expected_revision"] = json!(revision);
    assert_eq!(s.apply(ALIGN_FOOTPRINTS, &params).unwrap(), revision);

    let mut legacy = Session::default();
    legacy.open_snapshot(original);
    legacy.apply(ALIGN_FOOTPRINTS, &json!({"footprints":[mover, reference],"mode":"left","expected_revision":legacy.revision()})).unwrap();
    assert_eq!(bounds(&view(&legacy).footprint(&mover).unwrap())[0], legacy_left);
}

/// Bound geometry work on the actual synchronous edit path, not just its alignment planner.
/// Timings for live MCP are a separate qualification: these counters are deterministic.
#[test]
fn large_routed_board_reference_alignment_has_bounded_geometry_work() {
    use crate::board::{
        track::tests::{copper_layers, route, routed_fixture, view},
        update::TERMINAL_NET_INDEX_BUILDS,
        view::FOOTPRINT_GEOMETRY_RESOLUTIONS,
    };
    const FILLERS: i64 = 256;
    const SHIFT: i64 = 5_000_000;
    let mut s = routed_fixture();
    let ids: Vec<_> = view(&s).footprint_ids().collect();
    let (mover, reference) = (&ids[0], &ids[1]);
    let anchor = view(&s).footprint(reference).unwrap().position;
    s.apply(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":reference,"x_nm":anchor.x_nm,"y_nm":anchor.y_nm + SHIFT,"expected_revision":s.revision()}))
        .unwrap();
    for i in 0..FILLERS {
        let x = 100_000_000 + i % 16 * 10_000_000;
        let y = 100_000_000 + i / 16 * 10_000_000;
        s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":y,"expected_revision":s.revision()})).unwrap();
        let symbol = s.project().unwrap().symbols().last().unwrap().id();
        s.apply(
            PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},
                "x_nm":x,"y_nm":y,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let moving = view(&s).footprint(mover).unwrap();
    let pad = moving.pads.iter().find(|p| p.net.as_deref() == Some("ACCEPTED")).unwrap().center;
    let free = Point { x_nm: pad.x_nm, y_nm: pad.y_nm - 3_000_000 };
    let layer = copper_layers(&s)[0];
    route(&mut s, layer, &[pad, free]).unwrap();
    let stub = view(&s).tracks()[0].id;
    let background: Vec<_> = (0..=128).map(|i| Point { x_nm: 100_000_000 + i * 2_000_000, y_nm: 78_000_000 + (i % 2) * 500_000 }).collect();
    route(&mut s, layer, &background).unwrap();
    let board = view(&s);
    let n = board.footprint_ids().count();
    assert_eq!(n, FILLERS as usize + 3);
    assert_eq!(board.tracks().len(), 129);
    let revision = s.revision();
    let mut params = json!({"board":board.target.board,"footprints":[mover],"mode":"top",
        "reference_footprint":reference,"expected_revision":revision});
    let align = |s: &mut Session, params: &Value| {
        // Measure the edit and publication, without a full-board inspection response.
        FOOTPRINT_GEOMETRY_RESOLUTIONS.with(|count| count.set(0));
        TERMINAL_NET_INDEX_BUILDS.with(|count| count.set(0));
        let revision = s.apply(ALIGN_FOOTPRINTS, params).unwrap();
        (revision, FOOTPRINT_GEOMETRY_RESOLUTIONS.with(|count| count.get()), TERMINAL_NET_INDEX_BUILDS.with(|count| count.get()))
    };
    let (published, placements, indexes) = align(&mut s, &params);
    assert_eq!(published, revision + 1);
    // Two selected views plus at most three whole-board passes (old lands, new lands, shorts).
    // The short check also builds a membership index independently of its geometry pass.
    assert!(placements <= 3 * n + 2, "resolved {placements} footprints on a board with {n}");
    assert!(indexes <= 5, "built {indexes} accepted-net indexes for one move");
    let board = view(&s);
    let carried = board.tracks().iter().find(|track| track.id == stub).unwrap();
    assert_eq!((carried.start, carried.end), (Point { x_nm: pad.x_nm, y_nm: pad.y_nm + SHIFT }, free));
    assert_eq!(board.footprint(reference).unwrap().position, Point { x_nm: anchor.x_nm, y_nm: anchor.y_nm + SHIFT });
    params["expected_revision"] = json!(revision + 1);
    let (published, placements, indexes) = align(&mut s, &params);
    assert_eq!(published, revision + 1);
    assert_eq!((placements, indexes), (2, 1), "a repeated alignment must not scan whole-board geometry or recheck copper");
}

/// The six target modes use the reference, not the movers' combined bounds. Centers round
/// consistently for odd-size boxes and negative reference coordinates.
#[test]
fn fixed_reference_controls_all_edge_and_center_modes() {
    let boxes = [[0, 0, 5, 5]];
    let target = Some([-15, -9, -2, 6]);
    for (mode, shift) in [
        (AlignMode::Left, (-15, 0)),
        (AlignMode::Right, (-7, 0)),
        (AlignMode::Top, (0, -9)),
        (AlignMode::Bottom, (0, 1)),
        (AlignMode::CenterX, (-11, 0)),
        (AlignMode::CenterY, (0, -4)),
    ] {
        assert_eq!(alignment(mode, &boxes, target).unwrap(), vec![shift], "{mode:?}");
    }
}

/// Invalid references and locked movers refuse the whole edit, with typed errors and unchanged
/// history. The same locked placement becomes a legal anchor when it is the chosen reference.
#[test]
fn reference_alignment_refuses_bad_targets_and_locked_movers_atomically() {
    use crate::board::track::tests::view;
    let fixture = pitch_connectors(&[Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 10_000_000, y_nm: 10_000_000 }]);
    let ids: Vec<_> = view(&fixture).footprint_ids().collect();
    let mut locked = fixture.snapshot().unwrap().as_ref().clone();
    Arc::make_mut(&mut locked.board).footprints[0].locked = true;
    let mut s = Session::default();
    s.open_snapshot(Arc::new(locked));
    let base = json!({"footprints":[ids[1]],"mode":"center_x","reference_footprint":ids[0],"expected_revision":s.revision()});
    for (key, value, code, reason) in [
        ("reference_footprint", json!("footprint-999"), ErrorCode::NotFound, "unknown reference footprint"),
        ("reference_footprint", json!("footprint-01"), ErrorCode::InvalidParams, "must identify a placed footprint"),
        ("footprints", json!([ids[0]]), ErrorCode::InvalidParams, "other than the reference"),
        ("mode", json!("distribute_x"), ErrorCode::InvalidParams, "only for edge or center alignment"),
        ("pitch_nm", json!(5_080_000), ErrorCode::InvalidParams, "pitch_nm requires"),
    ] {
        let mut args = base.clone();
        args[key] = value;
        assert_alignment_refused(&mut s, &args, code, reason);
    }
    let mut args = base.clone();
    args["footprints"] = json!([ids[0]]);
    args["reference_footprint"] = json!(ids[1]);
    assert_alignment_refused(&mut s, &args, ErrorCode::InvalidParams, &format!("({})", ids[0]));
    let revision = s.revision();
    s.apply(ALIGN_FOOTPRINTS, &base).unwrap();
    assert_eq!(s.revision(), revision + 1);
}

/// Moving, aligning and rotating a footprint carry the track end on its pad, one undo step each,
/// while the track's far end stays. A flip that takes the SMD pad off the track's layer releases
/// the end instead, and a move that would put another net's pad on the track is refused.
#[test]
fn footprint_transforms_carry_attached_track_ends() {
    use crate::board::track::tests::{copper_layers, route, routed_fixture, view};
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let line = view(&s).ratsnest().unwrap().lines[0].clone();
    let (pad, owner) = (line.from.position, line.from.footprint.clone());
    let number = view(&s).footprints().find(|f| f.id == owner).unwrap().pads.into_iter().find(|p| p.center == pad).unwrap().number;
    let free = Point { x_nm: pad.x_nm, y_nm: pad.y_nm.max(line.to.position.y_nm) + 2_000_000 };
    route(&mut s, front, &[pad, free]).unwrap();
    let stub = view(&s).tracks()[0].id;
    let ends = |s: &Session| view(s).tracks().iter().find(|t| t.id == stub).map(|t| (t.start, t.end)).unwrap();
    let placed = |s: &Session| view(s).footprints().find(|f| f.id == owner).map(|f| (f.position, f.rotation)).unwrap();
    let pad_center =
        |s: &Session| view(s).footprints().find(|f| f.id == owner).unwrap().pads.into_iter().find(|p| p.number == number).unwrap().center;
    let transform = |s: &mut Session, position: Point, rotation: u16, side: Option<&str>| {
        let mut params = json!({"footprint":owner,"x_nm":position.x_nm,"y_nm":position.y_nm,"rotation":rotation,"expected_revision":s.revision()});
        if let Some(side) = side {
            params["side"] = json!(side);
        }
        s.apply(SET_FOOTPRINT_TRANSFORM, &params)
    };
    let shifted = |p: Point, dx: i64, dy: i64| Point { x_nm: p.x_nm + dx, y_nm: p.y_nm + dy };

    let (origin, rotation) = placed(&s);
    let revision = s.revision();
    transform(&mut s, shifted(origin, 1_000_000, 1_000_000), rotation, None).unwrap();
    assert_eq!(ends(&s), (shifted(pad, 1_000_000, 1_000_000), free), "the pad end follows the move");
    assert_eq!(s.revision(), revision + 1, "the move and its track are one edit");

    let other = view(&s).footprints().find(|f| f.id != owner).unwrap().id.clone();
    s.apply(super::ALIGN_FOOTPRINTS, &json!({"footprints":[owner, other],"mode":"top","expected_revision":s.revision()})).unwrap();
    assert_eq!(ends(&s), (shifted(pad, 1_000_000, 0), free), "aligning carries the end too");
    assert_eq!(s.revision(), revision + 2);

    // A quarter turn swings the part's other pad, on another net, onto the stub.
    let (origin, rotation) = placed(&s);
    let error = transform(&mut s, origin, (rotation + 90) % 360, None).unwrap_err().to_string();
    assert!(error.contains("would connect nets"), "{error}");
    transform(&mut s, origin, (rotation + 180) % 360, None).unwrap();
    assert_eq!(ends(&s), (pad_center(&s), free), "the end stays on the turned pad");

    let before = ends(&s);
    transform(&mut s, origin, (rotation + 180) % 360, Some("back")).unwrap();
    assert_eq!(ends(&s), before, "a flip takes the SMD pad off the front, so the end stays");
    s.apply("edit.undo", &json!({})).unwrap();

    // Another net's pad landing on the stub's free end would short the nets.
    let (victim, offset) = {
        let net = view(&s).footprints().find(|f| f.id == owner).unwrap().pads.into_iter().find(|p| p.number == number).unwrap().net;
        let f = view(&s).footprints().find(|f| f.id != owner && f.pads.iter().any(|p| p.net.is_some() && p.net != net)).unwrap();
        let p = f.pads.iter().find(|p| p.net.is_some() && p.net != net).unwrap();
        (f.id.clone(), (p.center.x_nm - f.position.x_nm, p.center.y_nm - f.position.y_nm, f.rotation))
    };
    let revision = s.revision();
    let error = s
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":victim,"x_nm":free.x_nm - offset.0,"y_nm":free.y_nm - offset.1,"rotation":offset.2,"expected_revision":revision}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("would connect nets"), "{error}");
    assert_eq!(s.revision(), revision, "the refused move changes nothing");
}

/// Flipping a plated pad mirrors an off-center track end on the same layer and leaves its far end fixed.
#[test]
fn flipping_through_hole_pad_mirrors_its_off_center_track_end() {
    use crate::board::track::tests::{copper_layers, route, view};

    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let symbol = s.project().unwrap().symbols().first().unwrap().id();
    let origin = Point { x_nm: 4_000_000, y_nm: 1_000_000 };
    s.apply(
        PLACE_FOOTPRINT,
        &json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},
            "x_nm":origin.x_nm,"y_nm":origin.y_nm,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().next().unwrap();
    let id = footprint.id;
    let pad = footprint.pads.iter().find(|pad| pad.number == "1").unwrap();
    let start = Point { x_nm: pad.center.x_nm + 180_000, y_nm: pad.center.y_nm + 100_000 };
    let free = Point { x_nm: start.x_nm, y_nm: start.y_nm + 2_000_000 };
    let front = copper_layers(&s)[0];
    route(&mut s, front, &[start, free]).unwrap();

    let revision = s.revision();
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":origin.x_nm,"y_nm":origin.y_nm,"rotation":0,"side":"back","expected_revision":revision}),
    )
    .unwrap();
    let track = &view(&s).tracks()[0];
    assert_eq!(track.start, Point { x_nm: 2 * origin.x_nm - start.x_nm, y_nm: start.y_nm });
    assert_eq!(track.end, free);
    assert_eq!(track.layer, front);
    assert_eq!(s.revision(), revision + 1);
}

/// A legal quarter-turn carries an off-center end on a rectangular pad and leaves the far end fixed.
#[test]
fn quarter_turn_carries_an_off_center_track_end() {
    use crate::board::track::tests::{copper_layers, route, view};

    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let symbol = s.project().unwrap().symbols().first().unwrap().id();
    let origin = Point { x_nm: 4_000_000, y_nm: 1_000_000 };
    s.apply(
        PLACE_FOOTPRINT,
        &json!({"symbol":symbol,"definition":"resistor-0603","mapping":{"1":"1","2":"2"},
            "x_nm":origin.x_nm,"y_nm":origin.y_nm,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().next().unwrap();
    let id = footprint.id;
    let pad = footprint.pads.iter().find(|pad| pad.number == "1").unwrap();
    let start = Point { x_nm: pad.center.x_nm + 180_000, y_nm: pad.center.y_nm + 100_000 };
    let free = Point { x_nm: start.x_nm - 2_000_000, y_nm: start.y_nm };
    let front = copper_layers(&s)[0];
    route(&mut s, front, &[start, free]).unwrap();

    let revision = s.revision();
    s.apply(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":id,"x_nm":origin.x_nm,"y_nm":origin.y_nm,"rotation":90,"expected_revision":revision}))
        .unwrap();
    let track = &view(&s).tracks()[0];
    assert_eq!(track.start, Point { x_nm: origin.x_nm + (start.y_nm - origin.y_nm), y_nm: origin.y_nm - (start.x_nm - origin.x_nm) });
    assert_eq!(track.end, free);
    assert_eq!(s.revision(), revision + 1);
}

/// A track near a rounded corner stays electrically separate before and after the footprint moves.
#[test]
fn moving_a_footprint_leaves_track_outside_rounded_copper_fixed() {
    use crate::board::track::tests::{copper_layers, routed_fixture, view};

    let mut s = routed_fixture();
    let footprint = view(&s).footprints().next().unwrap();
    let id = footprint.id;
    let origin = footprint.position;
    let pad = footprint.pads.iter().find(|pad| pad.net.is_some()).unwrap();
    let pad_key = (id.clone(), pad.number.clone());
    let start = Point { x_nm: pad.center.x_nm + 290_000, y_nm: pad.center.y_nm + 290_000 };
    let free = Point { x_nm: start.x_nm, y_nm: start.y_nm + 1_500_000 };
    let front = copper_layers(&s)[0];
    s.apply(
        crate::ADD_TRACKS,
        &json!({"layer":front,"width_nm":super::super::track::MIN_WIDTH_NM,"points":[start,free],"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = view(&s).tracks().to_vec();
    let track = before[0].id;
    let copper = view(&s).copper().unwrap();
    assert_ne!(copper.pads[&pad_key], copper.tracks[&track], "the copper does not reach the rounded pad");
    assert!(!view(&s).copper_nets().unwrap().contains_key(&track));
    let position = Point { x_nm: origin.x_nm + 2_000_000, y_nm: origin.y_nm };
    assert!(view(&s).carried_tracks(&id, position, 0, false).unwrap().is_empty());
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":position.x_nm,"y_nm":position.y_nm,"rotation":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    assert_eq!(view(&s).tracks(), before);
    let copper = view(&s).copper().unwrap();
    assert_ne!(copper.pads[&pad_key], copper.tracks[&track]);
    assert!(!view(&s).copper_nets().unwrap().contains_key(&track));
}

/// Direct pad overlap between different nets is refused on the same layer, even without routing.
#[test]
fn footprint_move_refuses_direct_pad_short_on_shared_copper() {
    use crate::board::track::tests::{routed_fixture, view};

    let mut s = routed_fixture();
    let board = view(&s);
    assert!(board.tracks().is_empty() && board.vias().is_empty());
    let source = board.footprints().next().unwrap();
    let pad = source.pads.iter().find(|p| p.net.is_some()).unwrap();
    let target = board
        .footprints()
        .filter(|footprint| footprint.id != source.id)
        .flat_map(|footprint| footprint.pads)
        .find(|other| other.net.is_some() && other.net != pad.net)
        .unwrap();
    let x = source.position.x_nm + target.center.x_nm - pad.center.x_nm;
    let y = source.position.y_nm + target.center.y_nm - pad.center.y_nm;
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let mut transform = json!({"footprint":source.id,"x_nm":x,"y_nm":y,"rotation":0,"expected_revision":revision});
    let error = s.apply(SET_FOOTPRINT_TRANSFORM, &transform).unwrap_err().to_string();
    assert!(error.contains("would connect nets"), "{error}");
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);

    transform["side"] = json!("back");
    s.apply(SET_FOOTPRINT_TRANSFORM, &transform).unwrap();
    assert_eq!(s.revision(), revision + 1, "overlap across separate layers is allowed");
}
