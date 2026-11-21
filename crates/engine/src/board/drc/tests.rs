use super::*;
use crate::{
    ADD_DRAWING, ADD_TRACKS, ADD_VIA, SET_FOOTPRINT_TRANSFORM,
    board::track::tests::{copper_layers, route, routed_fixture, view},
};
use serde_json::json;

/// Close a Board Outline rectangle around every fixture footprint, with room to spare.
pub(crate) fn outline(s: &mut Session) {
    let layer = view(s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let p = |x: i64, y: i64| json!({"x_nm":x,"y_nm":y});
    s.apply(
        ADD_DRAWING,
        &json!({"layer":layer,"points":[p(-5_000_000,-5_000_000),p(25_000_000,-5_000_000),p(25_000_000,35_000_000),p(-5_000_000,35_000_000)],"closed":true,"expected_revision":s.revision()}),
    )
    .unwrap();
}

fn check(s: &Session) -> Report {
    view(s).check_design(Rules::default())
}

fn count(report: &Report, kind: Kind) -> usize {
    report.counts.get(&kind).copied().unwrap_or(0)
}

/// The two pad centers of a footprint whose pads are on two different accepted nets.
fn two_net_pads(s: &Session) -> (Point, Point) {
    let f = view(s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
    (f.pads[0].center, f.pads[1].center)
}

/// An unrouted board without an outline reports the missing outline and one unconnected item
/// per airwire, and nothing else. Closing an outline clears the outline finding.
#[test]
fn an_unrouted_board_reports_outline_and_unconnected_items() {
    let mut s = routed_fixture();
    let report = check(&s);
    assert_eq!(count(&report, Kind::NoOutline), 1);
    assert_eq!(count(&report, Kind::Unconnected), view(&s).ratsnest().unwrap().lines.len());
    assert_eq!(report.total, 1 + count(&report, Kind::Unconnected), "{report:#?}");
    outline(&mut s);
    let report = check(&s);
    assert_eq!(count(&report, Kind::NoOutline), 0);
    assert_eq!(report.total, count(&report, Kind::Unconnected), "{report:#?}");
}

/// A thin track through the 0.45 mm gap between two nets' lands is 0.175 mm from each, below a
/// 0.2 mm clearance, so both pairs are reported; a 0.15 mm rule accepts it.
#[test]
fn clearance_between_different_nets_is_measured_edge_to_edge() {
    let mut s = routed_fixture();
    outline(&mut s);
    let front = copper_layers(&s)[0];
    let (a, b) = two_net_pads(&s);
    let x = (a.x_nm + b.x_nm) / 2;
    s.apply(
        ADD_TRACKS,
        &json!({"layer":front,"width_nm":100_000,"points":[{"x_nm":x,"y_nm":a.y_nm - 2_000_000},{"x_nm":x,"y_nm":a.y_nm + 2_000_000}],"expected_revision":s.revision()}),
    )
    .unwrap();
    let report = check(&s);
    let clearance: Vec<_> = report.violations.iter().filter(|v| v.kind == Kind::Clearance).collect();
    assert_eq!(clearance.len(), 2, "{clearance:#?}");
    assert!(clearance.iter().all(|v| v.message.contains("0.175 mm")), "{clearance:#?}");
    let relaxed = view(&s).check_design(Rules { clearance_nm: 150_000, ..Rules::default() });
    assert_eq!(count(&relaxed, Kind::Clearance), 0);
    assert_eq!(count(&report, Kind::TrackWidth), 1, "0.1 mm is below the 0.15 mm minimum");
}

/// A routed connection between pads of one net is not a clearance finding, and routing it
/// removes its unconnected item.
#[test]
fn routed_same_net_copper_is_clean() {
    let mut s = routed_fixture();
    outline(&mut s);
    let before = count(&check(&s), Kind::Unconnected);
    let line = view(&s).ratsnest().unwrap().lines[0].clone();
    let front = copper_layers(&s)[0];
    let (a, b) = (line.from.position, line.to.position);
    let y = a.y_nm.max(b.y_nm) + 2_000_000;
    route(&mut s, front, &[a, Point { x_nm: a.x_nm, y_nm: y }, Point { x_nm: b.x_nm, y_nm: y }, b]).unwrap();
    let report = check(&s);
    assert_eq!(count(&report, Kind::Unconnected), before - 1);
    assert_eq!(count(&report, Kind::Clearance), 0, "{report:#?}");
}

/// Copper outside the outline or within the edge clearance is reported, and a via whose ring
/// is thinner than the minimum is reported.
#[test]
fn outline_edge_and_annular_ring_findings() {
    let mut s = routed_fixture();
    outline(&mut s);
    let front = copper_layers(&s)[0];
    route(&mut s, front, &[Point { x_nm: 30_000_000, y_nm: 0 }, Point { x_nm: 32_000_000, y_nm: 0 }]).unwrap();
    route(&mut s, front, &[Point { x_nm: 0, y_nm: -4_900_000 }, Point { x_nm: 2_000_000, y_nm: -4_900_000 }]).unwrap();
    s.apply(
        ADD_VIA,
        &json!({"at":{"x_nm":10_000_000,"y_nm":25_000_000},"via_diameter_nm":400_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let report = check(&s);
    assert_eq!(count(&report, Kind::OutsideBoard), 1, "{report:#?}");
    assert_eq!(count(&report, Kind::EdgeClearance), 1, "{report:#?}");
    assert_eq!(count(&report, Kind::AnnularRing), 1, "{report:#?}");
    let ring = report.violations.iter().find(|v| v.kind == Kind::AnnularRing).unwrap();
    assert_eq!(ring.at, Point { x_nm: 10_000_000, y_nm: 25_000_000 });
}

/// Moving one footprint onto another overlaps their courtyards.
#[test]
fn overlapping_courtyards_are_reported() {
    let mut s = routed_fixture();
    outline(&mut s);
    let (moved, onto) = {
        let footprints: Vec<_> = view(&s).footprints().map(|f| (f.id.clone(), f.position, f.rotation)).collect();
        (footprints[0].clone(), footprints[1].1)
    };
    assert_eq!(count(&check(&s), Kind::CourtyardOverlap), 0);
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":moved.0,"x_nm":onto.x_nm + 300_000,"y_nm":onto.y_nm,"rotation":moved.2,"expected_revision":s.revision()}),
    )
    .unwrap();
    let report = check(&s);
    assert_eq!(count(&report, Kind::CourtyardOverlap), 1, "{report:#?}");
}

/// The command edits nothing, returns its report through execute, and validates its rules.
#[test]
fn check_design_command_reports_without_editing() {
    let mut s = routed_fixture();
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let result = s.execute(CHECK_DESIGN, &json!({"clearance_nm":250_000})).unwrap();
    assert_eq!(result["revision"], revision);
    assert_eq!(result["design_check"]["rules"]["clearance_nm"], 250_000);
    assert_eq!(result["design_check"]["counts"]["no_outline"], 1);
    assert_eq!(s.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    assert!(s.execute(CHECK_DESIGN, &json!({"clearance_nm":-1})).is_err());
    assert!(s.execute(CHECK_DESIGN, &json!({"board":uuid::Uuid::from_u128(9)})).is_err());
}

/// Add a closed square Board Outline drawing with its lower corner at (`x`, `y`) mm and return
/// its UUID.
pub(crate) fn outline_square(s: &mut Session, x_mm: i64, y_mm: i64, side_mm: i64) -> Uuid {
    let layer = view(s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let (x, y, side) = (x_mm * 1_000_000, y_mm * 1_000_000, side_mm * 1_000_000);
    let points = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)].map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    s.apply(ADD_DRAWING, &json!({"layer":layer,"points":points,"closed":true,"expected_revision":s.revision()})).unwrap();
    view(s).drawings().iter().find(|drawing| drawing.points == points).unwrap().id
}

/// The routed fixture inside a 40 mm square board with a closed 10 mm square cut-out from
/// (5, 15) mm to (15, 25) mm, clear of every footprint. Returns the cut-out drawing.
fn cut_out_board() -> (Session, Uuid) {
    let mut s = routed_fixture();
    outline_square(&mut s, -5, -5, 40);
    let cut_out = outline_square(&mut s, 5, 15, 10);
    (s, cut_out)
}

/// Add one front-copper track between two points in millimetres and return its UUID.
fn track_mm(s: &mut Session, from: (f64, f64), to: (f64, f64)) -> String {
    let front = copper_layers(s)[0];
    let at = |(x, y): (f64, f64)| Point { x_nm: (x * 1e6) as i64, y_nm: (y * 1e6) as i64 };
    route(s, front, &[at(from), at(to)]).unwrap();
    view(s).tracks().iter().find(|track| track.start == at(from)).unwrap().id.to_string()
}

/// The outline findings about one copper item.
fn outline_findings<'a>(report: &'a Report, item: &str) -> Vec<&'a Violation> {
    report.violations.iter().filter(|v| matches!(v.kind, Kind::OutsideBoard | Kind::EdgeClearance) && v.items[0] == item).collect()
}

/// A closed contour inside the board outline is a cut-out: copper fully inside it is off the
/// board and the finding names the cut-out drawing, while the same track on solid board is clean.
#[test]
fn copper_inside_a_cut_out_is_outside_the_board() {
    let (mut s, cut_out) = cut_out_board();
    let inside = track_mm(&mut s, (8., 20.), (12., 20.));
    let report = check(&s);
    let found = outline_findings(&report, &inside);
    assert_eq!(found.len(), 1, "{report:#?}");
    assert_eq!(found[0].kind, Kind::OutsideBoard);
    assert_eq!(found[0].message, "Copper lies inside a board cut-out");
    assert_eq!(found[0].items, vec![inside.clone(), cut_out.to_string()]);
    assert_eq!(count(&report, Kind::OutsideBoard), 1, "{report:#?}");

    let (mut s, _) = cut_out_board();
    let solid = track_mm(&mut s, (8., 30.), (12., 30.));
    let report = check(&s);
    assert!(outline_findings(&report, &solid).is_empty(), "{report:#?}");
    assert_eq!(count(&report, Kind::OutsideBoard) + count(&report, Kind::EdgeClearance), 0, "{report:#?}");
}

/// Copper that starts on the board and ends in the cut-out is off the board, as copper leaving
/// the outer edge is; copper with both ends on the board that crosses the cut-out still crosses
/// the outline; copper beside the cut-out keeps the ordinary edge clearance to it.
#[test]
fn copper_at_a_cut_out_edge_keeps_the_edge_findings() {
    let (mut s, cut_out) = cut_out_board();
    let entering = track_mm(&mut s, (3., 20.), (8., 20.));
    let across = track_mm(&mut s, (3., 24.), (17., 24.));
    let near = track_mm(&mut s, (16., 17.), (16., 23.));
    let report = check(&s);
    let entering = outline_findings(&report, &entering);
    assert_eq!(entering.len(), 1, "{report:#?}");
    assert_eq!((entering[0].kind, entering[0].items[1].as_str()), (Kind::OutsideBoard, cut_out.to_string().as_str()));
    let across = outline_findings(&report, &across);
    assert_eq!(across.len(), 1, "{report:#?}");
    assert_eq!((across[0].kind, across[0].message.as_str()), (Kind::EdgeClearance, "Copper crosses the Board Outline"));
    assert!(outline_findings(&report, &near).is_empty(), "1 mm from the cut-out clears the 0.3 mm rule: {report:#?}");
    let strict = view(&s).check_design(Rules { edge_clearance_nm: 1_000_000, ..Rules::default() });
    let near = outline_findings(&strict, &near);
    assert_eq!(near.len(), 1, "{strict:#?}");
    assert_eq!(near[0].kind, Kind::EdgeClearance);
}

/// A contour nested inside the cut-out is board again. Copper on that island is legal; copper
/// in the cut-out next to the island is still off the board and names the cut-out, not the
/// nearer island.
#[test]
fn an_island_inside_a_cut_out_is_board_again() {
    let (mut s, cut_out) = cut_out_board();
    let island = outline_square(&mut s, 8, 18, 4);
    let on_island = track_mm(&mut s, (9.5, 20.), (10.5, 20.));
    let beside = track_mm(&mut s, (12.6, 19.), (12.6, 21.));
    let report = check(&s);
    assert!(outline_findings(&report, &on_island).is_empty(), "{report:#?}");
    let beside = outline_findings(&report, &beside);
    assert_eq!(beside.len(), 1, "{report:#?}");
    assert_eq!(beside[0].kind, Kind::OutsideBoard);
    assert_eq!(beside[0].items[1], cut_out.to_string());
    assert_ne!(beside[0].items[1], island.to_string());
}

/// Closed outlines that overlap are one board, not a board with a cut-out: an L-shaped board
/// drawn as two overlapping squares keeps copper in their shared area and in each arm legal.
#[test]
fn overlapping_outlines_form_one_board_without_cut_outs() {
    let mut s = routed_fixture();
    outline_square(&mut s, -5, -5, 40);
    outline_square(&mut s, 25, -5, 20);
    let shared = track_mm(&mut s, (28., 10.), (32., 10.));
    let arm = track_mm(&mut s, (38., 2.), (42., 2.));
    let outside = track_mm(&mut s, (38., 25.), (42., 25.));
    let report = check(&s);
    assert!(outline_findings(&report, &shared).is_empty(), "{report:#?}");
    assert!(outline_findings(&report, &arm).is_empty(), "{report:#?}");
    let outside = outline_findings(&report, &outside);
    assert_eq!(outside.len(), 1, "{report:#?}");
    assert_eq!(outside[0].message, "Copper lies outside the Board Outline");
    assert_eq!(outside[0].items.len(), 1);
    assert_eq!(count(&report, Kind::OutsideBoard), 1, "{report:#?}");
}

/// A Board Outline drawn twice in the same place is still one board: copper on it stays legal,
/// and a contour touching another is not a cut-out either.
#[test]
fn duplicated_or_touching_outlines_make_no_cut_out() {
    let mut s = routed_fixture();
    outline_square(&mut s, -5, -5, 40);
    outline_square(&mut s, -5, -5, 40);
    let solid = track_mm(&mut s, (8., 30.), (12., 30.));
    let report = check(&s);
    assert!(outline_findings(&report, &solid).is_empty(), "{report:#?}");
    assert_eq!(count(&report, Kind::OutsideBoard), 0, "{report:#?}");

    let mut s = routed_fixture();
    outline_square(&mut s, -5, -5, 40);
    outline_square(&mut s, -5, 15, 10);
    let inside = track_mm(&mut s, (-1., 20.), (1., 20.));
    let report = check(&s);
    assert!(outline_findings(&report, &inside).is_empty(), "{report:#?}");
    assert_eq!(count(&report, Kind::OutsideBoard), 0, "{report:#?}");
}

/// A single closed outline keeps its existing wording, and the resumable check reports exactly
/// what the direct check reports for cut-outs and islands at every batch size.
#[test]
fn cut_out_findings_agree_between_direct_and_resumable_checks() {
    let mut s = routed_fixture();
    outline(&mut s);
    let outside = track_mm(&mut s, (30., 0.), (32., 0.));
    let report = check(&s);
    assert_eq!(outline_findings(&report, &outside)[0].message, "Copper lies outside the Board Outline");
    assert_eq!(outline_findings(&report, &outside)[0].items, vec![outside.clone()]);

    let (mut s, _) = cut_out_board();
    outline_square(&mut s, 8, 18, 4);
    for (from, to) in [((6., 20.), (6.5, 20.)), ((9.5, 20.), (10.5, 20.)), ((3., 23.), (17., 23.)), ((30., 30.), (40., 30.))] {
        track_mm(&mut s, from, to);
    }
    let expected = s.check_design(&json!({})).unwrap();
    assert_eq!(expected["counts"]["outside_board"], 2, "{expected:#}");
    for budget in [1, 3, 256] {
        let mut job = s.design_check_job(&json!({})).unwrap();
        while !job.step(budget) {}
        assert_eq!(json!(job.take_report().unwrap()), expected, "budget={budget}");
    }
}
