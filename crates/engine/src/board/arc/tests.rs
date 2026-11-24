mod limits;
mod reconciliation;

use super::*;
use crate::{
    board::track::tests::{copper_layers, routed_fixture, view},
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};

/// The shared production commands create all project, package, baseline and layer state.
fn fixture() -> (Session, ArcTrack) {
    let session = routed_fixture();
    let arc = ArcTrack {
        id: Uuid::from_u128(0xa11c),
        layer: copper_layers(&session)[0],
        width_nm: 250_000,
        center: Point { x_nm: 0, y_nm: 0 },
        start: Point { x_nm: 5_000_000, y_nm: 0 },
        end: Point { x_nm: 0, y_nm: 5_000_000 },
        direction: Direction::Clockwise,
    };
    (session, arc)
}
/// Both UI and headless callers supply the same explicit board, revision and stable records.
fn params(session: &Session, arcs: &[ArcTrack]) -> Value {
    json!({"board":view(session).target.board,"expected_revision":session.revision(),"arcs":arcs})
}

/// Invalid authored input retains its public error category as well as its diagnostic.
fn assert_invalid_params(error: CommandError, reason: &str) {
    assert!(matches!(&error, CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains(reason), "{error}");
}
/// Valid requests refused by current session state are unavailable, not malformed input.
fn assert_unavailable(error: CommandError, reason: &str) {
    assert!(matches!(&error, CommandError::Unavailable(_)), "{error:?}");
    assert!(error.to_string().contains(reason), "{error}");
}

/// Discovery and execution require explicit board authority even with one available board.
#[test]
fn arc_command_requires_explicit_board_without_changing_history() {
    let description = crate::catalog().iter().find(|c| c.id == SET_ARCS).unwrap().to_json();
    assert_eq!(description["params"]["required"], json!(["board", "expected_revision", "arcs"]));
    let (mut s, arc) = fixture();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let mut omitted = params(&s, &[arc]);
    omitted.as_object_mut().unwrap().remove("board");
    assert_invalid_params(s.execute(SET_ARCS, &omitted).unwrap_err(), "needs `board`");
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Dry runs, no-ops, undo and native document/archive reopen preserve exact authored bytes.
#[test]
fn exact_arcs_round_trip_and_remove_without_changing_older_bytes() {
    let (mut s, arc) = fixture();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let mut request = params(&s, std::slice::from_ref(&arc));
    request["dry_run"] = json!(true);
    let preview = s.execute(SET_ARCS, &request).unwrap();
    assert_eq!(preview["arc_tracks"]["before"], json!([]));
    assert_eq!(preview["arc_tracks"]["after"], json!([arc]));
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    request["dry_run"] = json!(false);
    let result = s.execute(SET_ARCS, &request).unwrap();
    assert_eq!(result["arc_tracks"]["after"], preview["arc_tracks"]["after"]);
    assert_eq!(s.revision(), revision + 1);
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.boards().next().unwrap().arc_tracks(), &[arc]);
    let packed = encode_packed(reopened).unwrap();
    assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
    let unchanged = params(&s, view(&s).arc_tracks());
    s.apply(SET_ARCS, &unchanged).unwrap();
    assert_eq!(s.revision(), revision + 1);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    s.apply(SET_ARCS, &params(&s, &[])).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
}

/// Inclusive radial and width boundaries use exact integers, including irrational radii.
#[test]
fn exact_radius_and_sweep_boundaries_are_platform_independent() {
    let (_, arc) = fixture();
    assert_eq!(arc.radius_squared_nm2().unwrap(), 25_000_000_000_000);
    for direction in [Direction::Clockwise, Direction::Counterclockwise] {
        for end in [
            Point { x_nm: 0, y_nm: 5_000_000 },
            Point { x_nm: 0, y_nm: 5_000_001 },
            Point { x_nm: 0, y_nm: 4_999_999 },
            Point { x_nm: -5_000_000, y_nm: 0 },
        ] {
            let mut a = arc.clone();
            a.direction = direction;
            a.end = end;
            assert!(a.validate().is_ok(), "{a:?}");
        }
    }
    for width_nm in [super::super::track::MIN_WIDTH_NM, super::super::track::MAX_WIDTH_NM] {
        let mut a = arc.clone();
        a.width_nm = width_nm;
        assert!(a.validate().is_ok());
    }
    let mut irrational = arc.clone();
    irrational.start = Point { x_nm: 1, y_nm: 1 };
    irrational.end = Point { x_nm: -1, y_nm: 1 };
    assert_eq!(irrational.radius_squared_nm2().unwrap(), 2);
    assert!(irrational.validate().is_ok());
    irrational.end = Point { x_nm: -2, y_nm: 1 }; // sqrt(5) - sqrt(2) < 1
    assert!(irrational.validate().is_ok());
    irrational.end = Point { x_nm: -2, y_nm: 2 }; // sqrt(8) - sqrt(2) > 1
    assert!(irrational.validate().unwrap_err().contains("more than 1 nm"));
    let max = crate::schematic::MAX_CENTER_NM;
    let mut large = arc.clone();
    large.center = Point { x_nm: -max, y_nm: -max };
    large.start = Point { x_nm: max, y_nm: -max };
    large.end = Point { x_nm: -max, y_nm: max };
    assert!(large.validate().is_ok());
    large.end.x_nm -= 1;
    assert!(large.validate().unwrap_err().contains("bounded points"));
    large.center.x_nm = i64::MIN;
    large.start.x_nm = i64::MAX;
    assert!(large.radius_squared_nm2().unwrap_err().contains("out of range"));
}

/// Invalid authored records pair with semantic reasons for command and native-path regressions.
fn invalid_records(s: &Session, arc: &ArcTrack) -> Vec<(ArcTrack, &'static str)> {
    let mut bad = Vec::new();
    for (end, reason) in [
        (arc.start, "nonzero radius and distinct ends"),
        (arc.center, "nonzero radius and distinct ends"),
        (Point { x_nm: 0, y_nm: 5_000_002 }, "more than 1 nm"),
        (Point { x_nm: 0, y_nm: 4_999_998 }, "more than 1 nm"),
        (Point { x_nm: 5_000_001, y_nm: 0 }, "nonzero angular sweep"),
    ] {
        let mut a = arc.clone();
        a.end = end;
        bad.push((a, reason));
    }
    let mut a = arc.clone();
    a.start = a.center;
    bad.push((a, "nonzero radius and distinct ends"));
    for width_nm in [super::super::track::MIN_WIDTH_NM - 1, super::super::track::MAX_WIDTH_NM + 1] {
        let mut a = arc.clone();
        a.width_nm = width_nm;
        bad.push((a, "supported track width"));
    }
    let mut a = arc.clone();
    a.id = Uuid::nil();
    bad.push((a, "non-nil identities"));
    let mut a = arc.clone();
    a.layer = Uuid::nil();
    bad.push((a, "non-nil identities"));
    let mut a = arc.clone();
    a.layer = Uuid::from_u128(999);
    bad.push((a, "copper layer"));
    let mut a = arc.clone();
    a.layer = view(s).layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
    bad.push((a, "copper layer"));
    for x_nm in [-crate::schematic::MAX_CENTER_NM - 1, crate::schematic::MAX_CENTER_NM + 1] {
        let mut a = arc.clone();
        a.center.x_nm = x_nm;
        bad.push((a, "bounded points"));
    }
    bad
}

/// Invalid records and stale authority cannot alter saved data, revision or history.
#[test]
fn invalid_arcs_fail_atomically_on_the_shared_command_path() {
    let (mut s, arc) = fixture();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for (a, reason) in invalid_records(&s, &arc) {
        assert!(validate(s.project().unwrap(), std::slice::from_ref(&a)).unwrap_err().contains(reason), "{a:?}: {reason}");
        let error = s.execute(SET_ARCS, &params(&s, &[a])).unwrap_err();
        let command_reason = if reason == "copper layer" { reason } else { "invalid circular arc geometry" };
        assert_invalid_params(error, command_reason);
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    assert_invalid_params(s.execute(SET_ARCS, &params(&s, &[arc.clone(), arc.clone()])).unwrap_err(), "duplicate arc");
    let mut foreign = params(&s, std::slice::from_ref(&arc));
    foreign["board"] = json!(Uuid::from_u128(999));
    let error = s.execute(SET_ARCS, &foreign).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::NotFound);
    assert!(error.to_string().contains("unknown board ID"), "{error}");
    let mut stale = params(&s, &[arc]);
    stale["expected_revision"] = json!(revision - 1);
    let error = s.execute(SET_ARCS, &stale).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::StaleRevision);
    assert_eq!(error.details(), json!({"current_revision":revision}));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Geometry consumers refuse exact arcs, while stackup inspection remains available.
#[test]
fn unsupported_consumers_refuse_curves_instead_of_using_the_chord() {
    let (mut s, arc) = fixture();
    let selection = super::super::clipboard::BoardSelection { footprints: view(&s).footprint_ids().collect(), ..Default::default() };
    let fragment = super::super::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, false).unwrap();
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let v = view(&s);
    let target = v.target;
    assert_eq!(v.copper().unwrap_err(), UNSUPPORTED);
    assert_eq!(v.copper_nets().unwrap_err(), UNSUPPORTED);
    assert_eq!(v.copper_with_nets().unwrap_err(), UNSUPPORTED);
    assert_eq!(v.ratsnest().unwrap_err(), UNSUPPORTED);
    assert_eq!(v.carried_tracks("footprint-1", Point { x_nm: 0, y_nm: 0 }, 0, false).unwrap_err(), UNSUPPORTED);
    assert_eq!(v.fabrication().err().unwrap(), UNSUPPORTED);
    assert_eq!(v.fabrication_outputs().err().unwrap(), UNSUPPORTED);
    assert_unavailable(v.net_class_target_for_copper("anything").err().unwrap(), UNSUPPORTED);
    assert_unavailable(v.route_rules_at(Point { x_nm: 0, y_nm: 0 }, None).err().unwrap(), UNSUPPORTED);
    let report = v.check_design(Default::default());
    assert_eq!(report.total, 1);
    assert_eq!(report.violations[0].kind, super::super::drc::Kind::UnsupportedGeometry);
    assert_unavailable(s.clearance_query(target, 100_000).err().unwrap(), UNSUPPORTED);
    assert_unavailable(s.copper_preparation(target).err().unwrap(), UNSUPPORTED);
    assert_unavailable(s.check_design(&json!({"board":target.board})).unwrap_err(), UNSUPPORTED);
    assert_unavailable(s.design_check_job(&json!({"board":target.board})).err().unwrap(), UNSUPPORTED);
    assert_unavailable(super::super::movement::MovePlan::new(s.snapshot().unwrap(), target, selection.clone()).err().unwrap(), UNSUPPORTED);
    assert_unavailable(super::super::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, false).err().unwrap(), UNSUPPORTED);
    assert_unavailable(fragment.preview(s.project().unwrap(), 1_000, &Default::default()).err().unwrap(), UNSUPPORTED);
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let groups = view(&s).setup().unwrap().layer_groups();
    assert_eq!(
        s.execute(crate::INSPECT_LAYER_GROUPS, &json!({"board":target.board})).unwrap(),
        json!({"revision":s.revision(),"layer_groups":groups})
    );
    assert_unavailable(
        s.apply(crate::ADD_VIA, &json!({"board":target.board,"at":{"x_nm":0,"y_nm":0},"expected_revision":s.revision()})).unwrap_err(),
        UNSUPPORTED,
    );
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    assert_eq!(s.inspect()["project"]["boards"][0]["geometry_supported"], false);
}

/// The arc field belongs only to its explicit format and cannot be absent, null or empty.
#[test]
fn format26_rejects_missing_null_empty_and_older_headers() {
    let (mut s, arc) = fixture();
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = s.project().unwrap().board_document_path().unwrap();
    let board: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/26");
    for replacement in [None, Some(Value::Null), Some(json!([]))] {
        let mut b = board.clone();
        if let Some(value) = replacement {
            b["arcs"] = value;
        } else {
            b.as_object_mut().unwrap().remove("arcs");
        }
        let mut d = documents.clone();
        d.insert(path.clone(), serde_json::to_vec(&b).unwrap());
        let error = decode_documents(&d).unwrap_err();
        assert!(error.contains("arcs") || error.contains("null"), "{error}");
    }
    for header in [
        "pcbcraft-board/12",
        "pcbcraft-board/18",
        "pcbcraft-board/19",
        "pcbcraft-board/20",
        "pcbcraft-board/21",
        "pcbcraft-board/22",
        "pcbcraft-board/23",
        "pcbcraft-board/24",
        "pcbcraft-board/25",
        "pcbcraft-board/27",
    ] {
        let mut b = board.clone();
        b["format"] = json!(header);
        let mut d = documents.clone();
        d.insert(path.clone(), serde_json::to_vec(&b).unwrap());
        assert!(decode_documents(&d).unwrap_err().contains("arcs require format 26"), "{header}");
    }
}
