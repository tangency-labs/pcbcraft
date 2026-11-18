#![allow(clippy::unwrap_used, clippy::expect_used)]

use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP, Session, board::drawing::ADD_DRAWING};
use pcbcraft_geometry::ErrorCode;
use serde_json::{Value, json};

use super::*;

/// New two-layer board, populated only by shared commands. The minimum is deliberately
/// different from the nominal finished declaration, to catch selecting the check value.
fn board(thickness_nm: Option<i64>) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    apply(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    if let Some(nm) = thickness_nm {
        let mut setup = session.inspect()["project"]["board"]["setup"].clone();
        setup["stackup"]["finished_thickness_nm"] = json!(nm);
        setup["stackup"]["declared_minimum_thickness_nm"] = json!(nm - 100_000);
        apply(&mut session, SET_BOARD_SETUP, json!({"setup":setup}));
    }
    session
}

fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

fn view(session: &Session) -> BoardView<'_> {
    let project = session.project().unwrap();
    project.board_view(project.board_target().unwrap()).unwrap()
}

fn draw(session: &mut Session, points: &[[i64; 2]], closed: bool) {
    let layer = view(session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let points: Vec<_> = points.iter().map(|[x, y]| json!({"x_nm": x, "y_nm": y})).collect();
    apply(session, ADD_DRAWING, json!({"layer":layer,"points":points,"closed":closed,"stroke_nm":250_000}));
}

fn add_rectangle(session: &mut Session) {
    draw(session, &[[5_000_000, -3_000_000], [55_000_000, -3_000_000], [55_000_000, 27_000_000], [5_000_000, 27_000_000]], true);
}

/// The command-built 50 × 30 mm test board, deliberately away from board zero. This
/// PCB document format has plated round drills only: use a 3 mm via, not a mock NPTH.
#[test]
fn board_mesh_preserves_bounds_and_through_voids() {
    let mut session = board(Some(1_600_000));
    add_rectangle(&mut session);
    draw(&mut session, &[[30_000_000, 8_000_000], [38_000_000, 8_000_000], [38_000_000, 14_000_000], [30_000_000, 14_000_000]], true);
    apply(&mut session, "pcb.add_via", json!({"at":{"x_nm":12_000_001,"y_nm":6_000_003},"via_diameter_nm":4_000_000,"via_drill_nm":3_000_000}));
    let revision = session.revision();
    let snapshot = session.snapshot().unwrap();
    let before = pcbcraft_engine::persistence::encode_documents(snapshot.clone()).unwrap();
    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return;
    }
    let solid = solid.unwrap();
    assert_eq!(solid.target, view(&session).target);
    assert_eq!(solid.thickness, Thickness::SpecifiedFinishedNm(1_600_000));
    let bounds = solid.body.bounding_box().unwrap();
    assert_eq!(bounds.min_nm, [5_000_000, -3_000_000, 0]);
    assert_eq!(bounds.max_nm, [55_000_000, 27_000_000, 1_600_000]);
    let mesh = substrate::tessellate(&solid.body, 1_000).unwrap();
    for i in 0..3 {
        let lo = mesh.positions_nm.iter().map(|p| p[i]).min().unwrap();
        let hi = mesh.positions_nm.iter().map(|p| p[i]).max().unwrap();
        assert!(lo.abs_diff(bounds.min_nm[i]) <= 1_000);
        assert!(hi.abs_diff(bounds.max_nm[i]) <= 1_000);
    }
    assert_eq!(vertical_hits(&mesh, [12_000_001, 6_000_003]), [false; 2], "the centre of the 3 mm drill is not solid");
    assert_eq!(vertical_hits(&mesh, [34_000_123, 11_000_456]), [false; 2]);
    // Within the drilled radius and outside it but still within the 4 mm land.
    assert_eq!(vertical_hits(&mesh, [13_480_001, 6_000_003]), [false; 2]);
    assert_eq!(vertical_hits(&mesh, [13_520_001, 6_000_003]), [true; 2]);
    assert_eq!(vertical_hits(&mesh, [20_100_123, 21_000_456]), [true; 2]);
    assert_eq!(substrate::tessellate(&solid.body, 0).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(session.revision(), revision);
    assert_eq!(pcbcraft_engine::persistence::encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Scan exact projected triangles against a ray from negative Z. Non-degenerate
/// XY projections are the caps in a prismatic body, and their Z identifies the end.
fn vertical_hits(mesh: &substrate::BoardMesh, [x, y]: [i64; 2]) -> [bool; 2] {
    let orient =
        |a: [i64; 3], b: [i64; 3], p: [i64; 3]| i128::from(b[0] - a[0]) * i128::from(p[1] - a[1]) - i128::from(b[1] - a[1]) * i128::from(p[0] - a[0]);
    let mut hits = [false; 2];
    for &triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|i| mesh.positions_nm[i as usize]);
        let sign = orient(a, b, c).signum();
        let p = [x, y, 0];
        if sign != 0 && [orient(a, b, p), orient(b, c, p), orient(c, a, p)].into_iter().all(|t| sign * t >= 0) {
            hits[usize::from(a[2] > 0)] = true;
        }
    }
    hits
}

/// Absent/open drawings refuse instead of guessing; exact unbranched segments close
/// a rectangle, and an undeclared finished thickness is explicitly nominal.
#[test]
fn closes_exact_paths_and_labels_the_nominal_fallback() {
    let mut session = board(None);
    assert_eq!(BoardSolid::build(view(&session)).unwrap_err().code(), ErrorCode::InvalidParams);
    draw(&mut session, &[[0, 0], [4_000_000, 0], [4_000_000, 3_000_000]], false);
    let error = BoardSolid::build(view(&session)).unwrap_err();
    assert!(error.message().contains("unbranched"), "{error}");
    draw(&mut session, &[[0, 0], [0, 3_000_000], [4_000_000, 3_000_000]], false);
    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return;
    }
    let solid = solid.unwrap();
    // The documented two-layer default is a nominal value, not a guarantee.
    assert_eq!(solid.thickness, Thickness::NominalFallback);
    assert_eq!(solid.body.bounding_box().unwrap().max_nm, [4_000_000, 3_000_000, 1_600_000]);
}

/// A crossing outline is refused on all builds rather than silently masking a region.
#[test]
fn refuses_crossing_board_outline() {
    let mut session = board(None);
    draw(&mut session, &[[0, 0], [4_000_000, 3_000_000], [4_000_000, 0], [0, 3_000_000]], true);
    let error = BoardSolid::build(view(&session)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams);
    assert!(error.message().contains("cross or touch"), "{error}");
}
