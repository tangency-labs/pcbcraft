//! Original command-built boards exercise the resolved production pad geometry.
use super::*;
use crate::{
    ADD_TRACKS, ADD_VIA, ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM,
    board::{
        definition::PadShape,
        track::tests::{copper_layers, view},
    },
};

fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Seed a validated original definition, then assign and place it through normal commands.
/// The first pad is the contour under test; the second is a small circular probe or a remote pad.
fn fixture(shape: PadShape, width_nm: i64, height_nm: i64, probe: Point, probe_width: i64) -> Session {
    let mut definition = (*super::super::definition::starter("resistor-0402").unwrap()).clone();
    definition.library = "drc-contours".into();
    definition.id = "two-pads".into();
    definition.source = super::super::definition::Source {
        kind: "original".into(),
        reference: "Original DRC regression geometry".into(),
        locator: "Command-built test fixture".into(),
    };
    definition.graphics.clear();
    definition.artwork.clear();
    definition.silkscreen.clear();
    let pad = &mut definition.pads[0];
    pad.center = p(0, 0);
    pad.width_nm = width_nm;
    pad.height_nm = height_nm;
    pad.shape = shape;
    pad.corner_radius_nm = if shape == PadShape::RoundedRectangle { 150_000 } else { 0 };
    let pad = &mut definition.pads[1];
    pad.center = probe;
    pad.width_nm = probe_width;
    pad.height_nm = probe_width;
    pad.shape = PadShape::Circle;
    pad.corner_radius_nm = 0;
    definition.validate().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let board = session.project().unwrap().board_target().unwrap().board;
    session.apply(INITIALIZE_BOARD_SETUP, &json!({"board":board,"copper_layers":2,"expected_revision":session.revision()})).unwrap();
    session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    // Seed only a validated native definition; all placements and copper use production commands.
    let mut project = session.project().unwrap().clone();
    std::sync::Arc::make_mut(&mut project.board).definitions.insert(definition.key(), std::sync::Arc::new(definition.clone()));
    session.open_snapshot(std::sync::Arc::new(project));
    let component = format!("component-{}", session.project().unwrap().symbols()[0].component);
    session
        .apply(
            ASSIGN_FOOTPRINT,
            &json!({"board":board,"component":component,"library":definition.library,"definition":definition.id,
        "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    session
        .apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"board":board,"component":component,"x_nm":0,"y_nm":0,"expected_revision":session.revision()}))
        .unwrap();
    session
}

/// Transform the whole footprint so the test traverses the same rotation/side resolution as the editor.
fn pose(session: &mut Session, rotation: u16, bottom: bool) {
    let footprint = view(session).footprints().next().unwrap().id;
    let board = view(session).target.board;
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":board,"footprint":footprint,"x_nm":0,"y_nm":0,
        "rotation":rotation,"side":if bottom {"back"} else {"front"},"expected_revision":session.revision()}),
        )
        .unwrap();
}

/// Add a track or via through production commands, retaining validated board and layer identities.
fn obstacle(session: &mut Session, kind: &str, at: Point, end: Point, width: i64, layer: Uuid) {
    let board = view(session).target.board;
    match kind {
        "track" => {
            session
                .apply(
                    ADD_TRACKS,
                    &json!({"board":board,"layer":layer,"width_nm":width,
            "points":[at,end],"expected_revision":session.revision()}),
                )
                .unwrap();
        }
        "via" => {
            session
                .apply(
                    ADD_VIA,
                    &json!({"board":board,"at":at,"via_diameter_nm":width,
            "via_drill_nm":50_000,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
        "pad" => {}
        _ => panic!("unknown test obstacle"),
    }
}

/// Assert only copper clearance, leaving unrelated outline/minimum-size diagnostics visible.
fn clearance(session: &Session, required: i64) -> Vec<Violation> {
    view(session)
        .check_design(Rules { clearance_nm: required, ..Rules::default() })
        .violations
        .into_iter()
        .filter(|v| v.kind == Kind::Clearance)
        .collect()
}

/// All supported rounded shapes clear probes in their enclosing corners, on rotated and flipped pads.
#[test]
fn rounded_corners_do_not_create_false_pad_track_or_via_clearance_findings() {
    for (shape, width, height) in
        [(PadShape::Circle, 600_000, 600_000), (PadShape::Oval, 1_000_000, 600_000), (PadShape::RoundedRectangle, 1_000_000, 600_000)]
    {
        for (rotation, bottom) in [(0, false), (37, false), (90, true)] {
            for kind in ["pad", "track", "via"] {
                let probe = if kind == "pad" { p(width / 2, height / 2) } else { p(10_000_000, 0) };
                let mut session = fixture(shape, width, height, probe, 10_000);
                pose(&mut session, rotation, bottom);
                let corner = view(&session).footprints().next().unwrap().pads[0].copper.unwrap()[2];
                let end = p(corner.x_nm + corner.x_nm / 10, corner.y_nm + corner.y_nm / 10);
                let layer = copper_layers(&session)[usize::from(bottom)];
                obstacle(&mut session, kind, corner, end, if kind == "via" { 60_000 } else { 10_000 }, layer);
                assert!(clearance(&session, 10_000).is_empty(), "{shape:?}, {rotation}, {bottom}, {kind}: {:?}", clearance(&session, 10_000));
            }
        }
    }
}

/// Equality passes while a one-nanometer deficit fails for every shape and copper pairing.
/// Odd circle/oval diameters and odd probe widths retain their half-nanometer radii.
#[test]
fn pad_clearance_boundaries_preserve_fractional_radii_and_rectangles() {
    for shape in [PadShape::Circle, PadShape::Oval, PadShape::RoundedRectangle, PadShape::Rectangle] {
        let odd = matches!(shape, PadShape::Circle | PadShape::Oval);
        let height = 600_000 + i64::from(odd);
        let width = if shape == PadShape::Circle { height } else { 1_000_000 + i64::from(odd) };
        for kind in ["pad", "track", "via"] {
            let diameter = if kind == "via" { 60_000 } else { 10_000 } + i64::from(odd);
            for (delta, rotation) in [-1, 0, 1].into_iter().flat_map(|delta| [0, 90].map(|rotation| (delta, rotation))) {
                let at = p(0, (height + diameter) / 2 + 20_000 + delta);
                let probe = if kind == "pad" { at } else { p(10_000_000, 0) };
                let mut session = fixture(shape, width, height, probe, diameter);
                pose(&mut session, rotation, false);
                let layer = copper_layers(&session)[0];
                let (x, y) = crate::board::rotate(at.x_nm, at.y_nm, rotation);
                let (ex, ey) = crate::board::rotate(at.x_nm + 10_000, at.y_nm, rotation);
                obstacle(&mut session, kind, p(x, y), p(ex, ey), diameter, layer);
                assert_eq!(clearance(&session, 20_000).len(), usize::from(delta < 0), "{shape:?}, {kind}, {delta}, {rotation}");
            }
        }
    }
}

/// Surface copper is checked only on its resolved side, including after flipping the footprint.
#[test]
fn rounded_pad_clearance_follows_resolved_copper_layers() {
    for bottom in [false, true] {
        for same_layer in [false, true] {
            let mut session = fixture(PadShape::Oval, 1_000_000, 600_000, p(10_000_000, 0), 10_000);
            pose(&mut session, 90, bottom);
            let layer = copper_layers(&session)[usize::from(bottom ^ !same_layer)];
            obstacle(&mut session, "track", p(315_000, 0), p(315_000, 100_000), 10_000, layer);
            assert_eq!(clearance(&session, 20_000).len(), usize::from(same_layer), "{bottom}, {same_layer}");
        }
    }
}

/// A diagonal outline may cut an enclosing box's empty corner while clearing the rounded copper.
/// Edge clearance uses the same contour gap as copper-to-copper checks.
#[test]
fn board_edges_use_the_pad_contour_instead_of_empty_bounding_corners() {
    for (shape, width, limit) in
        [(PadShape::Circle, 600_000, 450_000), (PadShape::Oval, 1_000_000, 650_000), (PadShape::RoundedRectangle, 1_000_000, 738_000)]
    {
        let mut session = fixture(shape, width, 600_000, p(-1_000_000, -1_000_000), 10_000);
        let layer = view(&session).layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
        session
            .apply(
                crate::ADD_DRAWING,
                &json!({"board":view(&session).target.board,"layer":layer,
            "points":[p(limit,0),p(0,limit),p(-2_000_000,limit),p(-2_000_000,-2_000_000),p(limit,-2_000_000)],
            "closed":true,"expected_revision":session.revision()}),
            )
            .unwrap();
        let report = view(&session).check_design(Rules { edge_clearance_nm: 10_000, ..Rules::default() });
        assert!(!report.violations.iter().any(|v| matches!(v.kind, Kind::OutsideBoard | Kind::EdgeClearance)), "{shape:?}: {report:?}");
        let report = view(&session).check_design(Rules { edge_clearance_nm: 20_000, ..Rules::default() });
        assert_eq!(report.counts.get(&Kind::EdgeClearance), Some(&1), "{shape:?}: {report:?}");
    }
}

/// A notch tip can enter the copper while leaving every core anchor inside the board.
/// Zero required clearance must still report penetration; exact tangency remains allowed.
#[test]
fn zero_edge_clearance_still_reports_notches_entering_pad_copper() {
    for shape in [PadShape::Circle, PadShape::Oval, PadShape::RoundedRectangle, PadShape::Rectangle] {
        let width = if shape == PadShape::Circle { 600_000 } else { 1_000_000 };
        for tip in [width / 2 + 1, width / 2, width / 2 - 1, 0] {
            let mut session = fixture(shape, width, 600_000, p(-1_000_000, -1_000_000), 10_000);
            let points = [
                p(-2_000_000, -2_000_000),
                p(2_000_000, -2_000_000),
                p(2_000_000, 1_140_000),
                p(tip, 0),
                p(2_000_000, 1_260_000),
                p(2_000_000, 2_000_000),
                p(-2_000_000, 2_000_000),
            ];
            if shape == PadShape::RoundedRectangle && tip == 0 {
                let contour = lands(view(&session))[0].contour();
                assert!(contour.core.iter().all(|point| in_polygon(*point, &points)), "The reported case bypasses the core-anchor check");
            }
            let layer = view(&session).layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
            session
                .apply(
                    crate::ADD_DRAWING,
                    &json!({"board":view(&session).target.board,"layer":layer,
                "points":points,"closed":true,"expected_revision":session.revision()}),
                )
                .unwrap();
            let report = view(&session).check_design(Rules { edge_clearance_nm: 0, ..Rules::default() });
            let boundary: Vec<_> = report.violations.iter().filter(|v| matches!(v.kind, Kind::OutsideBoard | Kind::EdgeClearance)).collect();
            assert_eq!(boundary.len(), usize::from(tip < width / 2), "{shape:?}, tip {tip}: {report:?}");
        }
    }
}

/// Clipping detects entry through opposite vertices without treating a coincident edge as
/// penetration. Resolved rotation and bottom-side mirroring may reverse the core winding.
#[test]
fn rectangular_core_penetration_distinguishes_interior_from_boundary_contact() {
    for (rotation, bottom) in [(0, false), (37, false), (90, true)] {
        let mut session = fixture(PadShape::Rectangle, 1_000_000, 600_000, p(10_000_000, 0), 10_000);
        pose(&mut session, rotation, bottom);
        let contour = lands(view(&session))[0].contour();
        let c = contour.core;
        let start = p(2 * c[0].x_nm - c[2].x_nm, 2 * c[0].y_nm - c[2].y_nm);
        let end = p(2 * c[2].x_nm - c[0].x_nm, 2 * c[2].y_nm - c[0].y_nm);
        assert!(contour.penetrates_segment(start, end));
        assert!(contour.penetrates_segment(end, start));
        assert!(contour.penetrates_segment(c[0], c[2]));
        assert!(contour.penetrates_segment(p(0, 0), p(0, 0)));
        assert!(!contour.penetrates_segment(c[0], c[1]));
        assert!(!contour.penetrates_segment(c[0], c[0]));
    }
}
