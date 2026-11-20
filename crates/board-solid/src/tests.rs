#![allow(clippy::unwrap_used, clippy::expect_used)]

use pcbcraft_engine::{
    ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP,
    SET_FOOTPRINT_TRANSFORM, Session,
    authoring::{Action, Item, Kind, Primitive},
    board::{definition::CircularNpth, drawing::ADD_DRAWING},
    library::{LIBRARY_FORMAT, sources::LOAD},
    schematic::Point,
};
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

/// The command-built 50 × 30 mm test board, deliberately away from board zero, with a 3 mm via.
/// The cut-out surrounds a separate material island that must survive drill subtraction.
#[test]
fn board_mesh_preserves_bounds_and_through_voids() {
    let mut session = board(Some(1_600_000));
    add_rectangle(&mut session);
    draw(&mut session, &[[30_000_000, 8_000_000], [38_000_000, 8_000_000], [38_000_000, 14_000_000], [30_000_000, 14_000_000]], true);
    draw(&mut session, &[[33_000_000, 10_000_000], [35_000_000, 10_000_000], [35_000_000, 12_000_000], [33_000_000, 12_000_000]], true);
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
    assert_eq!(vertical_hits(&mesh, [34_000_123, 11_000_456]), [true; 2], "both caps survive on the nested material island");
    assert_eq!(vertical_hits(&mesh, [32_000_123, 11_000_456]), [false; 2], "the cut-out beside the island stays void");
    // Within the drilled radius and outside it but still within the 4 mm land.
    assert_eq!(vertical_hits(&mesh, [13_480_001, 6_000_003]), [false; 2]);
    assert_eq!(vertical_hits(&mesh, [13_520_001, 6_000_003]), [true; 2]);
    assert_eq!(vertical_hits(&mesh, [20_100_123, 21_000_456]), [true; 2]);
    assert_eq!(substrate::tessellate(&solid.body, 0).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(session.revision(), revision);
    assert_eq!(pcbcraft_engine::persistence::encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Use a non-default finished thickness and drill both offset pads of a rotated, translated header.
/// The footprint origin is solid: neither pad can be substituted with a hole at that point.
#[test]
fn board_solid_uses_finished_thickness_and_rotated_pad_drill_centres() {
    let mut session = board(Some(2_350_000));
    draw(&mut session, &[[0, 0], [16_000_000, 0], [16_000_000, 12_000_000], [0, 12_000_000]], true);
    apply(&mut session, PLACE_SYMBOL, json!({"definition":"connector","x_nm":0,"y_nm":0}));
    let symbol = session.project().unwrap().symbols().last().unwrap().id();
    let (x_nm, y_nm) = (8_000_000, 6_000_000);
    apply(&mut session, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":x_nm,"y_nm":y_nm}));
    let footprint = view(&session).footprints().next().unwrap().id;
    apply(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":footprint,"x_nm":x_nm,"y_nm":y_nm,"rotation":90}));

    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return;
    }
    let solid = solid.unwrap();
    assert_eq!(solid.thickness, Thickness::SpecifiedFinishedNm(2_350_000));
    assert_eq!(solid.body.bounding_box().unwrap().max_nm[2], 2_350_000);
    let mesh = substrate::tessellate(&solid.body, 1_000).unwrap();
    // The library centres are ±1.27 mm on local X. A quarter turn puts them on board Y.
    // Derive neither the expected centres nor the controls from the projection under test.
    assert_eq!(vertical_hits(&mesh, [8_000_000, 7_270_000]), [false; 2], "pad 1 is drilled through");
    assert_eq!(vertical_hits(&mesh, [8_000_000, 4_730_000]), [false; 2], "pad 2 is drilled through");
    assert_eq!(vertical_hits(&mesh, [x_nm, y_nm]), [true; 2], "the footprint origin is not drilled");
    assert_eq!(vertical_hits(&mesh, [8_600_000, 7_270_000]), [true; 2], "material remains outside the 0.5 mm drill radius");
}

/// Author one 3.2 mm non-plated hole at local (3 mm, 5 mm) in a footprint whose pads have no
/// drills, and place it at board (20 mm, 12 mm). Only the opening, at (23 mm, 17 mm), is cut.
#[test]
fn cuts_footprint_non_plated_holes_at_their_finished_diameter() {
    let mut editor = pcbcraft_engine::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let hole = CircularNpth { center: Point { x_nm: 3_000_000, y_nm: 5_000_000 }, diameter_nm: 3_200_000 };
    editor.execute(editor.revision(), Action::Insert { primitive: Primitive::NonPlatedHole { hole } }).unwrap();
    let item = editor.snapshot();
    let definition = item.footprint().unwrap();

    let mut session = board(None);
    draw(&mut session, &[[0, 0], [40_000_000, 0], [40_000_000, 30_000_000], [0, 30_000_000]], true);
    let manifest = json!({"format":LIBRARY_FORMAT,"id":definition.library,"name":"Synthetic NPTH fixture","license":definition.license});
    let native = item.export_native().unwrap().to_string();
    session.execute(LOAD, &json!({"source":"npth-fixture","documents":{"library.json":manifest.to_string(),"footprints/npth.json":native}})).unwrap();
    apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":definition.library,"definition":definition.id,
            "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
    apply(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":20_000_000,"y_nm":12_000_000}));

    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return;
    }
    let mesh = substrate::tessellate(&solid.unwrap().body, 1_000).unwrap();
    // Expected centre and the 1.6 mm radius come from the authored values, not the projection.
    assert_eq!(vertical_hits(&mesh, [23_000_000, 17_000_000]), [false; 2], "the hole centre is open");
    assert_eq!(vertical_hits(&mesh, [24_580_000, 17_000_000]), [false; 2], "open just inside the radius on X");
    assert_eq!(vertical_hits(&mesh, [24_620_000, 17_000_000]), [true; 2], "solid just outside the radius on X");
    assert_eq!(vertical_hits(&mesh, [23_000_000, 15_420_000]), [false; 2], "open just inside the radius on Y");
    assert_eq!(vertical_hits(&mesh, [23_000_000, 15_380_000]), [true; 2], "solid just outside the radius on Y");
    assert_eq!(vertical_hits(&mesh, [20_000_000, 12_000_000]), [true; 2], "the footprint origin is not cut");
    assert_eq!(vertical_hits(&mesh, [19_500_000, 12_000_000]), [true; 2], "undrilled pads leave no hole");
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
