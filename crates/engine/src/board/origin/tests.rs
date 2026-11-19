use super::*;
use crate::{
    EXPORT_FABRICATION, SET_RULES,
    board::{
        drc::tests::outline,
        fabrication::Output,
        track::tests::{copper_layers, route_with_vias, routed_fixture, view},
    },
    persistence::{decode_documents, encode_documents, test_board_path},
};

const MM: i64 = 1_000_000;

/// Set the origin through the same addressed, revision-checked command that GUI and MCP use.
fn set(session: &mut Session, origin: Value) -> Result<u64, CommandError> {
    let board = session.project().unwrap().board_target().unwrap().board;
    session.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":session.revision(),"origin":origin}))
}

/// A routed two-layer board with an outline, tracks on both sides and a plated via.
fn board() -> Session {
    let mut s = routed_fixture();
    outline(&mut s);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let y = 20 * MM;
    route_with_vias(
        &mut s,
        front,
        &[Point { x_nm: 5 * MM, y_nm: y }, Point { x_nm: 7 * MM, y_nm: y }, Point { x_nm: 9 * MM, y_nm: y }],
        &[(1, back)],
    )
    .unwrap();
    s
}

/// Every coordinate pair of a Gerber (integer nm) or Excellon (mm, three decimals, read as
/// integer micrometres) file, in file order, with the rest of each line.
fn coordinates(output: &Output) -> Vec<(i64, i64, String)> {
    let drill = output.name.ends_with(".drl");
    let number = |text: &str| -> i64 { if drill { text.replace('.', "").parse().unwrap() } else { text.parse().unwrap() } };
    output
        .contents
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('X')?;
            let (x, rest) = rest.split_once('Y')?;
            let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.')).unwrap_or(rest.len());
            Some((number(x), number(&rest[..end]), rest[end..].to_owned()))
        })
        .collect()
}

/// Moving the origin 10 mm right and 10 mm up (Y up, seen from the top) moves every Gerber and
/// Excellon coordinate by exactly -10.0000 mm on both axes and changes nothing else. The result
/// names the origin it used.
#[test]
fn moving_the_origin_shifts_every_output_coordinate_exactly() {
    let mut s = board();
    let before = view(&s).fabrication().unwrap();
    assert_eq!(before.origin.source, Source::Builtin);
    assert_eq!(before.origin.anchor, Anchor::BoardZero);
    // Board Y grows down, so a point 10 mm above board zero is at y = -10 mm.
    set(&mut s, json!({"kind":"point","x_nm":10 * MM,"y_nm":-10 * MM})).unwrap();
    let after = view(&s).fabrication().unwrap();
    assert_eq!(before.files.len(), after.files.len());
    let mut checked = 0;
    for (old, new) in before.files.iter().zip(&after.files) {
        assert_eq!(old.name, new.name);
        let (old_xy, new_xy) = (coordinates(old), coordinates(new));
        assert_eq!(old_xy.len(), new_xy.len(), "{}", old.name);
        let step = if old.name.ends_with(".drl") { 10_000 } else { 10 * MM };
        for (a, b) in old_xy.iter().zip(&new_xy) {
            assert_eq!((b.0, b.1, &b.2), (a.0 - step, a.1 - step, &a.2), "{}", old.name);
            checked += 1;
        }
        let strip = |text: &str| text.lines().filter(|line| !line.starts_with('X')).map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(strip(&old.contents), strip(&new.contents), "{}: only coordinates change", old.name);
    }
    assert!(checked > 20, "the fixture exercises pads, tracks, vias, silk, outline and holes");
    assert!(coordinates(after.files.iter().find(|f| f.name.ends_with("-PTH.drl")).unwrap()).iter().any(|(x, _, _)| *x == -3_000));

    let id = s.project().unwrap().board.placement_origin.as_ref().unwrap().id;
    let result = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    let origin = &result["fabrication"]["origin"];
    assert_eq!(origin["anchor"], json!({"kind":"point","x_nm":10 * MM,"y_nm":-10 * MM}));
    assert_eq!(origin["source"], json!({"kind":"board","origin":id}));
    assert_eq!(origin["at"], json!({"x_nm":10 * MM,"y_nm":-10 * MM}));
    assert_eq!(origin["axes"], "x_right_y_up");
}

/// Without a saved origin, and with board zero chosen explicitly, the outputs are the same bytes.
#[test]
fn board_zero_keeps_the_existing_output_bytes() {
    let mut s = board();
    let before = view(&s).fabrication_outputs().unwrap();
    set(&mut s, json!({"kind":"board_zero"})).unwrap();
    let explicit = view(&s).fabrication().unwrap();
    assert_eq!(explicit.files, before);
    assert!(matches!(explicit.origin.source, Source::Board { .. }));
    assert_eq!(Frame::BOARD_ZERO.to_output(Point { x_nm: 3, y_nm: 4 }), Point { x_nm: 3, y_nm: -4 });
}

/// The outline corner follows the outline: its lower-left becomes (0, 0) and the board lies in
/// the positive quadrant. Without an outline the origin cannot resolve and export says why.
#[test]
fn outline_lower_left_puts_the_board_in_the_positive_quadrant() {
    let mut s = routed_fixture();
    set(&mut s, json!({"kind":"outline_lower_left"})).unwrap();
    let error = view(&s).placement_origin().unwrap_err();
    assert!(error.contains("Board Outline"), "{error}");
    assert!(s.inspect()["project"]["boards"][0]["placement_origin"]["effective"].is_null());
    outline(&mut s);
    let resolved = view(&s).placement_origin().unwrap();
    assert_eq!(resolved.at, Point { x_nm: -5 * MM, y_nm: 35 * MM });
    let outputs = view(&s).fabrication_outputs().unwrap();
    let profile = &outputs.iter().find(|f| f.name.ends_with("-Outline.gbr")).unwrap().contents;
    let corners: Vec<_> =
        coordinates(outputs.iter().find(|f| f.name.ends_with("-Outline.gbr")).unwrap()).into_iter().map(|(x, y, _)| (x, y)).collect();
    assert!(corners.contains(&(0, 0)) && corners.contains(&(30 * MM, 40 * MM)), "{profile}");
    assert!(corners.iter().all(|(x, y)| *x >= 0 && *y >= 0));
    assert_eq!(s.inspect()["project"]["boards"][0]["placement_origin"]["effective"]["at"], json!({"x_nm":-5 * MM,"y_nm":35 * MM}));
}

/// The command is one undo step that keeps the origin's identity, ignores repeats and resets,
/// and refuses bad anchors, stale revisions and unknown boards without changing anything.
#[test]
fn setting_the_origin_is_addressed_atomic_and_undoable() {
    let mut s = board();
    let original = s.snapshot().unwrap();
    set(&mut s, Value::Null).unwrap();
    assert!(Arc::ptr_eq(&original, &s.snapshot().unwrap()));
    set(&mut s, json!({"kind":"outline_lower_left"})).unwrap();
    let first = s.snapshot().unwrap();
    let id = first.board.placement_origin.as_ref().unwrap().id;
    let revision = s.revision();
    set(&mut s, json!({"kind":"outline_lower_left"})).unwrap();
    assert_eq!(s.revision(), revision);
    for bad in [
        json!({"kind":"point","x_nm":crate::schematic::MAX_CENTER_NM + 1,"y_nm":0}),
        json!({"kind":"point","x_nm":0}),
        json!({"kind":"board_zero","x_nm":0}),
        json!({"kind":"drill"}),
        json!("board_zero"),
    ] {
        assert!(set(&mut s, bad.clone()).is_err(), "{bad}");
        assert!(Arc::ptr_eq(&first, &s.snapshot().unwrap()));
    }
    let mut params = json!({"origin":{"kind":"board_zero"},"expected_revision":revision});
    assert!(s.apply(SET_PLACEMENT_ORIGIN, &params).is_err(), "the board is required");
    params["board"] = json!(Uuid::nil());
    assert!(s.apply(SET_PLACEMENT_ORIGIN, &params).is_err());
    params["board"] = json!(first.board_target().unwrap().board);
    params["expected_revision"] = json!(revision - 1);
    assert!(s.apply(SET_PLACEMENT_ORIGIN, &params).is_err());
    assert!(Arc::ptr_eq(&first, &s.snapshot().unwrap()));
    set(&mut s, json!({"kind":"point","x_nm":MM,"y_nm":MM})).unwrap();
    assert_eq!(s.project().unwrap().board.placement_origin.as_ref().unwrap().id, id);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&first, &s.snapshot().unwrap());
    set(&mut s, Value::Null).unwrap();
    crate::assert_same_content(&original, &s.snapshot().unwrap());
}

/// Format 21 is written only while an origin is saved, carries rules and every earlier field,
/// reopens to the same bytes, and removing the origin restores the earlier encoding.
#[test]
fn origin_format_round_trips_and_reset_restores_previous_bytes() {
    let mut s = board();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    set(&mut s, json!({"kind":"point","x_nm":2 * MM,"y_nm":3 * MM})).unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_RULES, &json!({"board":board,"expected_revision":s.revision(),"rules":crate::board::rules::Values::default()})).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&documents);
    let saved: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/21");
    assert_eq!(saved["placement_origin"]["anchor"], json!({"kind":"point","x_nm":2 * MM,"y_nm":3 * MM}));
    assert!(saved["rules"].is_object() && saved["vias"].is_array() && saved["tracks"].is_array());
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), documents);

    let text = String::from_utf8(documents[&path].clone()).unwrap();
    for (from, to) in [("pcbcraft-board/21", "pcbcraft-board/19"), ("\"pcbcraft-origin/1\"", "\"pcbcraft-origin/2\"")] {
        let mut altered = documents.clone();
        altered.insert(path.clone(), text.replacen(from, to, 1).into_bytes());
        assert!(decode_documents(&altered).is_err(), "{to}");
    }

    set(&mut s, Value::Null).unwrap();
    s.apply(SET_RULES, &json!({"board":board,"expected_revision":s.revision(),"rules":null})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    let mut missing = original.clone();
    let text = String::from_utf8(original[&path].clone()).unwrap();
    let format = serde_json::from_str::<Value>(&text).unwrap()["format"].as_str().unwrap().to_owned();
    missing.insert(path, text.replacen(&format, "pcbcraft-board/21", 1).into_bytes());
    assert!(decode_documents(&missing).is_err(), "format 21 requires an origin");
}

/// Format 23 carries the stack fields together with a placement origin: a board with an origin,
/// a copper role and a finished thickness saves as format 23, reopens with every value and
/// re-encodes to the same bytes, and header 21, which accepts the origin, refuses the stack
/// fields (header 20 requires zones, covered separately by the setup tests). Removing the origin
/// keeps format 23; removing the stack fields instead falls back to format 21.
#[test]
fn stack_format_carries_the_placement_origin() {
    let mut s = board();
    let plain = s.inspect()["project"]["board"]["setup"].clone();
    let mut candidate = plain.clone();
    candidate["stackup"]["strata"][0]["role"] = json!("signal");
    candidate["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    s.apply(crate::SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision()})).unwrap();
    set(&mut s, json!({"kind":"point","x_nm":2 * MM,"y_nm":3 * MM})).unwrap();

    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&documents);
    let saved: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/23");
    assert_eq!(saved["placement_origin"]["anchor"], json!({"kind":"point","x_nm":2 * MM,"y_nm":3 * MM}));
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(reopened.board_setup(), s.project().unwrap().board_setup());
    assert_eq!(reopened.board_setup().unwrap().stackup().finished_thickness_nm, Some(1_600_000));
    let target = reopened.board_target().unwrap();
    assert_eq!(reopened.board_view(target).unwrap().authored_placement_origin(), view(&s).authored_placement_origin());
    assert_eq!(encode_documents(reopened).unwrap(), documents);

    let text = String::from_utf8(documents[&path].clone()).unwrap();
    let mut altered = documents.clone();
    altered.insert(path.clone(), text.replacen("pcbcraft-board/23", "pcbcraft-board/21", 1).into_bytes());
    match decode_documents(&altered) {
        Ok(_) => panic!("format 21 opened the stack fields"),
        Err(error) => assert!(error.contains("require board format 23"), "{error}"),
    }

    let format = |s: &Session| {
        let documents = encode_documents(s.snapshot().unwrap()).unwrap();
        serde_json::from_slice::<Value>(&documents[&test_board_path(&documents)]).unwrap()["format"].clone()
    };
    set(&mut s, Value::Null).unwrap();
    assert_eq!(format(&s), "pcbcraft-board/23");
    set(&mut s, json!({"kind":"board_zero"})).unwrap();
    s.apply(crate::SET_BOARD_SETUP, &json!({"setup":plain,"expected_revision":s.revision()})).unwrap();
    assert_eq!(format(&s), "pcbcraft-board/21");
}

/// The last in-range coordinates are accepted on both sides and one step beyond is refused.
#[test]
fn point_origins_accept_the_whole_coordinate_range_and_nothing_beyond() {
    let max = crate::schematic::MAX_CENTER_NM;
    let mut s = board();
    for (x, y) in [(max, max), (-max, -max), (max, -max)] {
        set(&mut s, json!({"kind":"point","x_nm":x,"y_nm":y})).unwrap();
        assert_eq!(view(&s).placement_origin().unwrap().at, Point { x_nm: x, y_nm: y });
    }
    let before = s.snapshot().unwrap();
    for (x, y) in [(-max - 1, 0), (0, -max - 1), (max + 1, 0), (0, max + 1)] {
        assert!(set(&mut s, json!({"kind":"point","x_nm":x,"y_nm":y})).is_err(), "{x} {y}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// A saved origin is read as strictly as a request: a nil identity, an out-of-range point and a
/// field its kind does not use are each refused, and the unaltered file still opens.
#[test]
fn saved_origins_refuse_nil_ids_out_of_range_points_and_extra_fields() {
    let max = crate::schematic::MAX_CENTER_NM;
    let mut s = board();
    set(&mut s, json!({"kind":"point","x_nm":2 * MM,"y_nm":3 * MM})).unwrap();
    let id = s.project().unwrap().board.placement_origin.as_ref().unwrap().id;
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&documents);
    let text = String::from_utf8(documents[&path].clone()).unwrap();
    let anchor = format!(r#"{{"kind":"point","x_nm":{},"y_nm":{}}}"#, 2 * MM, 3 * MM);
    assert!(text.contains(&anchor) && text.contains(&id.to_string()));
    assert!(decode_documents(&documents).is_ok());
    for (from, to) in [
        (id.to_string(), Uuid::nil().to_string()),
        (anchor.clone(), format!(r#"{{"kind":"point","x_nm":{},"y_nm":0}}"#, max + 1)),
        (anchor.clone(), r#"{"kind":"board_zero","x_nm":0}"#.to_owned()),
        (anchor.clone(), r#"{"kind":"outline_lower_left","y_nm":0}"#.to_owned()),
    ] {
        let mut altered = documents.clone();
        altered.insert(path.clone(), text.replacen(&from, &to, 1).into_bytes());
        assert!(decode_documents(&altered).is_err(), "{to}");
    }
}

/// Excellon coordinates are rounded to 0.001 mm from integer nanometres with halves going up,
/// so a hole at a half micrometre writes the same rounding on both sides of zero and after any
/// whole-millimetre shift, and a hole just left of zero writes 0.000 rather than -0.000.
#[test]
fn drill_coordinates_round_the_same_way_wherever_the_origin_is() {
    let mut s = routed_fixture();
    outline(&mut s);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let (via_x, via_y) = (7 * MM + 2_500, 20 * MM);
    route_with_vias(
        &mut s,
        front,
        &[Point { x_nm: 5 * MM, y_nm: via_y }, Point { x_nm: via_x, y_nm: via_y }, Point { x_nm: 9 * MM, y_nm: via_y }],
        &[(1, back)],
    )
    .unwrap();
    let drill = |s: &Session| view(s).fabrication_outputs().unwrap().into_iter().find(|f| f.name.ends_with("-PTH.drl")).unwrap();
    for (offset, written) in
        [(-2_500, "0.003"), (2_500, "-0.002"), (400, "0.000"), (-400, "0.000"), (-10 * MM - 2_500, "10.003"), (10 * MM + 2_500, "-10.002")]
    {
        set(&mut s, json!({"kind":"point","x_nm":via_x + offset,"y_nm":via_y})).unwrap();
        let near = drill(&s);
        assert!(near.contents.contains(&format!("\nX{written}Y0.000\n")), "{offset}: {}", near.contents);
        assert!(!near.contents.contains("-0.000"));
        set(&mut s, json!({"kind":"point","x_nm":via_x + offset - 10 * MM,"y_nm":via_y + 10 * MM})).unwrap();
        let shifted = drill(&s);
        let (before, after) = (coordinates(&near), coordinates(&shifted));
        assert_eq!(before.len(), after.len());
        for (a, b) in before.iter().zip(&after) {
            assert_eq!((b.0, b.1), (a.0 + 10_000, a.1 + 10_000), "{offset}");
        }
    }
}

/// Move a footprint through the shared transform command.
fn place(session: &mut Session, footprint: &str, at: Point, rotation: u16, side: &str) {
    let params = json!({"footprint":footprint,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"side":side,"expected_revision":session.revision()});
    session.apply(crate::SET_FOOTPRINT_TRANSFORM, &params).unwrap();
}

/// Rows of one top and one bottom part at 30 degrees, measured from an origin at board
/// (10 mm, 30 mm), under both bottom-side conventions.
#[test]
fn bottom_part_at_thirty_degrees_follows_each_convention() {
    let mut s = routed_fixture();
    let ids: Vec<_> = view(&s).footprints().map(|f| f.id).collect();
    let (bottom, top) = (&ids[0], &ids[1]);
    place(&mut s, bottom, Point { x_nm: 40 * MM, y_nm: 25 * MM }, 30, "back");
    place(&mut s, top, Point { x_nm: 40 * MM, y_nm: 45 * MM }, 30, "front");
    set(&mut s, json!({"kind":"point","x_nm":10 * MM,"y_nm":30 * MM})).unwrap();
    let frame = view(&s).placement_origin().unwrap().frame();
    let row = |id: &String, convention| {
        let footprint = view(&s).footprints().find(|f| &f.id == id).unwrap();
        frame.place(footprint.position, footprint.rotation, footprint.bottom, convention)
    };
    // Bottom part, seen from the top: X = 40 - 10 = 30 mm; Y up = 30 - 25 = 5 mm; rotation as
    // placed, 30, still counterclockwise from the top (IPC-2581, 3.3.2).
    assert_eq!(row(bottom, BottomSide::ViewedFromTop), Placed { at: Point { x_nm: 30 * MM, y_nm: 5 * MM }, rotation: 30 });
    // Seen from below: X = -(40 - 10) = -30 mm; Y unchanged, 5 mm; rotation 360 - 30 = 330.
    assert_eq!(row(bottom, BottomSide::ViewedFromBottom), Placed { at: Point { x_nm: -30 * MM, y_nm: 5 * MM }, rotation: 330 });
    // Top part under either convention: X = 40 - 10 = 30 mm; Y up = 30 - 45 = -15 mm; rotation 30.
    for convention in [BottomSide::ViewedFromTop, BottomSide::ViewedFromBottom] {
        assert_eq!(row(top, convention), Placed { at: Point { x_nm: 30 * MM, y_nm: -15 * MM }, rotation: 30 });
    }
    assert_eq!(BottomSide::default(), BottomSide::ViewedFromTop);
    assert_eq!(serde_json::to_value(BottomSide::ViewedFromBottom).unwrap(), "viewed_from_bottom");
}

/// Each convention reproduces where the pads really are. Seen from the top, a bottom row is
/// the mirrored footprint turned by its rotation; seen from below (X negated), it is the
/// unmirrored footprint turned by its rotation. Checked on every quarter turn, which is exact.
#[test]
fn each_convention_reproduces_the_bottom_pads() {
    let mut s = routed_fixture();
    let id = view(&s).footprints().next().unwrap().id;
    let at = Point { x_nm: 40 * MM, y_nm: 25 * MM };
    let pads = |s: &Session| {
        let footprint = view(s).footprints().find(|f| f.id == id).unwrap();
        (footprint.position, footprint.rotation, footprint.pads.iter().map(|pad| pad.center).collect::<Vec<_>>())
    };
    place(&mut s, &id, at, 0, "front");
    // Library offsets from the placement point, Y up.
    let local: Vec<_> = pads(&s).2.iter().map(|pad| (pad.x_nm - at.x_nm, at.y_nm - pad.y_nm)).collect();
    assert!(local.iter().any(|(x, y)| (*x, *y) != (0, 0)));
    set(&mut s, json!({"kind":"point","x_nm":10 * MM,"y_nm":30 * MM})).unwrap();
    let frame = view(&s).placement_origin().unwrap().frame();
    let turn = |(x, y): (i64, i64), degrees: u16| match degrees {
        0 => (x, y),
        90 => (-y, x),
        180 => (-x, -y),
        _ => (y, -x),
    };
    for rotation in [0, 90, 180, 270] {
        place(&mut s, &id, at, rotation, "back");
        let (position, turned, board) = pads(&s);
        assert_eq!(turned, rotation);
        let top = frame.place(position, turned, true, BottomSide::ViewedFromTop);
        let below = frame.place(position, turned, true, BottomSide::ViewedFromBottom);
        for (&(x, y), pad) in local.iter().zip(&board) {
            let seen = frame.to_output(*pad);
            let (dx, dy) = turn((-x, y), top.rotation);
            assert_eq!((seen.x_nm, seen.y_nm), (top.at.x_nm + dx, top.at.y_nm + dy), "from the top at {rotation}");
            let (dx, dy) = turn((x, y), below.rotation);
            assert_eq!((-seen.x_nm, seen.y_nm), (below.at.x_nm + dx, below.at.y_nm + dy), "from below at {rotation}");
        }
    }
}

/// Rotations are folded into one turn: 359 is 1 seen from below, and 360 or 750 write the
/// same rows as 0 or 30.
#[test]
fn rotation_is_folded_into_one_turn() {
    let at = Point { x_nm: 40 * MM, y_nm: 25 * MM };
    let frame = Frame::BOARD_ZERO;
    let placed = |rotation, convention| frame.place(at, rotation, true, convention).rotation;
    assert_eq!(placed(359, BottomSide::ViewedFromTop), 359);
    assert_eq!(placed(359, BottomSide::ViewedFromBottom), 1);
    for convention in [BottomSide::ViewedFromTop, BottomSide::ViewedFromBottom] {
        for on_bottom in [false, true] {
            assert_eq!(frame.place(at, 360, on_bottom, convention), frame.place(at, 0, on_bottom, convention));
            assert_eq!(frame.place(at, 750, on_bottom, convention), frame.place(at, 30, on_bottom, convention));
        }
        assert_eq!(placed(0, convention), 0);
    }
}

/// Both convention names read back, and an unknown name is refused because it is not one of the two.
#[test]
fn bottom_side_names_round_trip() {
    for convention in [BottomSide::ViewedFromTop, BottomSide::ViewedFromBottom] {
        let name = serde_json::to_value(convention).unwrap();
        assert_eq!(serde_json::from_value::<BottomSide>(name).unwrap(), convention);
    }
    assert_eq!(serde_json::from_value::<BottomSide>(json!("viewed_from_top")).unwrap(), BottomSide::ViewedFromTop);
    assert_eq!(serde_json::from_value::<BottomSide>(json!("viewed_from_bottom")).unwrap(), BottomSide::ViewedFromBottom);
    let refused = serde_json::from_value::<BottomSide>(json!("viewed_from_below")).unwrap_err().to_string();
    assert!(refused.contains("unknown variant `viewed_from_below`"), "{refused}");
    assert!(refused.contains("`viewed_from_top`") && refused.contains("`viewed_from_bottom`"), "{refused}");
}
