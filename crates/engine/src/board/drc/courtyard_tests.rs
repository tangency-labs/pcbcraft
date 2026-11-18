//! Exact courtyard decisions: predicate cases, an independent convex clipping oracle and
//! original command-built boards that exercise transforms, history, persistence and the job.
use super::*;
use crate::{
    ASSIGN_FOOTPRINT, CHECK_DESIGN, INITIALIZE_BOARD_SETUP, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM, Session,
    board::{
        definition::{Artwork, FootprintDefinition, GraphicRect, Source, basic_components},
        drc::{Report, Rules},
        track::tests::view,
    },
    library_graphics::{Fill, Geometry, Graphic},
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;
use std::sync::Arc;

fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

fn polygon(points: &[(i64, i64)]) -> Polygon {
    Polygon::new(&points.iter().map(|&(x, y)| p(x, y)).collect::<Vec<_>>()).expect("supported polygon")
}

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
    polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// The exact decision, checked in both argument orders and both orientations.
fn overlaps(a: &Polygon, b: &Polygon) -> bool {
    let forward = a.overlap_witness(b).is_some();
    assert_eq!(forward, b.overlap_witness(a).is_some(), "symmetric for {a:?} and {b:?}");
    let reversed = |q: &Polygon| polygon(&q.points.iter().rev().map(|v| (v.x_nm, v.y_nm)).collect::<Vec<_>>());
    assert_eq!(forward, reversed(a).overlap_witness(&reversed(b)).is_some(), "orientation independent");
    forward
}

/// Interior overlap, containment and the touching cases that must stay allowed.
#[test]
fn touching_is_allowed_and_interior_overlap_or_containment_is_reported() {
    let square = rect(0, 0, 10, 10);
    assert!(!overlaps(&square, &rect(20, 0, 30, 10)), "disjoint");
    assert!(!overlaps(&square, &rect(10, 0, 20, 10)), "shared edge");
    assert!(!overlaps(&square, &rect(10, 5, 20, 15)), "partly shared edge");
    assert!(!overlaps(&square, &rect(10, 10, 20, 20)), "shared corner");
    assert!(!overlaps(&square, &polygon(&[(10, 5), (20, 0), (20, 10)])), "vertex on an edge");
    assert!(overlaps(&square, &rect(5, 5, 15, 15)), "crossing edges");
    assert!(overlaps(&square, &rect(2, 2, 8, 8)), "strict containment");
    assert!(overlaps(&square, &rect(0, 0, 5, 10)), "containment along shared edges");
    assert!(overlaps(&square, &rect(0, 0, 10, 10)), "identical");
    assert!(overlaps(&square, &polygon(&[(0, 0), (10, 0), (10, 10)])), "triangle on the diagonal");
    assert!(overlaps(&square, &polygon(&[(10, 10), (5, 0), (15, 0)])), "vertex enters through a corner");
}

/// Rotated and concave shapes whose bounds overlap are decided by their actual edges.
#[test]
fn rotated_and_concave_shapes_ignore_empty_bounding_box_space() {
    let diamond = polygon(&[(10, 0), (20, 10), (10, 20), (0, 10)]);
    assert!(!overlaps(&diamond, &rect(15, 15, 25, 25)), "outside a diamond corner");
    assert!(!overlaps(&diamond, &polygon(&[(15, 5), (25, 15), (25, 0)])), "touching a diamond edge");
    assert!(overlaps(&diamond, &rect(14, 14, 25, 25)), "entering past the edge");
    let ell = polygon(&[(0, 0), (30, 0), (30, 10), (10, 10), (10, 30), (0, 30)]);
    assert!(!overlaps(&ell, &rect(15, 15, 25, 25)), "inside the notch");
    assert!(!overlaps(&ell, &rect(10, 10, 25, 25)), "filling the notch corner");
    assert!(overlaps(&ell, &rect(9, 15, 25, 25)), "reaching into the vertical arm");
    assert!(overlaps(&ell, &rect(2, 2, 8, 8)), "inside the corner block");
}

/// Repeated points and straight-through vertices are normalized; unsupported shapes are refused.
#[test]
fn polygons_normalize_closing_points_and_refuse_degenerate_or_crossing_paths() {
    let closed = polygon(&[(0, 0), (5, 0), (10, 0), (10, 10), (10, 10), (0, 10), (0, 0)]);
    assert_eq!(closed.points.len(), 4);
    let refused = |points: &[(i64, i64)]| Polygon::new(&points.iter().map(|&(x, y)| p(x, y)).collect::<Vec<_>>()).is_none();
    assert!(refused(&[]));
    assert!(refused(&[(0, 0), (10, 0)]));
    assert!(refused(&[(0, 0), (5, 0), (10, 0)]), "zero area");
    assert!(refused(&[(0, 0), (10, 10), (10, 0), (0, 10)]), "bow tie");
    assert!(refused(&[(0, 0), (10, 0), (20, 0), (10, 0), (10, 10)]), "spike");
    assert!(refused(&[(0, 0), (20, 0), (20, 20), (0, 20), (0, 10), (30, 10), (30, 5), (0, 5)]), "edges cross");
}

/// Twice the signed area of a floating polygon.
fn area(points: &[(f64, f64)]) -> f64 {
    (0..points.len()).map(|i| points[i].0 * points[(i + 1) % points.len()].1 - points[(i + 1) % points.len()].0 * points[i].1).sum()
}

/// Sutherland–Hodgman: clip `subject` by a positively oriented convex `clip`.
fn clip(subject: &[(f64, f64)], clip: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut output = subject.to_vec();
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let side = |p: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let input = std::mem::take(&mut output);
        for j in 0..input.len() {
            let (from, to) = (input[j], input[(j + 1) % input.len()]);
            let (s, e) = (side(from), side(to));
            if s >= 0. {
                output.push(from);
            }
            if (s >= 0.) != (e >= 0.) {
                let t = s / (s - e);
                output.push((from.0 + t * (to.0 - from.0), from.1 + t * (to.1 - from.1)));
            }
        }
    }
    output
}

/// Small integer convex shapes, including many touching and coincident cases, agree with
/// the area of their clipped intersection. The generator is a fixed LCG for determinism.
#[test]
fn convex_cases_agree_with_an_independent_clipping_oracle() {
    let mut state = 0x2750_u64;
    let mut next = |n: i64| {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % n as u64) as i64
    };
    let shape = |next: &mut dyn FnMut(i64) -> i64| loop {
        let points: Vec<(i64, i64)> = if next(2) == 0 {
            let (x, y) = (next(6), next(6));
            vec![(x, y), (x + 1 + next(4), y), (x + 1 + next(4), y + 1 + next(4)), (x, y + 1 + next(4))]
        } else {
            (0..3).map(|_| (next(7), next(7))).collect()
        };
        if let Some(polygon) = Polygon::new(&points.iter().map(|&(x, y)| p(x, y)).collect::<Vec<_>>()) {
            let convex = (0..polygon.points.len()).all(|i| {
                let n = polygon.points.len();
                cross(polygon.points[(i + n - 1) % n], polygon.points[i], polygon.points[(i + 1) % n]) > 0
            });
            if convex {
                return polygon;
            }
        }
    };
    let (mut overlapping, mut separate) = (0, 0);
    for _ in 0..20_000 {
        let (a, b) = (shape(&mut next), shape(&mut next));
        let float = |q: &Polygon| q.points.iter().map(|v| (v.x_nm as f64, v.y_nm as f64)).collect::<Vec<_>>();
        let expected = area(&clip(&float(&a), &float(&b))).abs() > 1e-9;
        assert_eq!(overlaps(&a, &b), expected, "{a:?} {b:?}");
        if expected { overlapping += 1 } else { separate += 1 }
    }
    assert!(overlapping > 1000 && separate > 1000, "{overlapping} overlapping, {separate} separate");
}

// Command-built boards.

/// A validated original definition with the given courtyard graphics and no other artwork.
fn definition(id: &str, graphics: Vec<GraphicRect>, artwork: Vec<Artwork>) -> FootprintDefinition {
    let mut definition = (*basic_components("resistor-0402").unwrap()).clone();
    definition.library = "drc-courtyards".into();
    definition.id = id.into();
    definition.source =
        Source { kind: "original".into(), reference: "Original DRC courtyard geometry".into(), locator: "Command-built test fixture".into() };
    definition.graphics = graphics;
    definition.artwork = artwork;
    definition.silkscreen.clear();
    definition.validate().unwrap();
    definition
}

fn courtyard_rect(cx: i64, cy: i64, width_nm: i64, height_nm: i64) -> GraphicRect {
    GraphicRect { purpose: GraphicPurpose::Courtyard, center: p(cx, cy), width_nm, height_nm, rotation: 0 }
}

fn courtyard_art(geometry: Geometry) -> Artwork {
    Artwork { purpose: GraphicPurpose::Courtyard, graphic: Graphic { geometry, stroke_nm: 50_000, fill: Fill::None } }
}

/// Seed two validated definitions, then assign and place one per resistor through commands.
fn board(a: FootprintDefinition, b: FootprintDefinition) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let board = session.project().unwrap().board_target().unwrap().board;
    session.apply(INITIALIZE_BOARD_SETUP, &json!({"board":board,"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    for x in [0, 20_000_000] {
        session.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let components: Vec<_> = session.project().unwrap().symbols().iter().map(|symbol| format!("component-{}", symbol.component)).collect();
    for ((component, definition), x) in components.iter().zip([&a, &b]).zip([0, 50_000_000]) {
        // Assignment retains only used definitions, so each is seeded just before its own assignment.
        let mut project = session.project().unwrap().clone();
        Arc::make_mut(&mut project.board).definitions.insert(definition.key(), Arc::new(definition.clone()));
        session.open_snapshot(Arc::new(project));
        session
            .apply(
                ASSIGN_FOOTPRINT,
                &json!({"board":board,"component":component,"library":definition.library,"definition":definition.id,
                    "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()}),
            )
            .unwrap();
        session
            .apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"board":board,"component":component,"x_nm":x,"y_nm":0,"expected_revision":session.revision()}))
            .unwrap();
    }
    session
}

/// Place footprint `index` (in placement order) through the shared transform command.
fn pose(session: &mut Session, index: usize, x_nm: i64, y_nm: i64, rotation: u16, bottom: bool) {
    let footprint = view(session).footprints().nth(index).unwrap().id;
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":footprint,"x_nm":x_nm,"y_nm":y_nm,"rotation":rotation,
                "side":if bottom {"back"} else {"front"},"expected_revision":session.revision()}),
        )
        .unwrap();
}

fn courtyard_findings(session: &Session) -> Vec<Violation> {
    let report: Report = view(session).check_design(Rules::default());
    report.violations.into_iter().filter(|violation| violation.kind == Kind::CourtyardOverlap).collect()
}

/// The same pair's bounds-only decision, the former behavior used as a contrast.
fn bounds_overlap_now(session: &Session) -> bool {
    let courtyards: Vec<_> = view(session).footprints().filter_map(|f| Courtyard::capture(&f)).collect();
    bounds_overlap(courtyards[0].bounds, courtyards[1].bounds)
}

/// Thin bars rotated by 45° sit side by side with overlapping bounds and no finding; turning
/// one across the other reports an overlap at their crossing, identifying both footprints.
#[test]
fn rotated_bars_are_decided_by_shape_rather_than_bounds() {
    let bar = || definition("bar", vec![courtyard_rect(0, 0, 4_000_000, 1_000_000)], Vec::new());
    let mut s = board(bar(), bar());
    pose(&mut s, 0, 0, 0, 45, false);
    pose(&mut s, 1, 1_500_000, 1_500_000, 45, false);
    assert!(bounds_overlap_now(&s), "the old bounds check would report this pair");
    assert!(courtyard_findings(&s).is_empty());
    pose(&mut s, 1, 1_500_000, 1_500_000, 135, false);
    let findings = courtyard_findings(&s);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    let ids: Vec<_> = view(&s).footprints().map(|f| f.id).collect();
    assert_eq!(findings[0].items, ids);
    assert_eq!(findings[0].message, OVERLAP);
    let at = findings[0].at;
    assert!(at.x_nm.abs() < 1_500_000 && at.y_nm.abs() < 1_500_000, "marker lies in the overlap: {at:?}");
}

/// A concave closed-polyline courtyard keeps its notch free; quarter turns and flips move it.
#[test]
fn concave_courtyards_and_multiple_elements_leave_their_empty_space_free() {
    let ell = courtyard_art(Geometry::Polyline {
        points: [(0, 0), (3, 0), (3, 1), (1, 1), (1, 3), (0, 3)].map(|(x, y)| p(x * 1_000_000, y * 1_000_000)).to_vec(),
        closed: true,
    });
    let block = || definition("block", vec![courtyard_rect(0, 0, 1_000_000, 1_000_000)], Vec::new());
    let mut s = board(definition("ell", Vec::new(), vec![ell]), block());
    pose(&mut s, 1, 2_000_000, 2_000_000, 0, false);
    assert!(bounds_overlap_now(&s));
    assert!(courtyard_findings(&s).is_empty(), "the block sits in the notch");
    pose(&mut s, 1, 1_400_000, 2_000_000, 0, false);
    assert_eq!(courtyard_findings(&s).len(), 1, "the block reaches into the arm");
    pose(&mut s, 1, 1_500_000, 1_500_000, 0, false);
    assert!(courtyard_findings(&s).is_empty(), "touching the inner corner is allowed");
    // A half turn of the L about its origin leaves the original notch position empty space far away.
    pose(&mut s, 0, 0, 0, 180, false);
    assert!(courtyard_findings(&s).is_empty());
    pose(&mut s, 1, -2_000_000, -2_000_000, 0, false);
    assert!(courtyard_findings(&s).is_empty(), "the rotated notch");
    pose(&mut s, 1, -500_000, -500_000, 0, false);
    assert_eq!(courtyard_findings(&s).len(), 1, "the rotated corner block");
    pose(&mut s, 1, -500_000, -500_000, 0, true);
    assert!(courtyard_findings(&s).is_empty(), "opposite sides are not compared");
    pose(&mut s, 0, 0, 0, 180, true);
    assert!(courtyard_findings(&s).is_empty(), "the flipped L mirrors its arm away from the block");
    pose(&mut s, 1, 500_000, -500_000, 0, true);
    assert_eq!(courtyard_findings(&s).len(), 1, "both on the back, overlapping the mirrored block");

    let split = definition(
        "split",
        vec![courtyard_rect(-2_000_000, 0, 1_000_000, 3_000_000), courtyard_rect(2_000_000, 0, 1_000_000, 3_000_000)],
        Vec::new(),
    );
    let mut s = board(split, block());
    pose(&mut s, 1, 0, 0, 0, false);
    assert!(bounds_overlap_now(&s));
    assert!(courtyard_findings(&s).is_empty(), "the gap between separate elements is free");
    pose(&mut s, 1, 1_000_000, 0, 0, false);
    assert!(courtyard_findings(&s).is_empty(), "touching the second element is allowed");
    pose(&mut s, 1, 1_100_000, 0, 0, false);
    assert_eq!(courtyard_findings(&s).len(), 1);
}

/// A circle is drawn as a polygonized curve, so it is compared by bounds and labeled as such.
#[test]
fn unsupported_geometry_is_reported_conservatively() {
    let ring = definition("ring", Vec::new(), vec![courtyard_art(Geometry::Circle { center: p(0, 0), radius_nm: 2_000_000 })]);
    let block = definition("block", vec![courtyard_rect(0, 0, 1_000_000, 1_000_000)], Vec::new());
    let mut s = board(ring, block);
    pose(&mut s, 1, 1_900_000, 1_900_000, 0, false);
    let findings = courtyard_findings(&s);
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].message, CONSERVATIVE);
    pose(&mut s, 1, 3_000_000, 0, 0, false);
    assert!(courtyard_findings(&s).is_empty(), "separate bounds");
}

/// Moves, undo/redo and archive reopen give deterministic findings without editing the project,
/// and the resumable job agrees with the synchronous checker at every batch size.
#[test]
fn findings_follow_history_and_persistence_and_the_job_agrees() {
    let bar = || definition("bar", vec![courtyard_rect(0, 0, 4_000_000, 1_000_000)], Vec::new());
    let mut s = board(bar(), bar());
    pose(&mut s, 0, 0, 0, 45, false);
    pose(&mut s, 1, 1_500_000, 1_500_000, 45, false);
    pose(&mut s, 1, 1_500_000, 1_500_000, 135, false);
    let crossing = courtyard_findings(&s);
    assert_eq!(crossing.len(), 1);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(courtyard_findings(&s).is_empty());
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(courtyard_findings(&s), crossing);

    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let expected = s.check_design(&json!({})).unwrap();
    assert_eq!(saved, encode_documents(s.snapshot().unwrap()).unwrap(), "checking edits nothing");
    assert_eq!(revision, s.revision());
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(courtyard_findings(&reopened), crossing);
    assert_eq!(s.execute(CHECK_DESIGN, &json!({})).unwrap()["design_check"], expected);
    for budget in [1, 3, 256] {
        let mut job = s.design_check_job(&json!({})).unwrap();
        while !job.step(budget) {}
        assert_eq!(json!(job.take_report().unwrap()), expected, "budget={budget}");
    }
}

/// The bounds filters never change a decision: footprints made of several small supported
/// elements agree with an exhaustive comparison of every element pair.
#[test]
fn candidate_filtering_agrees_with_exhaustive_element_pairs() {
    let mut state = 0x0275_u64;
    let mut next = |n: i64| {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % n as u64) as i64
    };
    let mut footprint = |id: usize| {
        let mut polygons = Vec::new();
        while polygons.len() < 1 + next(3) as usize {
            let (x, y) = (next(12), next(12));
            let points = if next(2) == 0 {
                vec![(x, y), (x + 1 + next(3), y), (x + 1 + next(3), y + 1 + next(3)), (x, y + 1 + next(3))]
            } else {
                vec![(x, y), (x + next(5) - 2, y + 1 + next(3)), (x + 1 + next(3), y + next(5) - 2)]
            };
            polygons.extend(Polygon::new(&points.into_iter().map(|(x, y)| p(x, y)).collect::<Vec<_>>()));
        }
        let all = polygons.iter().map(|polygon| polygon.bounds).fold(None, |all, b| join(all, Some(b))).unwrap();
        Courtyard { id: format!("footprint-{id}"), bottom: false, bounds: all, polygons, unsupported: None }
    };
    let (mut reported, mut clear) = (0, 0);
    for case in 0..5_000 {
        let (a, b) = (footprint(2 * case), footprint(2 * case + 1));
        let exhaustive = a.polygons.iter().any(|x| b.polygons.iter().any(|y| x.overlap_witness(y).is_some()));
        assert_eq!(a.overlap(&b).is_some(), exhaustive, "{a:?} {b:?}");
        if exhaustive { reported += 1 } else { clear += 1 }
    }
    assert!(reported > 500 && clear > 500, "{reported} reported, {clear} clear");
}
