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
