use super::{
    catalog::{self, Measure},
    commands::*,
    probes::{Op, trace},
    *,
};
use crate::{
    CommandError, Project, Session,
    board::{self, update},
    persistence::{Documents, decode_documents, encode_documents, test_board_path},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Execute the same addressed native/relationship commands that production clients discover.
pub(crate) fn run(s: &mut Session, command: &str, mut params: Value) -> Result<Value, CommandError> {
    if crate::command(command).unwrap().params.iter().any(|p| p.name == "expected_revision") {
        params["expected_revision"] = json!(s.revision());
    }
    s.execute(command, &params)
}

/// Real pinned resistor footprints on a native board, optionally with accepted physical history.
pub(crate) fn fixture(baseline: bool) -> Session {
    let mut s = Session::default();
    run(&mut s, "project.new", json!({"create_board":true})).unwrap();
    for (i, x) in [0, 10_000_000, 20_000_000].into_iter().enumerate() {
        run(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0})).unwrap();
        let id = *s.project().unwrap().components.keys().next_back().unwrap();
        let d = board::definition::basic_components(if i == 0 { "resistor-0402" } else { "resistor-0603" }).unwrap();
        run(&mut s, crate::ASSIGN_FOOTPRINT, json!({"component":format!("component-{id}"),"library":d.library,"definition":d.id,"definition_revision":d.revision,"mapping":{"1":"1","2":"2"}})).unwrap();
        run(&mut s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":format!("component-{id}"),"x_nm":x + 3_000_000,"y_nm":2_000_000})).unwrap();
    }
    if baseline {
        run(&mut s, update::INITIALIZE_BASELINE, json!({})).unwrap();
    }
    s
}

/// Select a reference exactly as the engine's public catalog emits it.
pub(crate) fn pick(s: &mut Session, index: usize, tag: &str, pad: Option<&str>) -> Value {
    let id = s.project().unwrap().board.footprints()[index].id;
    let response = run(s, FEATURES, json!({"fp":format!("footprint-{id}")})).unwrap();
    response["features"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|item| {
            let r = &item["reference"];
            (r["tag"] == tag && pad.is_none_or(|pad| r["pad"] == pad)).then(|| r.clone())
        })
        .unwrap()
}

pub(crate) fn add(s: &mut Session, kind: &str, a: Value, b: Value) -> Value {
    run(s, ADD, json!({"relation":{"kind":kind,"refs":[a,b]}})).unwrap()["id"].clone()
}

pub(crate) fn origin() -> Value {
    json!({"tag":"board.origin"})
}
pub(crate) fn bytes(s: &Session) -> Documents {
    encode_documents(s.snapshot().unwrap()).unwrap()
}
pub(super) fn reviewed(s: &mut Session, command: &str, params: Value) -> Value {
    run(s, PREVIEW, json!({"intent":{"command":command,"params":params}})).unwrap()["relation_preview"].clone()
}

/// Build exactly the two disclosed decision sets; tests deliberately corrupt them separately.
pub(super) fn decision(preview: &Value) -> Value {
    let disclosure = &preview["disclosure"];
    let ids = |key: &str| disclosure[key].as_array().unwrap().iter().map(|row| row["id"].clone()).collect::<Vec<_>>();
    json!({"token":preview["token"], "accept_loss":ids("losses"), "confirm_remaps":ids("remaps")})
}

/// Compare immutable snapshot, revision and both history stacks across a refusal.
pub(crate) fn unchanged(s: &mut Session, attempt: impl FnOnce(&mut Session) -> Result<Value, CommandError>) -> CommandError {
    let before = (s.snapshot().unwrap(), s.revision(), s.undo.len(), s.redo.len());
    let (undo, redo) = (s.undo.clone(), s.redo.clone());
    let error = attempt(s).unwrap_err();
    assert!(Arc::ptr_eq(&before.0, &s.snapshot().unwrap()));
    assert_eq!((before.1, before.2, before.3), (s.revision(), s.undo.len(), s.redo.len()));
    assert!(undo.iter().zip(&s.undo).all(|(before, after)| before.same_entry(after)), "refusal changed an undo entry");
    assert!(redo.iter().zip(&s.redo).all(|(before, after)| before.same_entry(after)), "refusal changed a redo entry");
    error
}

/// Catalog follows the same native rectangle projection across side, turns and odd sizes.
#[test]
fn a_cat_ref_transforms_and_closed_catalog() {
    let s = fixture(false);
    let base = s.project().unwrap();
    for (bottom, angle) in [false, true].into_iter().flat_map(|b| [0, 90, 180, 270, 37].map(|angle| (b, angle))) {
        for pad_turn in [0, 90, 30] {
            let mut p = base.clone();
            let board = Arc::make_mut(&mut p.board);
            let f = &mut board.footprints[0];
            (f.bottom, f.rotation, f.x_nm, f.y_nm) = (bottom, angle, 0, 0);
            let def = Arc::make_mut(board.definitions.get_mut(&f.key().unwrap()).unwrap());
            def.pads[1].center = Point { x_nm: -1_000_000, y_nm: 3_000_000 };
            let second = def.pads[1].number.clone();
            let pad = &mut def.pads[0];
            (pad.center, pad.width_nm, pad.height_nm, pad.rotation, pad.shape, pad.corner_radius_nm) =
                (Point { x_nm: 2_000_000, y_nm: 1_000_000 }, 123_457, 765_431, pad_turn, board::definition::PadShape::Rectangle, 0);
            let number = pad.number.clone();
            let expected = board::view::rectangle_points(f, pad.center, pad.width_nm, pad.height_nm, pad.rotation);
            let center = f.point(pad.center);
            let available = catalog::features(&p, &p.board.footprints()[0]);
            assert!(
                available.iter().any(|(r, w)| matches!(r, Reference::PadCenter { pad, .. } if pad == &number) && *w == Witness::Point { at: center })
            );
            let second_center = p.board.footprints()[0].point(Point { x_nm: -1_000_000, y_nm: 3_000_000 });
            assert!(
                available
                    .iter()
                    .any(|(r, w)| matches!(r, Reference::PadCenter {pad,..} if pad == &second) && *w == Witness::Point { at: second_center })
            );
            if bottom && angle == 0 {
                assert_eq!(center, Point { x_nm: -2_000_000, y_nm: 1_000_000 });
                assert_eq!(second_center, Point { x_nm: 1_000_000, y_nm: 3_000_000 });
            }
            let corners: Vec<_> = available
                .iter()
                .filter_map(|(r, w)| match (r, w) {
                    (Reference::PadCorner { pad, .. }, Witness::Point { at }) if pad == &number => Some(*at),
                    _ => None,
                })
                .collect();
            assert_eq!(corners, vec![expected[3], expected[2], expected[1], expected[0]]);
            let edges = available.iter().filter(|(r, _)| matches!(r, Reference::PadEdge {pad,..} if pad == &number)).count();
            assert_eq!(edges, if angle % 90 == 0 && pad_turn % 90 == 0 { 4 } else { 0 });
        }
    }
    let mut p = base.clone();
    let board = Arc::make_mut(&mut p.board);
    let f = &board.footprints[0];
    let def = Arc::make_mut(board.definitions.get_mut(&f.key().unwrap()).unwrap());
    let d = &mut def.pads[0];
    d.shape = board::definition::PadShape::RoundedRectangle;
    d.width_nm = d.height_nm;
    d.corner_radius_nm = d.width_nm / 2;
    let number = d.number.clone();
    let features = catalog::features(&p, &p.board.footprints()[0]);
    assert!(!features.iter().any(|(r, _)| r.pad() == Some(number.as_str()) && !matches!(r, Reference::PadCenter { .. })));
}

/// Signed projection always follows the ordered pair and never the outward normal; slant is a reference.
#[test]
fn a_cat_ref_ordered_measurement_and_refusal() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    let b = pick(&mut s, 1, "pad.edge", Some("1"));
    let a: Reference = serde_json::from_value(a).unwrap();
    let b: Reference = serde_json::from_value(b).unwrap();
    let resolver = catalog::Resolver::new(s.project().unwrap());
    let Some(Measure::Projection { signed_nm: d, .. }) = catalog::measure(&resolver, Kind::PointEdge, &[a.clone(), b.clone()]) else {
        panic!("point-edge")
    };
    let Some(Measure::Projection { signed_nm: r, .. }) = catalog::measure(&resolver, Kind::PointEdge, &[b.clone(), a.clone()]) else {
        panic!("edge-point")
    };
    assert_ne!(d, 0);
    assert_eq!(d, -r);
    let Witness::Segment { a: edge_a, b: edge_b } = catalog::resolve(&resolver, &b).unwrap() else { panic!("edge") };
    let Witness::Point { at } = catalog::resolve(&resolver, &a).unwrap() else { panic!("point") };
    assert_eq!(edge_a.x_nm, edge_b.x_nm);
    assert_eq!(d, edge_a.x_nm - at.x_nm);
    let Some(Measure::Aligned { distance_nm, .. }) = catalog::measure(&resolver, Kind::Aligned, &[Reference::BoardOrigin {}, a.clone()]) else {
        panic!("aligned")
    };
    assert!((distance_nm - 3_000_000_f64.hypot(2_000_000.)).abs() < 1e-9);
    let fp = a.footprint().unwrap().0;
    run(&mut s, crate::SET_FOOTPRINT_TRANSFORM, json!({"footprint":format!("footprint-{fp}"),"x_nm":at.x_nm,"y_nm":10_000_000,"rotation":0}))
        .unwrap();
    assert!(10_000_000 > edge_a.y_nm.max(edge_b.y_nm));
    let resolver = catalog::Resolver::new(s.project().unwrap());
    assert!(
        matches!(catalog::measure(&resolver, Kind::PointEdge, &[a.clone(), b.clone()]), Some(Measure::Projection {signed_nm,..}) if signed_nm == d)
    );
    assert!(
        matches!(catalog::measure(&resolver, Kind::PointEdge, &[b.clone(), a.clone()]), Some(Measure::Projection {signed_nm,..}) if signed_nm == -d)
    );
    let same = pick(&mut s, 0, "pad.center", Some("1"));
    let error = unchanged(&mut s, |s| run(s, ADD, json!({"relation":{"kind":"horizontal","refs":[a,same]}})));
    assert_eq!(error.details()["reason"], "lost_reference");
    let edge = |r: &Reference, side: Side| match r {
        Reference::PadEdge { fp, def, pad, .. } => Reference::PadEdge { fp: *fp, def: def.clone(), pad: pad.clone(), side },
        _ => panic!(),
    };
    let a: Reference = serde_json::from_value(pick(&mut s, 0, "pad.edge", Some("1"))).unwrap();
    let resolver = catalog::Resolver::new(s.project().unwrap());
    assert!(catalog::measure(&resolver, Kind::Gap, &[edge(&a, Side::Top), edge(&b, Side::Left)]).is_none());
    let (left, right) = (edge(&a, Side::Right), edge(&b, Side::Left));
    let Some(Measure::Projection { signed_nm: gap, .. }) = catalog::measure(&resolver, Kind::Gap, &[left.clone(), right.clone()]) else {
        panic!("parallel gap")
    };
    assert!(gap > 0);
    assert!(matches!(catalog::measure(&resolver, Kind::Gap, &[right,left]), Some(Measure::Projection {signed_nm,..}) if signed_nm == -gap));
}

/// F1 round-trips the original closed tags, adds no clipboard bytes, keeps the current board
/// format, and drops the record once the final row has gone. Known driving rows,
/// unsupported headers and a null record are refused intact.
#[test]
fn a_format_p1_clipboard_and_driving_row_reader() {
    let mut s = fixture(false);
    let plain = bytes(&s);
    let plain_format = serde_json::from_slice::<Value>(&plain[&test_board_path(&plain)]).unwrap()["format"].clone();
    let fp = s.project().unwrap().board.footprints()[0].id;
    let items = json!({"footprints":[format!("footprint-{fp}")]});
    let copy_before = run(&mut s, board::clipboard::COPY, json!({"items":items})).unwrap()["clipboard"].clone();
    let a = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), a);
    assert_eq!(run(&mut s, board::clipboard::COPY, json!({"items":items})).unwrap()["clipboard"], copy_before);
    let docs = bytes(&s);
    let path = test_board_path(&docs);
    let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    assert_eq!(board["format"], plain_format);
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    for (key, value) in [
        ("format", json!("pcbcraft-board/35")),
        ("format", json!("pcbcraft-board/38")),
        ("format", json!("pcbcraft-board/58")),
        ("relations", Value::Null),
    ] {
        let mut changed = board.clone();
        changed[key] = value;
        let mut doc = docs.clone();
        doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&doc).is_err());
    }
    let mut unknown = docs.clone();
    let mut record = board.clone();
    record["relations"]["relations"][0]["refs"][0] = json!({"tag":"outline.edge","drawing":Uuid::from_u128(27)});
    unknown.insert(path.clone(), serde_json::to_vec(&record).unwrap());
    let untouched = unknown.clone();
    assert!(decode_documents(&unknown).unwrap_err().contains("unknown variant"));
    assert_eq!(unknown, untouched);
    let mut changed = board.clone();
    changed["relations"]["relations"][0]["driving"] = json!(true);
    let mut doc = docs.clone();
    doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
    let untouched = doc.clone();
    assert!(decode_documents(&doc).unwrap_err().contains("reference-only reader"));
    assert_eq!(doc, untouched);
    s.open_snapshot(decode_documents(&docs).unwrap());
    run(&mut s, EDIT, json!({"id":id,"edit":{"suppressed":true}})).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "suppressed");
    run(&mut s, EDIT, json!({"id":id,"edit":{"suppressed":false}})).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
    let p = reviewed(&mut s, board::clipboard::CUT, json!({"items":items}));
    let cut = run(&mut s, APPLY, decision(&p)).unwrap()["clipboard"].clone();
    run(&mut s, board::clipboard::PASTE, json!({"text":cut,"reuse_components":true,"x_nm":8_000_000,"y_nm":9_000_000})).unwrap();
    let after = s.project().unwrap();
    assert_eq!(after.board.footprints().len(), 3);
    assert_ne!(after.board.footprints()[2].id, fp);
    let rows = run(&mut s, LIST, json!({})).unwrap()["relations"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "lost");
    assert_eq!(rows[0]["refs"][1]["fp"], fp);
    run(&mut s, DELETE, json!({"ids":[id]})).unwrap();
    let docs = bytes(&s);
    let board: Value = serde_json::from_slice(&docs[&test_board_path(&docs)]).unwrap();
    assert_eq!(board["format"], plain_format);
    assert!(board.get("relations").is_none());
}

/// Relations are an optional board record, not a format: a board with relations keeps the current
/// board format and reopens with the same relation and bytes. The pre-release
/// board/39 and board/58 labels are unsupported, and deleting the last relation restores the
/// plain bytes.
#[test]
fn a_format_relations_keep_the_board_format_and_reopen() {
    let mut s = fixture(false);
    let plain = bytes(&s);
    let a = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), a);
    let listed = run(&mut s, LIST, json!({})).unwrap()["relations"].clone();
    let docs = bytes(&s);
    let path = test_board_path(&docs);
    let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    assert_eq!(board["format"], serde_json::from_slice::<Value>(&plain[&path]).unwrap()["format"]);
    assert!(board["relations"].is_object());

    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&docs).unwrap());
    assert_eq!(run(&mut reopened, LIST, json!({})).unwrap()["relations"], listed);
    assert_eq!(listed[0]["id"], id);
    assert_eq!(bytes(&reopened), docs);

    // Isolate header dispatch from geometry and setup. Check semantic and structural witness
    // failures, including a supported-header control for each invalid payload.
    let mut out_of_bounds = board["relations"].clone();
    out_of_bounds["relations"][0]["last_resolved"][0]["at"]["x_nm"] = json!(2_000_000_000_i64);
    let mut wrong_arity = board["relations"].clone();
    wrong_arity["relations"][0]["last_resolved"].as_array_mut().unwrap().pop();
    for (record, diagnostic) in [(out_of_bounds, "display witness"), (wrong_arity, "relations: invalid length")] {
        for format in ["pcbcraft-board/39", "pcbcraft-board/58", crate::persistence::CURRENT_BOARD_FORMAT] {
            let changed = json!({"format":format,"relations":record,"footprints":[]});
            let mut doc = docs.clone();
            doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
            let error = decode_documents(&doc).unwrap_err();
            if format == crate::persistence::CURRENT_BOARD_FORMAT {
                assert!(error.contains(diagnostic), "{format}: {error}");
            } else {
                assert_eq!(error, format!("board.json: unsupported board format or incompatible setup: {format}"));
            }
        }
    }
    let mut empty = board.clone();
    empty["relations"]["relations"] = json!([]);
    let mut doc = docs.clone();
    doc.insert(path.clone(), serde_json::to_vec(&empty).unwrap());
    assert!(decode_documents(&doc).unwrap_err().contains("board relations"));

    run(&mut reopened, DELETE, json!({"ids":[id]})).unwrap();
    assert_eq!(bytes(&reopened), plain);
}

/// Pad and via marks share board/43 with a dimension attached to the marked pad. Mark edits do
/// not change the reference and invalidate an older Cut approval; new Cut previews refuse marks.
#[test]
fn a_format_board_43_marks_and_relations_round_trip_and_refuse_cut() {
    use crate::{ADD_VIA, SET_TEST_POINT, board::clipboard::CUT};

    let mut s = board::track::tests::routed_fixture();
    run(&mut s, ADD_VIA, json!({"at":{"x_nm":0,"y_nm":20_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000})).unwrap();
    let project = s.project().unwrap();
    let board = project.board_target().unwrap().board;
    let footprint = format!("footprint-{}", project.board.footprints()[0].id);
    let via_id = project.board.vias[0].id;
    let pad = json!({"kind":"pad","footprint":footprint,"pad":"1"});
    let via = json!({"kind":"via","via":via_id});
    let pad_cut = json!({"items":{"footprints":[footprint]}});
    let via_cut = json!({"items":{"vias":[via_id]}});
    let reference = pick(&mut s, 0, "pad.center", Some("1"));
    let id = add(&mut s, "horizontal", origin(), reference);
    let original = run(&mut s, LIST, json!({})).unwrap()["relations"].clone();
    assert_eq!(original[0]["id"], id);
    assert_eq!(original[0]["status"], "reference");

    let approval = reviewed(&mut s, CUT, pad_cut.clone());
    let mark = |s: &mut Session, target: &Value, purpose: &str, sides: &str| {
        run(s, SET_TEST_POINT, json!({"board":board,"target":target,"purpose":purpose,"sides":sides})).unwrap();
    };
    mark(&mut s, &pad, "assembly", "top");
    mark(&mut s, &via, "fabrication", "bottom");
    let stale = unchanged(&mut s, |s| run(s, APPLY, decision(&approval)));
    assert_eq!(stale.details()["reason"], "stale_preview");
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"], original);
    let saved = bytes(&s);
    let path = test_board_path(&saved);
    let stored: Value = serde_json::from_slice(&saved[&path]).unwrap();
    assert_eq!(stored["format"], "pcbcraft-board/43");
    assert_eq!(stored["relations"]["relations"][0]["id"], id);
    assert_eq!(stored["test_points"], json!([{"target":pad,"flags":{"assembly":"top"}}, {"target":via,"flags":{"fabrication":"bottom"}}]));
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(bytes(&reopened), saved);
    mark(&mut reopened, &pad, "assembly", "both");
    assert_eq!(run(&mut reopened, LIST, json!({})).unwrap()["relations"], original);
    let before_cut = bytes(&reopened);
    let edited: Value = serde_json::from_slice(&before_cut[&path]).unwrap();
    assert_eq!(edited["relations"], stored["relations"]);
    assert_eq!(edited["test_points"][0]["flags"]["assembly"], "both");
    assert_eq!(edited["test_points"][1], stored["test_points"][1]);
    for params in [pad_cut, via_cut] {
        let error = unchanged(&mut reopened, |s| run(s, PREVIEW, json!({"intent":{"command":CUT,"params":params}})));
        assert!(error.to_string().contains("Cut would discard test-point marks"), "{error}");
    }
    assert_eq!(bytes(&reopened), before_cut);
}

/// Adding and deleting a reference-only horizontal dimension on a project opened from legacy
/// `pcbcraft-project/6` must not adopt explicit board registration or `numeric_id_high_water`
/// in the session. Saves write the current project format, `project.json` is unchanged
/// by the relation, and the delete restores the saved documents.
#[test]
fn a_legacy_project_six_keeps_project_json_across_relation_add_and_delete() {
    let s = fixture(false);
    let mut historical = Project::clone(&crate::persistence::diagnostic_snapshot(s.snapshot().unwrap()));
    historical.layout = None;
    historical.numeric_id_high_water = None;
    Arc::make_mut(&mut historical.board).ownership = board::identity::Ownership::Legacy;
    let legacy = crate::persistence::historical_documents(Arc::new(historical), "pcbcraft-project/6");
    let entry: Value = serde_json::from_slice(&legacy[crate::persistence::ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/6");
    for field in ["boards", "layout_root", "numeric_id_high_water", "schematic_instances"] {
        assert!(entry.get(field).is_none(), "{field}");
    }
    let mut s = Session::default();
    s.open_snapshot(decode_documents(&legacy).unwrap());
    assert_eq!(crate::persistence::historical_documents(s.snapshot().unwrap(), "pcbcraft-project/6"), legacy);
    let before = bytes(&s);
    assert_eq!(
        serde_json::from_slice::<Value>(&before[crate::persistence::ENTRY_POINT]).unwrap()["format"],
        crate::persistence::CURRENT_PROJECT_FORMAT
    );

    let a = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), a);
    assert!(matches!(s.project().unwrap().board.ownership, board::identity::Ownership::Legacy));
    assert!(s.project().unwrap().numeric_id_high_water.is_none());
    let with_relation = bytes(&s);
    assert_eq!(with_relation[crate::persistence::ENTRY_POINT], before[crate::persistence::ENTRY_POINT]);
    let path = test_board_path(&with_relation);
    assert_eq!(
        serde_json::from_slice::<Value>(&with_relation[&path]).unwrap()["format"],
        serde_json::from_slice::<Value>(&before[&path]).unwrap()["format"]
    );
    assert_eq!(encode_documents(decode_documents(&with_relation).unwrap()).unwrap(), with_relation);

    run(&mut s, DELETE, json!({"ids":[id]})).unwrap();
    assert!(matches!(s.project().unwrap().board.ownership, board::identity::Ownership::Legacy));
    assert!(s.project().unwrap().numeric_id_high_water.is_none());
    let after = bytes(&s);
    assert_eq!(after[crate::persistence::ENTRY_POINT], before[crate::persistence::ENTRY_POINT]);
    assert_eq!(after, before);
}

/// DEC-1415: an exact reviewed loss survives native save/reopen, Undo/Redo and reappearance
/// of a datum, never resolves from its witness, and never moves the surviving placement.
#[test]
fn a_native_outline_datum_lost_witness_reopen_no_motion() {
    let mut s = board::track::tests::routed_fixture();
    board::drc::tests::outline(&mut s);
    let board_id = s.project().unwrap().board_target().unwrap().board;
    run(&mut s, crate::SET_PLACEMENT_ORIGIN, json!({"board":board_id,"origin":{"kind":"outline_lower_left"}})).unwrap();
    let a = pick(&mut s, 0, "pad.center", Some("1"));
    add(&mut s, "horizontal", origin(), a);
    let first = run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"]["witnesses"].clone();
    let before = s.project().unwrap().board.footprints().to_vec();
    let drawing = s.project().unwrap().board.drawings.as_ref().unwrap().items[0].id;
    run(&mut s, board::movement::MOVE_ITEMS, json!({"items":{"drawings":[drawing]},"dx_nm":1_000_000,"dy_nm":2_000_000})).unwrap();
    let shown = run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"]["witnesses"].clone();
    assert_eq!(shown[0]["at"]["x_nm"].as_i64().unwrap(), first[0]["at"]["x_nm"].as_i64().unwrap() + 1_000_000);
    assert_eq!(shown[0]["at"]["y_nm"].as_i64().unwrap(), first[0]["at"]["y_nm"].as_i64().unwrap() + 2_000_000);
    assert_eq!(shown[1], first[1]);
    assert_eq!(s.project().unwrap().board.footprints(), before.as_slice());
    let items = json!({"drawings":[drawing]});
    let error = unchanged(&mut s, |s| run(s, board::clipboard::DELETE, json!({"items":items})));
    assert_eq!(error.details()["reason"], "would_lose");
    let selection = board::clipboard::BoardSelection::parse(&items).unwrap();
    let native = |cut| board::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, cut).unwrap().to_text().unwrap();
    let (copy_text, cut_text) = (native(false), native(true));
    let copy = run(&mut s, board::clipboard::COPY, json!({"items":items})).unwrap();
    assert_eq!(copy["clipboard"], copy_text);
    assert_eq!(copy["clipboard_notice"], json!({"kind":"relations_not_carried","count":1}));
    let p = reviewed(&mut s, board::clipboard::CUT, json!({"items":items}));
    assert_eq!(p["disclosure"]["losses"][0]["last_resolved"], shown);
    let response = run(&mut s, APPLY, decision(&p)).unwrap();
    assert_eq!(response["clipboard"], cut_text);
    assert_eq!(response["clipboard_notice"], copy["clipboard_notice"]);
    assert_eq!(s.project().unwrap().board.footprints(), before.as_slice());
    let stored = bytes(&s);
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), p["candidate_digest"].as_str().unwrap());
    let (_, ops) = trace(|| s.apply("edit.undo", &json!({})).unwrap());
    assert!(ops.is_empty());
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
    let (_, ops) = trace(|| s.apply("edit.redo", &json!({})).unwrap());
    assert!(ops.is_empty());
    assert_eq!(bytes(&s), stored);
    let (_, ops) = trace(|| s.open_snapshot(decode_documents(&stored).unwrap()));
    assert!(ops.is_empty());
    assert_eq!(bytes(&s), stored);
    assert_eq!(run(&mut s, FEATURES, json!({})).unwrap()["features"], json!([]));
    board::drc::tests::outline(&mut s);
    let result = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(result["relations"][0]["status"], "lost");
    assert_eq!(result["relations"][0]["last_resolved"], shown);
    assert!(result["relations"][0].get("measure").is_none());
    assert_eq!(s.project().unwrap().board.footprints(), before.as_slice());
}

/// Mixed native cut and delete prepare their complete selection once, never the incomplete inner result.
#[test]
fn a_nest_mixed_outline_cut_and_delete_exact_publication() {
    for command in [board::clipboard::CUT, board::clipboard::DELETE, crate::DELETE_DRAWING] {
        let mut s = board::track::tests::routed_fixture();
        board::drc::tests::outline(&mut s);
        let board_id = s.project().unwrap().board_target().unwrap().board;
        run(&mut s, crate::SET_PLACEMENT_ORIGIN, json!({"board":board_id,"origin":{"kind":"outline_lower_left"}})).unwrap();
        let reference = pick(&mut s, 0, "fp.origin", None);
        add(&mut s, "horizontal", origin(), reference);
        let f = s.project().unwrap().board.footprints()[2].id;
        let drawing = s.project().unwrap().board.drawings.as_ref().unwrap().items[0].id;
        let args = if command == crate::DELETE_DRAWING {
            json!({"drawing":drawing})
        } else {
            json!({"items":{"drawings":[drawing],"footprints":[format!("footprint-{f}")]}})
        };
        let observer: Arc<Observe> = Arc::default();
        s.observe_edits(Some(observer.clone()));
        let history = s.undo.len();
        let before = s.snapshot().unwrap();
        let error = unchanged(&mut s, |s| run(s, command, args.clone()));
        assert_eq!(error.details()["reason"], "would_lose");
        let (p, ops) = trace(|| reviewed(&mut s, command, args));
        assert_stage(&ops, if command == crate::DELETE_DRAWING { 0 } else { 2 }, 1, 0);
        assert_eq!(p["disclosure"]["losses"].as_array().unwrap().len(), 1);
        assert!(p.get("clipboard").is_none());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert!(observer.0.lock().unwrap().is_empty());
        let (response, ops) = trace(|| run(&mut s, APPLY, decision(&p)).unwrap());
        assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
        assert_eq!(response.get("clipboard").is_some(), command == board::clipboard::CUT);
        assert_eq!(response["candidate_digest"], p["candidate_digest"]);
        assert_eq!(s.undo.len(), history + 1);
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        let expected = if command == crate::DELETE_DRAWING { before.board.footprints() } else { &before.board.footprints()[..2] };
        assert_eq!(s.project().unwrap().board.footprints(), expected);
        assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
    }
}

/// A structurally out-of-range display record can't be admitted; it is never resolved as native geometry.
#[test]
fn dec_1415_witness_coordinate_and_arity_caps() {
    let mut s = fixture(false);
    let ref_b = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), ref_b);
    let docs = bytes(&s);
    let path = test_board_path(&docs);
    for which in [0, 1] {
        let mut bad = docs.clone();
        let mut board: Value = serde_json::from_slice(&docs[&path]).unwrap();
        let witness = &mut board["relations"]["relations"][0]["last_resolved"];
        if which == 0 {
            witness[0]["at"]["x_nm"] = json!(MAX_WITNESS_NM + 1);
        } else {
            witness.as_array_mut().unwrap().push(json!({"kind":"point","at":{"x_nm":0,"y_nm":0}}));
        }
        bad.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
}

#[derive(Debug, Default)]
pub(crate) struct Observe(pub(crate) std::sync::Mutex<Vec<crate::edits::CommittedEdit>>);
impl crate::edits::EditObserver for Observe {
    fn opened(&self, _: Arc<Project>) {}
    fn committed(&self, edit: crate::edits::CommittedEdit) {
        self.0.lock().unwrap().push(edit);
    }
}

/// Before approval there is no clipboard result, publication or observer event. Invalid, cancelled,
/// wrong-route and replaced capabilities cannot publish and cannot revoke the current review.
#[test]
fn a_generic_exact_cancel_and_engine_result_bounds() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), a);
    let fp = s.project().unwrap().board.footprints()[0].id;
    let items = json!({"footprints":[format!("footprint-{fp}")]});
    let args = json!({"items":items});
    let selection = board::clipboard::BoardSelection::parse(&items).unwrap();
    let expected = board::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, true).unwrap().to_text().unwrap();
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    unchanged(&mut s, |s| run(s, board::clipboard::CUT, args.clone()));
    let expected_revision = s.revision();
    assert!(s.apply(board::clipboard::CUT, &json!({"items":items,"expected_revision":expected_revision})).is_err());
    assert!(!s.available(CANCEL));
    let old = reviewed(&mut s, board::clipboard::CUT, args.clone());
    assert!(s.available(CANCEL));
    assert!(old.get("clipboard").is_none());
    let copied = run(&mut s, board::clipboard::COPY, args.clone()).unwrap();
    assert_eq!(copied["clipboard"], board::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, false).unwrap().to_text().unwrap());
    assert_eq!(copied["clipboard_notice"], json!({"kind":"relations_not_carried","count":1}));
    let remote = s.project().unwrap().board.footprints()[1].id;
    assert!(
        run(&mut s, board::clipboard::COPY, json!({"items":{"footprints":[format!("footprint-{remote}")]}}))
            .unwrap()
            .get("clipboard_notice")
            .is_none()
    );
    run(&mut s, CANCEL, json!({"token":old["token"]})).unwrap();
    assert!(!s.available(CANCEL));
    unchanged(&mut s, |s| run(s, APPLY, decision(&old)));
    let live = reviewed(&mut s, board::clipboard::CUT, args.clone());
    unchanged(&mut s, |s| run(s, CANCEL, json!({"token":old["token"]})));
    assert!(s.available(CANCEL));
    unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, json!({"plan_token":live["token"]})));
    unchanged(&mut s, |s| run(s, PREVIEW, json!({"intent":{"command":"project.new","params":{}}})));
    unchanged(&mut s, |s| {
        s.relation_command(PREVIEW, &json!({"expected_revision":expected_revision,"intent":{"command":"pcb.cut","params":args}}), 1)
    });
    unchanged(&mut s, |s| s.relation_command(LIST, &json!({}), 1));
    let mut invalid = decision(&live);
    invalid["accept_loss"] = json!([]);
    unchanged(&mut s, |s| run(s, APPLY, invalid));
    let mut decision = decision(&live);
    decision["expected_revision"] = json!(expected_revision);
    unchanged(&mut s, |s| s.relation_command(APPLY, &decision, 100));
    assert!(observer.0.lock().unwrap().is_empty());
    unchanged(&mut s, |s| s.apply(APPLY, &decision).map(|v| json!(v)));
    let response = s.relation_command(APPLY, &decision, usize::MAX).unwrap();
    assert_eq!(response["clipboard"].as_str(), Some(expected.as_str()));
    assert_eq!(response["clipboard_notice"], copied["clipboard_notice"]);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    assert_eq!(observer.0.lock().unwrap()[0].command, board::clipboard::CUT);
    unchanged(&mut s, |s| run(s, APPLY, decision.clone()));
}

/// Exercise the real fixed engine result cap, not a smaller injected budget. Both native
/// fragments are valid; the overflowing one becomes too large only in the escaped Apply reply.
#[test]
fn a_generic_public_cut_envelope_fit_and_refusal_keep_current_review() {
    use board::{clipboard, track::Track};
    let mut s = Session::default();
    run(&mut s, "project.new", json!({"create_board":true})).unwrap();
    run(&mut s, crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2})).unwrap();
    let layer = board::track::tests::copper_layers(&s)[0];
    let mut fixture = s.project().unwrap().clone();
    let tracks = &mut Arc::make_mut(&mut fixture.board).tracks;
    tracks.extend((0..41_000).map(|i| {
        let x_nm = 1_000_000 + (i % 205) * 40_000;
        let y_nm = 1_000_000 + (i / 205) * 40_000;
        Track {
            id: Uuid::from_u128(1 + i as u128),
            layer,
            locked: false,
            width_nm: 10_000,
            start: Point { x_nm, y_nm },
            end: Point { x_nm: x_nm + 15_000, y_nm },
        }
    }));
    let saved = encode_documents(Arc::new(fixture)).unwrap();
    s.open_snapshot(decode_documents(&saved).unwrap());
    let args = |n| json!({"items":{"tracks":sliced_track_ids(n)}});
    let cap = 8 * 1024 * 1024;
    let (fit, overflow) = (args(39_940), args(39_960));
    let original = s.snapshot().unwrap();
    let text = |params: &Value| {
        let selection = clipboard::BoardSelection::parse(&params["items"]).unwrap();
        clipboard::BoardFragment::capture(&original, &selection, true).unwrap().to_text().unwrap()
    };
    let expected = text(&fit);
    let too_large = text(&overflow);
    assert!(too_large.len() < cap && serde_json::to_vec(&too_large).unwrap().len() > cap);
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let live = reviewed(&mut s, clipboard::CUT, fit);
    let retained = s.checked_generic_candidate().unwrap().clone();
    let error = unchanged(&mut s, |s| run(s, PREVIEW, json!({"intent":{"command":clipboard::CUT,"params":overflow}})));
    assert!(matches!(error, CommandError::CapExceeded(ref msg) if msg.contains("complete relation command result")), "{error}");
    assert!(Arc::ptr_eq(s.checked_generic_candidate().unwrap(), &retained));
    let mut apply = decision(&live);
    apply["expected_revision"] = json!(s.revision() - 1);
    let error = unchanged(&mut s, |s| s.apply(APPLY, &apply).map(|v| json!(v)));
    assert!(matches!(error, CommandError::StaleRevision { .. }));
    assert!(observer.0.lock().unwrap().is_empty());
    let response = run(&mut s, APPLY, decision(&live)).unwrap();
    assert_eq!(response["clipboard"], expected);
    assert!(response.get("clipboard_notice").is_none());
    let envelope = serde_json::to_vec(&response).unwrap().len();
    assert!((cap - 10_000..=cap).contains(&envelope), "{envelope}");
    assert!(Arc::ptr_eq(&retained, &s.snapshot().unwrap()));
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    unchanged(&mut s, |s| run(s, APPLY, decision(&live)));
}

fn sliced_track_ids(n: u128) -> Vec<Uuid> {
    (1..=n).map(Uuid::from_u128).collect()
}

/// This exact bundle is the source witness for the separate #443 reader. Here we only
/// qualify the current reader on the lowest supported board format (board/1 with an optional
/// relations record). Saving adopts the current project and board formats; the board keeps its
/// relation, and re-encoding is byte-stable.
#[test]
fn a_format_intact_f1_fixture_for_baseline_reader_qualification() {
    const ROOT: &str = "sheets/00000000-0000-4000-8000-000000000003.json";
    let docs = Documents::from([
        ("project.json".into(), include_bytes!("../../../tests/fixtures/f1-intact/project.json").to_vec()),
        ("board.json".into(), include_bytes!("../../../tests/fixtures/f1-intact/board.json").to_vec()),
        (ROOT.into(), include_bytes!("../../../tests/fixtures/f1-intact/sheets/00000000-0000-4000-8000-000000000003.json").to_vec()),
    ]);
    let mut s = Session::default();
    s.open_snapshot(decode_documents(&docs).unwrap());
    let saved = bytes(&s);
    assert_eq!(
        serde_json::from_slice::<Value>(&saved[crate::persistence::ENTRY_POINT]).unwrap()["format"],
        crate::persistence::CURRENT_PROJECT_FORMAT
    );
    let saved_board: Value = serde_json::from_slice(&saved[&test_board_path(&saved)]).unwrap();
    let fixture_board: Value = serde_json::from_slice(&docs["board.json"]).unwrap();
    assert_eq!(fixture_board["format"], "pcbcraft-board/1");
    assert_eq!(saved_board["format"], crate::persistence::CURRENT_BOARD_FORMAT);
    assert_eq!(saved_board["relations"], fixture_board["relations"]);
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    let list = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(list["relations"][0]["status"], "lost");
    assert_eq!(list["relations"][0]["last_resolved"][1]["at"]["x_nm"], 1_000_000);
    assert!(list["relations"][0].get("measure").is_none());
}

#[test]
fn a_format_native_id_bound_in_commands_and_lost_readback() {
    let mut s = fixture(false);
    let reference = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), reference.clone());
    let mut invalid = reference.clone();
    invalid["fp"] = json!(crate::MAX_OBJECT_ID + 1);
    let live = reviewed(&mut s, board::clipboard::DELETE, json!({"items":{"footprints":[format!("footprint-{}",reference["fp"])]}}));
    unchanged(&mut s, |s| run(s, ADD, json!({"relation":{"kind":"horizontal","refs":[origin(),invalid]}})));
    unchanged(&mut s, |s| run(s, EDIT, json!({"id":id,"edit":{"repick":{"kind":"horizontal","refs":[origin(),invalid]}}})));
    unchanged(&mut s, |s| {
        run(
            s,
            PREVIEW,
            json!({"intent":{"command":"pcb.delete_items","params":{"items":{"footprints":[format!("footprint-{}",reference["fp"])]}}},"remap_choices":[{"relation":id,"pick":1,"choice":{"kind":"repick","reference":invalid}}]}),
        )
    });
    run(&mut s, APPLY, decision(&live)).unwrap();
    let original = bytes(&s);
    let path = test_board_path(&original);
    let mut board: Value = serde_json::from_slice(&original[&path]).unwrap();
    let altered = |board: &Value| {
        let mut docs = original.clone();
        docs.insert(path.clone(), serde_json::to_vec(board).unwrap());
        docs
    };
    board["relations"]["relations"][0]["refs"][1] = invalid;
    assert!(decode_documents(&altered(&board)).is_err());
    board["relations"]["relations"][0]["refs"][1]["fp"] = json!(crate::MAX_OBJECT_ID);
    s.open_snapshot(decode_documents(&altered(&board)).unwrap());
    let valid = bytes(&s);
    s.open_snapshot(decode_documents(&valid).unwrap());
    assert_eq!(bytes(&s), valid);
    let rows = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(rows["relations"][0]["status"], "lost");
    assert_eq!(rows["relations"][0]["refs"][1]["fp"], crate::MAX_OBJECT_ID);
    assert_eq!(rows["relations"][0]["last_resolved"], live["disclosure"]["losses"][0]["last_resolved"]);
}

/// First reference authoring on a native board runs final preparation exactly once, never moves it.
#[test]
fn a_preview_edge_first_create_and_reference_only_dispatch() {
    let mut s = fixture(false);
    let prior = s.snapshot().unwrap();
    assert!(prior.board.relations.is_none());
    let reference = pick(&mut s, 0, "fp.origin", None);
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let draft = json!({"kind":"horizontal","refs":[origin(),reference]});
    for field in ["driving", "last_resolved", "value"] {
        let mut bad = draft.clone();
        bad[field] = json!(true);
        unchanged(&mut s, |s| run(s, ADD, json!({"relation":bad})));
    }
    let (id, ops) = trace(|| add(&mut s, "horizontal", origin(), reference));
    assert_eq!(
        ops,
        [Op::Prepare(RelationStage::Outer), Op::VerifyReferences, Op::CandidateEncode, Op::CandidateDigest, Op::Publish(RelationStage::Outer)]
    );
    let rows = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(rows["relations"][0]["id"], id);
    assert_eq!(rows["relations"][0]["measure"]["signed_nm"], 3_000_000);
    assert_eq!(rows["relations"][0]["axes_set"], json!([]));
    assert_eq!(s.project().unwrap().board.footprints(), prior.board.footprints());
    let native = json!({"footprints":[format!("footprint-{}",prior.board.footprints()[0].id)],"accept_loss":[id]});
    unchanged(&mut s, |s| run(s, crate::DELETE_FOOTPRINTS, native));
    assert_eq!(observer.0.lock().unwrap().len(), 1);
}

/// The base Arc is part of authority even if save adoption changes neither revision nor board ID.
#[test]
fn a_preview_bind_metadata_arc_and_replacement() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), a);
    let fp = s.project().unwrap().board.footprints()[0].id;
    let args = json!({"footprints":[format!("footprint-{fp}")]});
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, args.clone());
    let snapshot = s.snapshot().unwrap();
    let rev = s.revision();
    assert!(s.adopt_saved_metadata(&snapshot, &Arc::new(snapshot.renamed("After Save").unwrap())));
    assert_eq!(s.revision(), rev);
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, args.clone());
    let other = s.project().unwrap().board.footprints()[1].id;
    let target = s.project().unwrap().board_target().unwrap().board;
    let replacement =
        reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"board":target,"footprints":[format!("footprint-{fp}"),format!("footprint-{other}")]}));
    assert_ne!(p["intent_digest"], replacement["intent_digest"]);
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
    unchanged(&mut s, |s| {
        run(
            s,
            PREVIEW,
            json!({"intent":{"command":crate::DELETE_FOOTPRINTS,"params":{"board":Uuid::from_u128(7),"footprints":[format!("footprint-{fp}")]}}}),
        )
    });
    let mut extra = decision(&replacement);
    extra["intent"] = json!({});
    unchanged(&mut s, |s| run(s, APPLY, extra));
    run(&mut s, APPLY, decision(&replacement)).unwrap();
    unchanged(&mut s, |s| run(s, APPLY, decision(&replacement)));
    let still_present = s.project().unwrap().board.footprints()[0].id;
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{still_present}")]}));
    let saved = bytes(&s);
    s.open_snapshot(decode_documents(&saved).unwrap());
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{still_present}")]}));
    run(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":40_000_000,"y_nm":0})).unwrap();
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
}

/// Native ECO stays lazy without authored rows and returns the exact old command envelope.
/// On the checked route, retained/suppressed rows still require candidate certification.
#[test]
fn a_eco_compat_lazy_checked_and_target() {
    for related in [false, true] {
        let mut s = fixture(true);
        if related {
            let a = pick(&mut s, 0, "fp.origin", None);
            let id = add(&mut s, "horizontal", origin(), a);
            run(&mut s, EDIT, json!({"id":id,"edit":{"suppressed":true}})).unwrap();
        }
        let sy = s.project().unwrap().symbols.last().unwrap().id();
        run(&mut s, crate::DELETE_SYMBOL, json!({"symbol":sy})).unwrap();
        let before = s.snapshot().unwrap();
        let target = before.board_target().unwrap();
        let report = run(&mut s, update::PREVIEW_UPDATE, json!({"board":target.board})).unwrap()["board_update_preview"].clone();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(report.get("candidate_digest").is_some(), related);
        unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"board":Uuid::from_u128(42),"plan_token":report["plan_token"]})));
        unchanged(&mut s, |s| run(s, CANCEL, json!({"token":report["plan_token"]})));
        run(&mut s, update::CANCEL_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
        let report = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
        run(&mut s, update::APPLY_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
        if related {
            assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), report["candidate_digest"].as_str().unwrap());
        }
        let noop = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
        let unchanged = s.snapshot().unwrap();
        run(&mut s, update::APPLY_UPDATE, json!({"plan_token":noop["plan_token"],"accept_loss":[],"confirm_remaps":[]})).unwrap();
        assert!(Arc::ptr_eq(&unchanged, &s.snapshot().unwrap()));
    }
}

/// A single checked ECO can disclose two independent decisions: a retired placement and
/// a still-offered reference moved to a different exact definition revision.
#[test]
fn a_preview_first_eco_requires_both_decisions() {
    let mut s = fixture(true);
    let kept = pick(&mut s, 0, "fp.origin", None);
    let retired = pick(&mut s, 2, "fp.origin", None);
    add(&mut s, "horizontal", origin(), kept);
    add(&mut s, "horizontal", origin(), retired);
    let fp = &s.project().unwrap().board.footprints()[0];
    let component = fp.component;
    let library = board::definition::basic_components("resistor-0603").unwrap();
    run(
        &mut s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{component}"),"library":library.library,
        "definition":library.id,"definition_revision":library.revision,"mapping":{"1":"1","2":"2"}}),
    )
    .unwrap();
    let symbol = s.project().unwrap().symbols.last().unwrap().id();
    run(&mut s, crate::DELETE_SYMBOL, json!({"symbol":symbol})).unwrap();
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let before = s.snapshot().unwrap();
    let (r, ops) = trace(|| run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone());
    assert_eq!(ops, super::eco_tests::CHECKED);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let d = &r["relation_detail"];
    assert_eq!(d["poses"].as_array().unwrap().len(), 1);
    assert_eq!(d["losses"].as_array().unwrap().len(), 1);
    assert_eq!(d["remaps"].as_array().unwrap().len(), 1);
    let loss = d["losses"][0]["id"].clone();
    let remap = d["remaps"][0]["id"].clone();
    assert!(loss.is_string() && remap.is_string());
    let token = &r["plan_token"];
    let wrong = Uuid::from_u128(74);
    let exact = json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[remap]});
    for args in [
        json!({"plan_token":token}),
        json!({"plan_token":token,"accept_loss":[loss]}),
        json!({"plan_token":token,"confirm_remaps":[remap]}),
        json!({"plan_token":token,"accept_loss":[loss,loss],"confirm_remaps":[remap]}),
        json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[remap,remap]}),
        json!({"plan_token":token,"accept_loss":[wrong],"confirm_remaps":[remap]}),
        json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[wrong]}),
        json!({"plan_token":token,"accept_loss":[loss,wrong],"confirm_remaps":[remap]}),
        json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[remap,wrong]}),
        json!({"plan_token":token,"accept_loss":[remap],"confirm_remaps":[loss]}),
        json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[remap],"remap_choices":[]}),
    ] {
        let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, args)));
        assert!(ops.is_empty());
        assert_eq!(s.board_update_report().unwrap(), r);
    }
    assert!(observer.0.lock().unwrap().is_empty());
    let retained = s.checked_eco_candidate().unwrap().clone();
    let docs = encode_documents(retained.clone()).unwrap();
    let history = s.undo.len();
    let (_, ops) = trace(|| run(&mut s, update::APPLY_UPDATE, exact.clone()).unwrap());
    assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
    assert!(Arc::ptr_eq(&retained, &s.snapshot().unwrap()));
    assert_eq!(bytes(&s), docs);
    assert_eq!(s.undo.len(), history + 1);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, exact));
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), r["candidate_digest"].as_str().unwrap());
}

/// Reference rows compose with native arc-bearing reads/saves, but do not broaden edit eligibility.
#[test]
fn a_format_arc_read_save_and_edit_refusal() {
    let mut s = board::track::tests::routed_fixture();
    let a = pick(&mut s, 0, "fp.origin", None);
    let row = add(&mut s, "horizontal", origin(), a);
    let layer = board::track::tests::copper_layers(&s)[0];
    let target = s.project().unwrap().board_target().unwrap();
    run(
        &mut s,
        board::arc::SET_ARCS,
        json!({"board":target.board,"arcs":[{"id":Uuid::from_u128(22),"layer":layer,"width_nm":250_000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5_000_000,"y_nm":0},"end":{"x_nm":0,"y_nm":5_000_000},"direction":"clockwise"}]}),
    )
    .unwrap();
    let saved = bytes(&s);
    s.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(bytes(&s), saved);
    let err = unchanged(&mut s, |s| run(s, EDIT, json!({"id":row,"edit":{"suppressed":true}})));
    assert_eq!(err.details()["reason"], "arc_board");
    assert_eq!(err.to_string(), board::arc::UNSUPPORTED);
    assert_eq!(bytes(&s), saved);
}

/// Real native alignment and quarter turns remeasure only; a non-quarter turn discloses loss.
#[test]
fn a_preview_edge_align_and_reference_axis_turn_without_follow() {
    let mut s = fixture(false);
    let ids: Vec<_> = s.project().unwrap().board.footprints().iter().map(|f| format!("footprint-{}", f.id)).collect();
    let transform =
        |i: usize, y_nm: i64, rotation: u16| json!({"footprint":ids[i],"x_nm":(3 + i as i64 * 10) * 1_000_000,"y_nm":y_nm,"rotation":rotation});
    run(&mut s, crate::SET_FOOTPRINT_TRANSFORM, transform(0, 4_000_000, 0)).unwrap();
    let edge = pick(&mut s, 0, "pad.edge", Some("1"));
    let point = pick(&mut s, 2, "fp.origin", None);
    add(&mut s, "point_edge", edge, point);
    let p0 = s.project().unwrap().board.footprints()[2].clone();
    let p = reviewed(&mut s, crate::ALIGN_FOOTPRINTS, json!({"footprints":[ids[0],ids[1]],"mode":"center_y"}));
    assert_eq!(p["disclosure"]["losses"], json!([]));
    run(&mut s, APPLY, decision(&p)).unwrap();
    let y = s.project().unwrap().board.footprints()[0].y_nm;
    let before = run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"].clone();
    assert_eq!(before["axis"], "x");
    let p = reviewed(&mut s, crate::SET_FOOTPRINT_TRANSFORM, transform(0, y, 90));
    assert_eq!(p["disclosure"]["losses"], json!([]));
    run(&mut s, APPLY, decision(&p)).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"]["axis"], "y");
    let p = reviewed(&mut s, crate::SET_FOOTPRINT_TRANSFORM, transform(0, y, 37));
    run(&mut s, APPLY, decision(&p)).unwrap();
    run(&mut s, crate::SET_FOOTPRINT_TRANSFORM, transform(0, y, 0)).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
    assert_eq!(s.project().unwrap().board.footprints()[2], p0);
}

/// Re-pick against the actual quarter-turned candidate: availability of each old address
/// is insufficient when the ordered gap is no longer parallel. Failed re-reviews retain authority.
#[test]
fn a_preview_edge_quarter_turn_repick_repairs_ordered_gap() {
    let mut s = fixture(false);
    let edge = |s: &mut Session, fp, side| {
        let features = run(s, FEATURES, json!({"fp":format!("footprint-{fp}")})).unwrap();
        features["features"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|item| {
                let r = &item["reference"];
                (r["tag"] == "pad.edge" && r["pad"] == "1" && r["side"] == side).then(|| r.clone())
            })
            .unwrap()
    };
    let original = s.project().unwrap().board.footprints().to_vec();
    let a = edge(&mut s, original[0].id, "left");
    let b = edge(&mut s, original[1].id, "left");
    let replacement = edge(&mut s, original[1].id, "top");
    let incompatible = edge(&mut s, original[1].id, "right");
    let typed = |r: &Value| serde_json::from_value::<Reference>(r.clone()).unwrap();
    assert!(catalog::measure(&catalog::Resolver::new(s.project().unwrap()), Kind::Gap, &[typed(&a), typed(&replacement)]).is_none());
    let row = add(&mut s, "gap", a.clone(), b.clone());
    let base = s.snapshot().unwrap();
    assert!(base.board.tracks.is_empty() && base.board.vias.is_empty());
    let base_bytes = bytes(&s);
    let (revision, undo, redo) = (s.revision(), s.undo.len(), s.redo.len());
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let moved = &original[1];
    let native = json!({"footprint":format!("footprint-{}",moved.id),"x_nm":moved.x_nm,"y_nm":moved.y_nm,"rotation":90});
    let loss = reviewed(&mut s, crate::SET_FOOTPRINT_TRANSFORM, native.clone());
    let lost = s.checked_generic_candidate().unwrap().clone();
    let history = loss["disclosure"]["losses"][0]["last_resolved"].clone();
    assert_eq!(loss["disclosure"]["losses"].as_array().unwrap().len(), 1);
    assert_eq!(loss["disclosure"]["losses"][0]["id"], row);
    assert_eq!(loss["disclosure"]["remaps"], json!([]));
    let loss_resolver = catalog::Resolver::new(&lost);
    assert_eq!(catalog::correspondence(&base, &loss_resolver, &typed(&b)), Some(typed(&b)));
    assert!(catalog::resolve(&loss_resolver, &typed(&a)).is_some());
    assert!(catalog::measure(&loss_resolver, Kind::Gap, &[typed(&a), typed(&b)]).is_none());
    let repreview = |s: &mut Session, id: &Value, pick: &Value| {
        run(
            s,
            PREVIEW,
            json!({
                "intent":{"command":crate::SET_FOOTPRINT_TRANSFORM,"params":native},
                "remap_choices":[{"relation":id,"pick":1,"choice":{"kind":"repick","reference":pick}}]
            }),
        )
    };
    let mut absent = replacement.clone();
    absent["pad"] = json!("99");
    for repick in [&b, &absent, &incompatible] {
        assert_eq!(unchanged(&mut s, |s| repreview(s, &row, repick)).details()["reason"], "lost_reference");
        assert!(Arc::ptr_eq(&lost, s.checked_generic_candidate().unwrap()));
    }
    assert!(matches!(unchanged(&mut s, |s| repreview(s, &json!(Uuid::from_u128(111)), &replacement)), CommandError::InvalidParams(_)));
    assert!(Arc::ptr_eq(&lost, s.checked_generic_candidate().unwrap()));
    let fixed = repreview(&mut s, &row, &replacement).unwrap()["relation_preview"].clone();
    let candidate = s.checked_generic_candidate().unwrap().clone();
    assert_eq!(fixed["disclosure"]["losses"], json!([]));
    assert_eq!(fixed["disclosure"]["remaps"].as_array().unwrap().len(), 1);
    let remap = &fixed["disclosure"]["remaps"][0];
    assert_eq!(remap["relation"], row);
    assert_eq!(remap["pick"], 1);
    assert_eq!(remap["old"], b);
    assert_eq!(remap["new"], replacement);
    assert_eq!(remap["before"], history[1]);
    let pair = [typed(&a), typed(&replacement)];
    let measured = catalog::measure(&catalog::Resolver::new(&candidate), Kind::Gap, &pair).unwrap();
    assert!(matches!(measured, Measure::Projection { axis: catalog::Axis::X, signed_nm, .. } if signed_nm > 0));
    assert_eq!(remap["after"], json!(measured.witnesses()[1]));
    assert_eq!(lost.board.footprints(), candidate.board.footprints());
    let mut expected_poses = original;
    expected_poses[1].rotation = 90;
    assert_eq!(candidate.board.footprints(), expected_poses.as_slice());
    let expected_disclosed: Vec<_> =
        expected_poses[..2].iter().map(|f| json!({"fp":f.id,"x_nm":f.x_nm,"y_nm":f.y_nm,"rotation":f.rotation,"bottom":f.bottom})).collect();
    assert_eq!(fixed["disclosure"]["poses"], json!(expected_disclosed));
    assert_eq!(candidate.board.tracks, base.board.tracks);
    assert_eq!(candidate.board.vias, base.board.vias);
    let mut wrong = decision(&fixed);
    wrong["confirm_remaps"] = json!([row]);
    let mut prior_loss = decision(&fixed);
    prior_loss["accept_loss"] = json!([row]);
    let mut missing = decision(&fixed);
    missing["confirm_remaps"] = json!([]);
    for denied in [decision(&loss), wrong, prior_loss, missing] {
        unchanged(&mut s, |s| run(s, APPLY, denied));
        assert!(Arc::ptr_eq(&candidate, s.checked_generic_candidate().unwrap()));
    }
    assert!(Arc::ptr_eq(&base, &s.snapshot().unwrap()));
    assert_eq!(bytes(&s), base_bytes);
    assert!(observer.0.lock().unwrap().is_empty());
    let expected = encode_documents(candidate.clone()).unwrap();
    let (reply, ops) = trace(|| run(&mut s, APPLY, decision(&fixed)).unwrap());
    assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
    assert_eq!(reply["candidate_digest"], fixed["candidate_digest"]);
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &candidate));
    assert_eq!(bytes(&s), expected);
    assert_eq!((s.revision(), s.undo.len(), s.redo.len()), (revision + 1, undo + 1, redo));
    let saved = &run(&mut s, LIST, json!({})).unwrap()["relations"][0];
    assert_eq!(saved["refs"], json!(pair));
    assert_eq!(saved["measure"], json!(measured));
    assert_eq!(saved["status"], "reference");
    unchanged(&mut s, |s| run(s, APPLY, decision(&fixed)));
    let events = observer.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(Arc::ptr_eq(&events[0].project, &candidate));
}

/// A native transform may remeasure, but an explicit replacement pick must author the
/// exact ordered final pair. Invalid authoring must not revoke the previous review.
#[test]
fn a_preview_edge_repick_authors_final_projection_and_align_predicate() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    let b = pick(&mut s, 1, "fp.origin", None);
    let pad = pick(&mut s, 1, "pad.center", Some("1"));
    let horizontal = add(&mut s, "horizontal", a.clone(), b.clone());
    let align = add(&mut s, "align_h", a.clone(), b);
    let base = s.snapshot().unwrap();
    assert!(base.board.tracks.is_empty() && base.board.vias.is_empty());
    let base_bytes = bytes(&s);
    let f = base.board.footprints();
    let Witness::Point { at } = catalog::resolve(&catalog::Resolver::new(&base), &serde_json::from_value(pad.clone()).unwrap()).unwrap() else {
        panic!("pad centre must be a point")
    };
    let pad_dx = at.x_nm - f[1].x_nm;
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let native =
        |signed_nm: i64| json!({"footprint":format!("footprint-{}",f[1].id),"x_nm":f[0].x_nm - pad_dx + signed_nm,"y_nm":8_000_000,"rotation":0});
    let repick = |s: &mut Session, id: &Value, native: &Value| {
        run(
            s,
            PREVIEW,
            json!({"intent":{"command":crate::SET_FOOTPRINT_TRANSFORM,"params":native},
            "remap_choices":[{"relation":id,"pick":1,"choice":{"kind":"repick","reference":pad}}]}),
        )
    };
    for (id, kind, x, message) in [(&horizontal, "horizontal", 0, "Use Align for zero"), (&align, "align_h", -2_000_000, "Align requires a zero")] {
        let native = native(x);
        let prior = reviewed(&mut s, crate::SET_FOOTPRINT_TRANSFORM, native.clone());
        assert_eq!(prior["disclosure"]["losses"], json!([]));
        assert_eq!(prior["disclosure"]["remaps"], json!([]));
        let candidate = s.checked_generic_candidate().unwrap().clone();
        let mut lookup = Session::default();
        lookup.open_snapshot(candidate.clone());
        let draft = json!({"kind":kind,"refs":[a,pad]});
        let add_error = unchanged(&mut lookup, |s| run(s, ADD, json!({"relation":draft})));
        let edit_error = unchanged(&mut lookup, |s| run(s, EDIT, json!({"id":id,"edit":{"repick":draft}})));
        let error = unchanged(&mut s, |s| repick(s, id, &native));
        assert!(matches!(error, CommandError::InvalidParams(_)));
        assert!(error.to_string().contains(message));
        assert_eq!(error.to_string(), add_error.to_string());
        assert_eq!(error.to_string(), edit_error.to_string());
        assert!(Arc::ptr_eq(&candidate, s.checked_generic_candidate().unwrap()));
        let mut prior_session = Session::default();
        prior_session.open_snapshot(base.clone());
        // The ordinary native transform is allowed even when that row now measures differently.
        let control = reviewed(&mut prior_session, crate::SET_FOOTPRINT_TRANSFORM, native);
        run(&mut prior_session, APPLY, decision(&control)).unwrap();
        let plain = bytes(&prior_session);
        let board: Value = serde_json::from_slice(&plain[&test_board_path(&plain)]).unwrap();
        assert_eq!(board["relations"]["relations"][0]["value"], 10_000_000);
        assert_eq!(board["relations"]["relations"][0]["side"], 1);
        run(&mut s, CANCEL, json!({"token":prior["token"]})).unwrap();
    }
    let native = native(-2_000_000);
    let reported = repick(&mut s, &horizontal, &native).unwrap()["relation_preview"].clone();
    let candidate = s.checked_generic_candidate().unwrap().clone();
    assert_eq!(reported["disclosure"]["losses"], json!([]));
    assert_eq!(reported["disclosure"]["remaps"].as_array().unwrap().len(), 1);
    assert_eq!(reported["disclosure"]["remaps"][0]["new"], pad);
    for (key, value) in [("confirm_remaps", json!([])), ("accept_loss", json!([horizontal]))] {
        let mut wrong = decision(&reported);
        wrong[key] = value;
        unchanged(&mut s, |s| run(s, APPLY, wrong));
        assert!(Arc::ptr_eq(&candidate, s.checked_generic_candidate().unwrap()));
    }
    assert_eq!(bytes(&s), base_bytes);
    assert!(observer.0.lock().unwrap().is_empty());
    let expected = encode_documents(candidate.clone()).unwrap();
    let (_, ops) = trace(|| run(&mut s, APPLY, decision(&reported)).unwrap());
    assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
    assert!(Arc::ptr_eq(&candidate, &s.snapshot().unwrap()));
    assert_eq!(bytes(&s), expected);
    let disk: Value = serde_json::from_slice(&expected[&test_board_path(&expected)]).unwrap();
    let row = &disk["relations"]["relations"][0];
    assert_eq!(row["refs"], json!([a, pad]));
    assert_eq!(row["value"], 2_000_000);
    assert_eq!(row["side"], -1);
    let reopened = decode_documents(&expected).unwrap();
    let mut native_poses = f.to_vec();
    native_poses[1].x_nm = native["x_nm"].as_i64().unwrap();
    native_poses[1].y_nm = native["y_nm"].as_i64().unwrap();
    assert_eq!(reopened.board.footprints(), native_poses);
    assert_eq!(reopened.board.tracks, base.board.tracks);
    assert_eq!(reopened.board.vias, base.board.vias);
    assert_eq!(encode_documents(reopened).unwrap(), expected);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    unchanged(&mut s, |s| run(s, APPLY, decision(&reported)));
}

/// Both engine checkers (and the query) see one typed advisory from the captured snapshot.
/// Checking is read-only even with a latched loss, and a later live edit cannot alter a job's rows.
#[test]
fn a_drc_relation_advisory_matches_sync_resumable_and_captured_history() {
    let mut s = fixture(false);
    let first = pick(&mut s, 0, "fp.origin", None);
    let second = pick(&mut s, 1, "fp.origin", None);
    let edge = pick(&mut s, 2, "pad.edge", Some("1"));
    let live = add(&mut s, "aligned", origin(), first.clone());
    let suppressed = add(&mut s, "horizontal", origin(), second.clone());
    run(&mut s, EDIT, json!({"id":suppressed,"edit":{"suppressed":true}})).unwrap();
    let lost = add(&mut s, "point_edge", first, edge);
    let f = &s.project().unwrap().board.footprints()[2];
    let transform = json!({"footprint":format!("footprint-{}",f.id),"x_nm":f.x_nm,"y_nm":f.y_nm,"rotation":37});
    let review = reviewed(&mut s, crate::SET_FOOTPRINT_TRANSFORM, transform);
    let witness = &review["disclosure"]["losses"][0]["last_resolved"];
    run(&mut s, APPLY, decision(&review)).unwrap();
    let snapshot = s.snapshot().unwrap();
    let saved = bytes(&s);
    let (revision, undo, redo) = (s.revision(), s.undo.len(), s.redo.len());
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let rows = run(&mut s, LIST, json!({})).unwrap()["relations"].clone();
    let row = |id: &Value| rows.as_array().unwrap().iter().find(|r| r["id"] == *id).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 3);
    assert_eq!(row(&live)["status"], "reference");
    assert_eq!(row(&live)["measure"]["kind"], "aligned");
    assert_eq!(row(&live)["measure"]["distance_nm"], json!(3_000_000_f64.hypot(2_000_000.)));
    assert_eq!(row(&suppressed)["status"], "suppressed");
    assert_eq!(row(&suppressed)["suppressed"], true);
    assert_eq!(row(&suppressed)["measure"]["signed_nm"], 13_000_000);
    assert_eq!(row(&lost)["status"], "lost");
    assert_eq!(row(&lost)["reason"], "lost_reference");
    assert_eq!(row(&lost)["last_resolved"], *witness);
    assert_eq!(row(&lost)["since_revision"], revision.max(snapshot.highest_id()));
    assert!(row(&lost).get("measure").is_none());
    let params = json!({"clearance_nm":300_000});
    let envelope = run(&mut s, crate::CHECK_DESIGN, params.clone()).unwrap();
    assert_eq!(envelope["revision"], revision);
    let sync = &envelope["design_check"];
    assert_eq!(sync["relations"], rows);
    let mut captured = s.design_check_job(&params).unwrap();
    for budget in [1, 17, 4096] {
        let mut job = s.design_check_job(&params).unwrap();
        assert!(Arc::ptr_eq(&job.source().snapshot, &snapshot));
        let mut steps = 0;
        while !job.step(budget) {
            assert!(job.take_report().is_none());
            steps += 1;
            assert!(steps < 10_000);
        }
        assert_eq!(serde_json::to_vec(&json!(job.take_report().unwrap())).unwrap(), serde_json::to_vec(sync).unwrap());
        assert!(job.take_report().is_none());
        assert!(job.source().is_current(&s));
    }
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!((s.revision(), s.undo.len(), s.redo.len()), (revision, undo, redo));
    assert_eq!(bytes(&s), saved);
    assert!(observer.0.lock().unwrap().is_empty());
    let packed = crate::persistence::encode_packed(snapshot.clone()).unwrap();
    assert_eq!(encode_documents(crate::persistence::decode_packed(&packed).unwrap().project).unwrap(), saved);

    run(&mut s, EDIT, json!({"id":suppressed,"edit":{"suppressed":false}})).unwrap();
    assert!(!captured.source().is_current(&s));
    assert_ne!(run(&mut s, crate::CHECK_DESIGN, params).unwrap()["design_check"]["relations"], rows);
    let mut steps = 0;
    while !captured.step(4096) {
        steps += 1;
        assert!(steps < 10_000);
    }
    assert_eq!(&json!(captured.take_report().unwrap()), sync);
}

/// All live rows share the native projection of their exact source; outline removal must
/// cache unavailability and latch display witnesses without using them as geometry.
#[test]
fn a_native_outline_datum_projects_once_per_exact_snapshot() {
    use super::probes::trace_native_datum;

    let mut s = fixture(false);
    let check = json!({"clearance_nm":300_000});
    let projections = |ops: &[Op]| ops.iter().filter(|op| **op == Op::NativeDatum).count();
    let (_, ops) = trace_native_datum(|| run(&mut s, crate::CHECK_DESIGN, check.clone()).unwrap());
    assert_eq!(projections(&ops), 0);
    board::drc::tests::outline(&mut s);
    let board = s.project().unwrap().board_target().unwrap().board;
    run(&mut s, crate::SET_PLACEMENT_ORIGIN, json!({"board":board,"origin":{"kind":"outline_lower_left"}})).unwrap();
    for i in 0..3 {
        let r = pick(&mut s, i, "fp.origin", None);
        add(&mut s, "horizontal", origin(), r);
    }
    let snapshot = s.snapshot().unwrap();
    let saved = bytes(&s);
    let poses = snapshot.board.footprints().to_vec();
    let drawing = snapshot.board.drawings.as_ref().unwrap().items[0].id;
    let check_sync = |s: &mut Session| trace_native_datum(|| run(s, crate::CHECK_DESIGN, check.clone()).unwrap()["design_check"].clone());
    let (sync, ops) = check_sync(&mut s);
    assert_eq!(projections(&ops), 1);
    let (listed, ops) = trace_native_datum(|| run(&mut s, LIST, json!({})).unwrap());
    assert_eq!(projections(&ops), 1);
    assert_eq!(sync["relations"], listed["relations"]);
    let rows = sync["relations"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let before_x = rows[0]["measure"]["witnesses"][0]["at"]["x_nm"].as_i64().unwrap();
    let finish = |mut job: board::drc::CheckJob| {
        trace_native_datum(|| {
            for _ in 0..10_000 {
                if job.step(1) {
                    return json!(job.take_report().unwrap());
                }
                assert!(job.take_report().is_none());
            }
            panic!("bounded small fixture did not complete");
        })
    };
    let captured = s.design_check_job(&check).unwrap();
    assert!(Arc::ptr_eq(&captured.source().snapshot, &snapshot));
    assert_eq!(bytes(&s), saved);
    let (_, ops) = trace_native_datum(|| {
        run(&mut s, board::movement::MOVE_ITEMS, json!({"items":{"drawings":[drawing]},"dx_nm":1_000_000,"dy_nm":0})).unwrap();
    });
    // All candidate witnesses came from the moved outline, not the captured or saved one.
    assert_eq!(projections(&ops), 1);
    let (moved, ops) = check_sync(&mut s);
    assert_eq!(projections(&ops), 1);
    let moved_rows = moved["relations"].as_array().unwrap();
    for (row, prior) in moved_rows.iter().zip(rows) {
        assert_eq!(row["measure"]["witnesses"][0]["at"]["x_nm"], before_x + 1_000_000);
        assert_eq!(row["measure"]["witnesses"][1], prior["measure"]["witnesses"][1]);
    }
    assert!(!captured.source().is_current(&s));
    let (report, ops) = finish(captured);
    assert_eq!(projections(&ops), 1);
    assert_eq!(serde_json::to_vec(&report).unwrap(), serde_json::to_vec(&sync).unwrap());

    let (loss, ops) = trace_native_datum(|| reviewed(&mut s, board::clipboard::DELETE, json!({"items":{"drawings":[drawing]}})));
    assert_eq!(projections(&ops), 2, "one unavailable F′ projection and one live P projection");
    let losses = loss["disclosure"]["losses"].as_array().unwrap();
    assert_eq!(losses.len(), 3);
    for (loss, row) in losses.iter().zip(moved_rows) {
        assert_eq!(loss["id"], row["id"]);
        assert_eq!(loss["last_resolved"], row["measure"]["witnesses"]);
    }
    let (_, ops) = trace_native_datum(|| run(&mut s, APPLY, decision(&loss)).unwrap());
    assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
    let (latched, ops) = check_sync(&mut s);
    assert_eq!(projections(&ops), 0, "latched display needs no projection");
    for row in latched["relations"].as_array().unwrap() {
        assert_eq!(row["status"], "lost");
        assert!(row.get("measure").is_none());
    }
    assert_eq!(s.project().unwrap().board.footprints(), poses);
    let stored = bytes(&s);
    assert_eq!(encode_documents(decode_documents(&stored).unwrap()).unwrap(), stored);

    // A saved live row can be unresolved on inspection. Inject only the fixture's original
    // live records into the now outline-free board to test this distinct read-only case.
    let mut absent = s.project().unwrap().clone();
    Arc::make_mut(&mut absent.board).relations = snapshot.board.relations.clone();
    s.open_snapshot(Arc::new(absent));
    let absent_bytes = bytes(&s);
    let (sync, ops) = check_sync(&mut s);
    assert_eq!(projections(&ops), 1);
    let (job, ops) = finish(s.design_check_job(&check).unwrap());
    assert_eq!(projections(&ops), 1);
    assert_eq!(serde_json::to_vec(&job).unwrap(), serde_json::to_vec(&sync).unwrap());
    for row in sync["relations"].as_array().unwrap() {
        assert_eq!(row["reason"], "unresolved_reference");
        assert!(row.get("last_resolved").is_none() && row.get("measure").is_none());
    }
    assert_eq!(bytes(&s), absent_bytes);
}

/// Disclose native participants, not merely the relation bodies. Alignment includes the
/// unchanged selected anchor but not the unchanged unselected footprint.
#[test]
fn a_preview_edge_complete_native_poses_match_certified_publication() {
    for method in ["unreferenced", "mixed", "align", "transform"] {
        let mut s = fixture(false);
        let reference = pick(&mut s, usize::from(method == "align"), "fp.origin", None);
        add(&mut s, "horizontal", origin(), reference);
        let old = s.project().unwrap().board.footprints().to_vec();
        let ids: Vec<_> = old.iter().map(|f| format!("footprint-{}", f.id)).collect();
        let (command, params, indices) = match method {
            "unreferenced" => (board::movement::MOVE_ITEMS, json!({"items":{"footprints":[ids[1]]},"dx_nm":600_000,"dy_nm":20_000}), vec![1]),
            "mixed" => (board::movement::MOVE_ITEMS, json!({"items":{"footprints":[ids[1],ids[0]]},"dx_nm":600_000,"dy_nm":20_000}), vec![0, 1]),
            "align" => (crate::ALIGN_FOOTPRINTS, json!({"footprints":[ids[1],ids[0]],"mode":"left"}), vec![0, 1]),
            _ => {
                (crate::SET_FOOTPRINT_TRANSFORM, json!({"footprint":ids[1],"x_nm":15_000_000,"y_nm":6_000_000,"rotation":90,"side":"back"}), vec![1])
            }
        };
        let observer: Arc<Observe> = Arc::default();
        s.observe_edits(Some(observer.clone()));
        let before = s.revision();
        let p = reviewed(&mut s, command, params);
        let candidate = s.checked_generic_candidate().unwrap().clone();
        let next = candidate.board.footprints();
        assert_eq!(next[2], old[2]);
        match method {
            "unreferenced" | "mixed" => {
                for &i in &indices {
                    assert_eq!((next[i].x_nm, next[i].y_nm), (old[i].x_nm + 600_000, old[i].y_nm + 20_000));
                }
            }
            "align" => {
                assert_eq!(next[0], old[0]);
                assert_ne!(next[1].x_nm, old[1].x_nm);
            }
            _ => assert_eq!((next[1].x_nm, next[1].y_nm, next[1].rotation, next[1].bottom), (15_000_000, 6_000_000, 90, true)),
        }
        if indices == [1] {
            assert_eq!(next[0], old[0]);
        }
        let poses: Vec<_> = indices
            .into_iter()
            .map(|i| {
                let f = &next[i];
                json!({"fp":f.id,"x_nm":f.x_nm,"y_nm":f.y_nm,"rotation":f.rotation,"bottom":f.bottom})
            })
            .collect();
        assert_eq!(p["disclosure"], json!({"poses":poses,"losses":[],"remaps":[]}));
        let encoded = encode_documents(candidate.clone()).unwrap();
        assert!(observer.0.lock().unwrap().is_empty());
        let (reply, ops) = trace(|| run(&mut s, APPLY, decision(&p)).unwrap());
        assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
        assert_eq!(reply["candidate_digest"], p["candidate_digest"]);
        assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), p["candidate_digest"].as_str().unwrap());
        assert!(Arc::ptr_eq(&candidate, &s.snapshot().unwrap()));
        assert_eq!(bytes(&s), encoded);
        let events = observer.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].revision, before + 1);
        assert!(Arc::ptr_eq(&candidate, &events[0].project));
    }
}

/// Projected and lifecycle deletion must settle only after the canonical outer candidate.
#[test]
fn a_preview_edge_projected_and_sheet_retirement() {
    for accepted in [false, true] {
        for command in [crate::DELETE_SYMBOL, crate::DELETE_ITEMS, crate::clipboard::CUT, crate::sheets::DELETE_SHEET] {
            let mut s = fixture(accepted);
            let reference = pick(&mut s, 0, "fp.origin", None);
            add(&mut s, "horizontal", origin(), reference);
            let component = s.project().unwrap().board.footprints()[0].component;
            let symbol = s.project().unwrap().symbols.iter().find(|s| s.component == component).unwrap().id();
            let sheet = s.project().unwrap().resolve_sheet(None).unwrap().sheet;
            if command == crate::sheets::DELETE_SHEET {
                run(&mut s, crate::sheets::CREATE_SHEET, json!({"name":"Keep"})).unwrap();
            }
            let args = match command {
                crate::DELETE_SYMBOL => json!({"symbol":symbol,"sheet":sheet}),
                crate::sheets::DELETE_SHEET => json!({"sheet":sheet}),
                _ => json!({"items":{"symbols":[symbol],"wires":[]},"sheet":sheet}),
            };
            if accepted {
                run(&mut s, command, args).unwrap();
                assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
                let preview = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
                let loss = preview["relation_detail"]["losses"][0]["id"].clone();
                let retained = s.checked_eco_candidate().unwrap().clone();
                unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"plan_token":preview["plan_token"],"accept_loss":[loss]})));
                let (_, ops) = trace(|| {
                    run(&mut s, update::APPLY_UPDATE, json!({"plan_token":preview["plan_token"],"accept_loss":[loss],"confirm_remaps":[]})).unwrap()
                });
                assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
                assert!(Arc::ptr_eq(&retained, &s.snapshot().unwrap()));
            } else {
                let err = unchanged(&mut s, |s| run(s, command, args.clone()));
                assert_eq!(err.details()["reason"], "would_lose");
                let observer: Arc<Observe> = Arc::default();
                s.observe_edits(Some(observer.clone()));
                let (p, ops) = trace(|| reviewed(&mut s, command, args));
                assert_stage(&ops, usize::from(command != crate::sheets::DELETE_SHEET), 1, 0);
                assert!(observer.0.lock().unwrap().is_empty());
                let (response, ops) = trace(|| run(&mut s, APPLY, decision(&p)).unwrap());
                assert_eq!(ops, [Op::Publish(RelationStage::Outer)]);
                assert_eq!(observer.0.lock().unwrap().len(), 1);
                assert_eq!(response.get("clipboard").is_some(), command == crate::clipboard::CUT);
            }
            assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
        }
    }
}

/// An ECO splitting accepted memberships marks only the affected physical island. The same
/// logical change is valid with disconnected lands; its geometry and reference do not move.
#[test]
fn a_net_eco_local_split_short_and_disconnected_control() {
    for joined in [false, true] {
        let mut s = board::track::tests::routed_fixture();
        let project = s.project().unwrap();
        let component = project.symbols[2].component;
        let b = project.board.footprints().iter().position(|f| f.component == component).unwrap();
        let a = (0..3).find(|i| *i != b).unwrap();
        let center = |i: usize| {
            let f = &project.board.footprints()[i];
            f.pad_position(crate::schematic::PinNumber::new("2").unwrap(), &project.board).unwrap()
        };
        let (a_at, b_at) = (center(a), center(b));
        let symbol = project.symbols[2].id();
        let terminal = project.symbols[2].terminals()[0];
        let at = project.terminal_position(terminal).unwrap();
        let wire = project.wires.iter().find(|w| w.endpoints().contains(&crate::schematic::WireEndpoint::Terminal(terminal))).unwrap();
        let wire_id = wire.id();
        if joined {
            let front = board::track::tests::copper_layers(&s)[0];
            let y = a_at.y_nm.max(b_at.y_nm) + 2_000_000;
            board::track::tests::route(&mut s, front, &[a_at, Point { x_nm: a_at.x_nm, y_nm: y }, Point { x_nm: b_at.x_nm, y_nm: y }, b_at]).unwrap();
        }
        let first = pick(&mut s, a, "pad.center", Some("2"));
        add(&mut s, "horizontal", origin(), first);
        run(&mut s, crate::DELETE_WIRE, json!({"wire":wire_id})).unwrap();
        run(&mut s, crate::net_labels::ADD, json!({"name":"SEPARATE","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm})).unwrap();
        assert_eq!(s.project().unwrap().symbols[2].id(), symbol);
        let before = s.snapshot().unwrap();
        if joined {
            let err = unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({})));
            assert_eq!(err.details()["reason"], "affected_short");
        } else {
            let p = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
            run(&mut s, update::APPLY_UPDATE, json!({"plan_token":p["plan_token"]})).unwrap();
            assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), p["candidate_digest"].as_str().unwrap());
        }
        let after = s.project().unwrap();
        assert_eq!(before.board.footprints(), after.board.footprints());
        assert_eq!(before.board.tracks, after.board.tracks);
        assert_eq!(before.board.vias, after.board.vias);
    }
}

/// Compare the complete retained v1 base and require the only changes to existing contracts
/// to be optional ECO inputs. Contents are the exact main snapshot at `ce5867a9`, the pin P1.1 merges onto, not reconstructed.
#[test]
fn a_eco_compat_exact_base_command_contracts() {
    use sha2::{Digest, Sha256};
    let frozen = include_bytes!("base-command-contracts.json");
    assert_eq!(format!("{:x}", Sha256::digest(frozen)), "33698a6b3619eddea1c2447a66832ec2b69e7e986ba03924fce2bc7f45319eda");
    let base: Value = serde_json::from_slice(frozen).unwrap();
    let current: Value = serde_json::from_str(include_str!("../../../../../docs/schemas/command-contracts.json")).unwrap();
    let described: Vec<_> = crate::CATALOG.iter().map(crate::Command::to_json).collect();
    crate::command_api::check_snapshot(&current, "commands", &described, crate::command_api::COMMAND_API_VERSION).unwrap();
    let arms = current["commands"][PREVIEW]["params"]["properties"]["intent"]["oneOf"].as_array().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    assert_eq!(arms.len(), 16);
    for arm in arms {
        let id = arm["properties"]["command"]["const"].as_str().unwrap();
        assert!(seen.insert(id));
        let mut native = base["commands"][id]["params"].clone();
        native["properties"].as_object_mut().unwrap().remove("expected_revision");
        native["required"].as_array_mut().unwrap().retain(|p| p != "expected_revision");
        assert_eq!(arm["properties"]["params"], native, "nested native contract drift: {id}");
    }
    let changes = crate::command_api::compare_snapshots(&base, &current).unwrap();
    assert!(!changes.is_empty());
    assert_eq!(base["command_api_version"], current["command_api_version"]);
    for (name, prior) in base["commands"].as_object().unwrap() {
        let mut now = current["commands"][name].clone();
        let props = now["params"]["properties"].as_object_mut().unwrap();
        match name.as_str() {
            update::PREVIEW_UPDATE => {
                assert!(props.remove("remap_choices").is_some());
            }
            update::APPLY_UPDATE => {
                assert!(props.remove("accept_loss").is_some());
                assert!(props.remove("confirm_remaps").is_some());
            }
            _ => {}
        }
        assert_eq!(prior, &now, "native command contract drift: {name}");
    }
}

/// Assert at native stage boundaries, independently of the resulting envelope.
pub(crate) fn assert_stage(ops: &[Op], raw: usize, outer: usize, live_publish: usize) {
    let count = |op| ops.iter().filter(|v| **v == op).count();
    assert_eq!(count(Op::Prepare(RelationStage::Raw)), raw, "{ops:?}");
    assert_eq!(count(Op::Publish(RelationStage::Raw)), raw, "{ops:?}");
    assert_eq!(count(Op::Prepare(RelationStage::Outer)), outer, "{ops:?}");
    assert_eq!(count(Op::VerifyReferences), outer, "{ops:?}");
    assert_eq!(count(Op::Publish(RelationStage::Outer)), live_publish, "{ops:?}");
}

/// Guard every authorized nested native staging location; none is a client-facing input.
#[test]
fn a_nest_raw_sites_are_explicit_and_unadvertised() {
    let sources = [
        include_str!("../clipboard.rs"),
        include_str!("../../part_instances/commands.rs"),
        include_str!("../../sheets.rs"),
        include_str!("../../persistence/power_migration.rs"),
        include_str!("../../lib.rs"),
    ];
    for (source, expected) in sources.into_iter().zip([1, 2, 1, 1, 0]) {
        assert_eq!(source.matches("RelationStage::Raw").count(), expected);
    }
    let mut s = fixture(false);
    run(&mut s, "edit.undo", json!({})).unwrap();
    assert!(!s.undo.is_empty() && !s.redo.is_empty());
    let fp = s.project().unwrap().board.footprints()[0].id;
    let params = json!({"items":{"footprints":[format!("footprint-{fp}")]}});
    let command = board::clipboard::CUT;
    let mut request = json!({"intent":{"command":command,"params":params}});
    let control = run(&mut s, PREVIEW, request.clone()).unwrap();
    request["intent"]["params"]["relation_stage"] = json!("Raw");
    let error = unchanged(&mut s, |s| run(s, PREVIEW, request));
    assert!(matches!(&error, CommandError::InvalidParams(message) if message == "`pcb.cut` has no parameter `relation_stage`"), "{error:?}");
    run(&mut s, CANCEL, json!({"token":control["relation_preview"]["token"]})).unwrap();
}

/// F1 validation rejects `Kind::Gap` rows whose saved witness segments are perpendicular,
/// covering both `(vertical, horizontal)` and `(horizontal, vertical)` pairs.
#[test]
fn gap_rejects_perpendicular_saved_witness_segments() {
    let mut s = fixture(false);
    let edge = |s: &mut Session, fp, side| {
        let features = run(s, FEATURES, json!({"fp": format!("footprint-{fp}")})).unwrap();
        features["features"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|item| {
                let r = &item["reference"];
                (r["tag"] == "pad.edge" && r["pad"] == "1" && r["side"] == side).then(|| r.clone())
            })
            .unwrap()
    };
    let fp0 = s.project().unwrap().board.footprints()[0].id;
    let fp1 = s.project().unwrap().board.footprints()[1].id;
    let a = edge(&mut s, fp0, "left");
    let b = edge(&mut s, fp1, "left");
    add(&mut s, "gap", a, b);

    let docs = bytes(&s);
    let path = test_board_path(&docs);
    let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    assert!(decode_documents(&docs).is_ok());

    let vertical = json!({"kind": "segment", "a": {"x_nm": 0, "y_nm": 0}, "b": {"x_nm": 0, "y_nm": 1_000_000}});
    let horizontal = json!({"kind": "segment", "a": {"x_nm": 0, "y_nm": 0}, "b": {"x_nm": 1_000_000, "y_nm": 0}});
    for witnesses in [json!([&vertical, &horizontal]), json!([&horizontal, &vertical])] {
        let mut changed = board.clone();
        changed["relations"]["relations"][0]["last_resolved"] = witnesses;
        let parsed: Relations = serde_json::from_value(changed["relations"].clone()).unwrap();
        assert_eq!(parsed.validate().unwrap_err(), "invalid board relation identity, picks, datum, label, state or display witness");

        let mut doc = docs.clone();
        doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&doc).unwrap_err().contains("invalid board relation identity, picks, datum, label, state or display witness"));
    }
}

/// Parallel `Kind::Gap` saved witnesses remain structurally readable through F1 decoding even
/// after the referenced original footprints/features no longer exist on the board.
#[test]
fn gap_parallel_saved_witness_remains_readable_when_original_features_are_missing() {
    let mut s = fixture(false);
    let edge = |s: &mut Session, fp, side| {
        let features = run(s, FEATURES, json!({"fp": format!("footprint-{fp}")})).unwrap();
        features["features"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|item| {
                let r = &item["reference"];
                (r["tag"] == "pad.edge" && r["pad"] == "1" && r["side"] == side).then(|| r.clone())
            })
            .unwrap()
    };
    let fp0 = s.project().unwrap().board.footprints()[0].id;
    let fp1 = s.project().unwrap().board.footprints()[1].id;
    let a = edge(&mut s, fp0, "left");
    let b = edge(&mut s, fp1, "left");
    add(&mut s, "gap", a, b);

    let preview =
        reviewed(&mut s, board::clipboard::DELETE, json!({"items": {"footprints": [format!("footprint-{fp0}"), format!("footprint-{fp1}")]}}));
    run(&mut s, APPLY, decision(&preview)).unwrap();
    assert!(s.project().unwrap().board.footprints().iter().all(|f| f.id != fp0 && f.id != fp1));

    let docs = bytes(&s);
    let reopened = decode_documents(&docs).unwrap();
    let relations = reopened.board.relations.as_ref().expect("lost gap relation preserved");
    assert_eq!(relations.relations.len(), 1);
    assert_eq!(relations.relations[0].kind, Kind::Gap);
    assert!(matches!(relations.relations[0].state, State::Lost { since_revision } if since_revision > 0));
    assert!(relations.relations[0].last_resolved[0].parallel_segments(relations.relations[0].last_resolved[1]));

    // Horizontal parallel saved witnesses remain equally readable when original features are absent.
    let path = test_board_path(&docs);
    let mut board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    board["relations"]["relations"][0]["last_resolved"] = json!([
        {"kind": "segment", "a": {"x_nm": 0, "y_nm": 0}, "b": {"x_nm": 1_000_000, "y_nm": 0}},
        {"kind": "segment", "a": {"x_nm": 0, "y_nm": 2_000_000}, "b": {"x_nm": 1_000_000, "y_nm": 2_000_000}}
    ]);
    let mut horizontal_doc = docs.clone();
    horizontal_doc.insert(path, serde_json::to_vec(&board).unwrap());
    let reopened_horizontal = decode_documents(&horizontal_doc).unwrap();
    assert_eq!(reopened_horizontal.board.relations.as_ref().unwrap().relations.len(), 1);
}
