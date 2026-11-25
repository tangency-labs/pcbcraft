#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use pcbcraft_engine::{
    ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP,
    SET_FOOTPRINT_TRANSFORM, Session,
    authoring::{Action, Item, Kind, Primitive},
    board::{definition::CircularNpth, drawing::ADD_DRAWING},
    library::{LIBRARY_FORMAT, sources::LOAD},
    schematic::Point,
};
use pcbcraft_geometry::{BoundingBox, ErrorCode, read_step, write_step};
use serde_json::{Value, json};

use super::*;
use crate::assembly::{AssemblyError, Binding, BoardAssembly, PlacedComponent, component_pose};

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
    // The board/1 frame: Y negated, and the body spans −thickness..0 below the finished top.
    assert_eq!(bounds.min_nm, [5_000_000, -27_000_000, -1_600_000]);
    assert_eq!(bounds.max_nm, [55_000_000, 3_000_000, 0]);
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
    let (display, thickness) = display_board(view(&session), 10_000).unwrap();
    assert_eq!(thickness, solid.thickness);
    for xy in [[12_000_001, 6_000_003], [34_000_123, 11_000_456], [32_000_123, 11_000_456], [20_100_123, 21_000_456]] {
        assert_eq!(vertical_hits(&display, xy), vertical_hits(&mesh, xy), "display at {xy:?}");
    }
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
    let bounds = solid.body.bounding_box().unwrap();
    assert_eq!((bounds.min_nm[2], bounds.max_nm[2]), (-2_350_000, 0));
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

/// [`frame_hits`] at an authored board point, which the board/1 frame puts at (x, −y).
fn vertical_hits(mesh: &substrate::BoardMesh, [x, y]: [i64; 2]) -> [bool; 2] {
    frame_hits(mesh, [x, -y])
}

/// Scan exact projected triangles against a vertical ray at a board/1 frame point. Non-degenerate
/// XY projections are the caps in a prismatic body: the top cap at Z = 0 (index 1) and the bottom
/// at Z = −thickness (index 0), at least 1.6 mm down in these fixtures.
fn frame_hits(mesh: &substrate::BoardMesh, [x, y]: [i64; 2]) -> [bool; 2] {
    let orient =
        |a: [i64; 3], b: [i64; 3], p: [i64; 3]| i128::from(b[0] - a[0]) * i128::from(p[1] - a[1]) - i128::from(b[1] - a[1]) * i128::from(p[0] - a[0]);
    let mut hits = [false; 2];
    for &triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|i| mesh.positions_nm[i as usize]);
        let sign = orient(a, b, c).signum();
        let p = [x, y, 0];
        if sign != 0 && [orient(a, b, p), orient(b, c, p), orient(c, a, p)].into_iter().all(|t| sign * t >= 0) {
            hits[usize::from(a[2] > -100_000)] = true;
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
    let bounds = solid.body.bounding_box().unwrap();
    assert_eq!((bounds.min_nm, bounds.max_nm), ([0, -3_000_000, -1_600_000], [4_000_000, 0, 0]));
}

/// An outline wholly at positive authored Y, away from board zero, with one via. In the board/1
/// frame it lies at negative Y: the drill opens at (x, −y), the mirror row about the board's
/// centre is solid, and the authored location itself has no material.
#[test]
fn board_solid_uses_the_board1_frame() {
    let mut session = board(Some(1_600_000));
    draw(&mut session, &[[2_000_000, 1_000_000], [12_000_000, 1_000_000], [12_000_000, 7_000_000], [2_000_000, 7_000_000]], true);
    apply(&mut session, "pcb.add_via", json!({"at":{"x_nm":4_000_000,"y_nm":5_000_000},"via_diameter_nm":1_200_000,"via_drill_nm":800_000}));
    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return;
    }
    let solid = solid.unwrap();
    let bounds = solid.body.bounding_box().unwrap();
    assert_eq!((bounds.min_nm, bounds.max_nm), ([2_000_000, -7_000_000, -1_600_000], [12_000_000, -1_000_000, 0]));
    let mesh = substrate::tessellate(&solid.body, 1_000).unwrap();
    assert_eq!(mesh.positions_nm.iter().map(|p| p[2]).max(), Some(0), "the finished top is at Z = 0");
    assert_eq!(frame_hits(&mesh, [4_000_000, -5_000_000]), [false; 2], "the via opens at (x, −y)");
    assert_eq!(frame_hits(&mesh, [4_000_000, -3_000_000]), [true; 2], "its mirror about the board centre is solid");
    assert_eq!(frame_hits(&mesh, [4_000_000, 5_000_000]), [false; 2], "nothing remains at the unflipped location");
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

/// A 100 mm board's kernel floor is 100 nm: a 10 nm request is refused rather than coarsened,
/// and a supported 1000 nm request still meshes.
fn hundred_mm_board_solid() -> Option<BoardSolid> {
    let mut session = board(None);
    draw(&mut session, &[[0, 0], [100_000_000, 0], [100_000_000, 20_000_000], [0, 20_000_000]], true);
    let solid = BoardSolid::build(view(&session));
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(solid.unwrap_err().code(), ErrorCode::KernelUnavailable);
        return None;
    }
    Some(solid.unwrap())
}

#[test]
fn refuses_chord_tolerance_below_the_kernel_floor() {
    let Some(solid) = hundred_mm_board_solid() else { return };
    let error = substrate::tessellate(&solid.body, 10).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams);
    assert!(error.message().contains("10 nm is below the"), "{error}");
}

#[test]
fn keeps_supported_chord_tolerance_on_a_100_mm_board() {
    let Some(solid) = hundred_mm_board_solid() else { return };
    assert!(!substrate::tessellate(&solid.body, 1_000).unwrap().triangles.is_empty());
}

/// A drill that joins another opening and touches the board edge uses the exact solid fallback.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-kernel"))]
#[test]
fn display_board_keeps_edge_and_overlapping_drill_cuts() {
    let outline = OutlinePath { points: [[0, 0], [10_000_000, 0], [10_000_000, 10_000_000], [0, 10_000_000]].map(BoardPoint).to_vec(), closed: true };
    let drills = [
        RoundDrill { center: BoardPoint([400_000, 5_000_000]), diameter_nm: 1_200_000 },
        RoundDrill { center: BoardPoint([1_000_000, 5_000_000]), diameter_nm: 1_200_000 },
    ];
    let mesh = substrate::display_board(&[outline], &drills, 1_600_000, 10_000).unwrap();
    assert_eq!(frame_hits(&mesh, [200_000, 5_000_000]), [false; 2]);
    assert_eq!(frame_hits(&mesh, [1_450_000, 5_000_000]), [false; 2]);
    assert_eq!(frame_hits(&mesh, [1_800_000, 5_000_000]), [true; 2]);
}

/// A dense board keeps its drill mouths open without a large solid Boolean.
#[test]
fn display_board_keeps_dense_interior_drills_open() {
    let outline = OutlinePath { points: [[0, 0], [33_000_000, 0], [33_000_000, 21_000_000], [0, 21_000_000]].map(BoardPoint).to_vec(), closed: true };
    let drills: Vec<_> = (1..=32)
        .flat_map(|x| (1..=20).map(move |y| RoundDrill { center: BoardPoint([x * 1_000_000, y * 1_000_000]), diameter_nm: 220_000 }))
        .collect();
    let mesh = substrate::display_board(&[outline], &drills, 1_600_000, 10_000).unwrap();
    for drill in [drills[0], drills[319], drills[639]] {
        let [x, y] = drill.center.0;
        assert_eq!(frame_hits(&mesh, [x, y]), [false; 2]);
        assert_eq!(frame_hits(&mesh, [x + 200_000, y]), [true; 2]);
    }
}

/// The kernel-free build has no solids: a model cannot even be made.
fn no_kernel() -> bool {
    if cfg!(all(target_arch = "wasm32", not(feature = "wasm-kernel"))) {
        assert_eq!(Body::cuboid("model", [0; 3], [1; 3]).unwrap_err().code(), ErrorCode::KernelUnavailable);
        return true;
    }
    false
}

/// A 40 × 20 mm board with a declared 1.6 mm finished thickness and two 0603 resistors at board
/// y = 6 mm: the first on the top at x = 10 mm turned a quarter turn, the second on the bottom
/// at x = 30 mm unturned. SMD pads, so moving a part leaves the board solid unchanged.
fn two_sided_board() -> (Session, [String; 2]) {
    let mut session = board(Some(1_600_000));
    draw(&mut session, &[[0, 0], [40_000_000, 0], [40_000_000, 20_000_000], [0, 20_000_000]], true);
    let mut ids = Vec::new();
    for (x_nm, rotation, side) in [(10_000_000, 90, "front"), (30_000_000, 0, "back")] {
        apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x_nm,"y_nm":0}));
        let symbol = session.project().unwrap().symbols().last().unwrap().id();
        apply(
            &mut session,
            PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0603","mapping":{"1":"1","2":"2"},"x_nm":x_nm,"y_nm":6_000_000}),
        );
        let footprint = view(&session).footprints().last().unwrap().id;
        apply(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":footprint,"x_nm":x_nm,"y_nm":6_000_000,"rotation":rotation,"side":side}));
        ids.push(footprint);
    }
    (session, ids.try_into().unwrap())
}

/// Two models, each 3 mm along X from −1 mm and 2 mm along Y from −0.5 mm, so a half turn about
/// Y can be told from one about X or a mirror. The first is 1 mm tall, the second 2 mm.
fn models() -> [Body; 2] {
    [
        Body::cuboid("model-a", [-1_000_000, -500_000, 0], [3_000_000, 2_000_000, 1_000_000]).unwrap(),
        Body::cuboid("model-b", [-1_000_000, -500_000, 0], [3_000_000, 2_000_000, 2_000_000]).unwrap(),
    ]
}

/// The first model raised 0.2 mm by its alignment on the top part; the second on the bottom part.
fn bindings([top, bottom]: &[String; 2]) -> BTreeMap<String, Binding> {
    BTreeMap::from([
        (top.clone(), Binding { model: 0, alignment: Transform::translation([0, 0, 200_000]) }),
        (bottom.clone(), Binding { model: 1, alignment: Transform::IDENTITY }),
    ])
}

/// Each placement read back from the assembly, by body name, with the placed body's box.
fn read_back(step: &[u8]) -> BTreeMap<String, (Transform, BoundingBox)> {
    let read = read_step(step).unwrap();
    assert!(read.refused.is_empty());
    let placed = read.placed_bodies().unwrap();
    assert_eq!(placed.len(), read.placements.len());
    let out: BTreeMap<_, _> = read
        .placements
        .iter()
        .zip(&placed)
        .map(|(placement, body)| (read.bodies[placement.body].name().to_owned(), (placement.transform, body.bounding_box().unwrap())))
        .collect();
    assert_eq!(out.len(), read.placements.len(), "each body is placed once");
    out
}

fn assert_near(actual: &Transform, expected: &Transform) {
    for (a, e) in actual.axes.iter().flatten().zip(expected.axes.iter().flatten()) {
        assert!((a - e).abs() < 1e-9, "{actual:?} != {expected:?}");
    }
    for (a, e) in actual.translation_nm.iter().zip(&expected.translation_nm) {
        assert!(a.abs_diff(*e) <= 1, "{actual:?} != {expected:?}");
    }
}

fn assert_box(actual: &BoundingBox, min_nm: [i64; 3], max_nm: [i64; 3]) {
    assert!(actual.max_deviation_nm(&BoundingBox { min_nm, max_nm }) <= 10, "{actual:?} != {min_nm:?}..{max_nm:?}");
}

/// The board solid and both models are placed in one assembly at their component poses. The
/// expected poses and boxes are worked by hand from the placements, not from the code under test:
/// the top part turns a quarter turn about Z and rises by its alignment; the bottom part is
/// turned over about Y (X and Z reversed, Y kept) and hangs below the 1.6 mm board.
#[test]
fn exports_the_board_and_both_models_at_their_placed_transforms() {
    if no_kernel() {
        return;
    }
    let (session, ids) = two_sided_board();
    let [a, b] = models();
    let assembly = BoardAssembly::build(view(&session), "two-sided", &[&a, &b], &bindings(&ids)).unwrap();
    assert_eq!(assembly.thickness_nm, 1_600_000);
    assert!(assembly.unbound.is_empty());
    let top = Transform { axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], translation_nm: [10_000_000, -6_000_000, 200_000] };
    let bottom = Transform { axes: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], translation_nm: [30_000_000, -6_000_000, -1_600_000] };
    assert_eq!(
        assembly.components,
        [
            PlacedComponent { footprint: ids[0].clone(), model: 0, transform: top },
            PlacedComponent { footprint: ids[1].clone(), model: 1, transform: bottom },
        ]
    );

    let read = read_back(&assembly.step);
    assert_eq!(read.len(), 3);
    let (board, board_box) = &read["Board substrate"];
    assert_near(board, &Transform::IDENTITY);
    assert_box(board_box, [0, -20_000_000, -1_600_000], [40_000_000, 0, 0]);
    let (placed, placed_box) = &read["model-a"];
    assert_near(placed, &top);
    assert_box(placed_box, [8_500_000, -7_000_000, 200_000], [10_500_000, -4_000_000, 1_200_000]);
    let (placed, placed_box) = &read["model-b"];
    assert_near(placed, &bottom);
    assert_box(placed_box, [28_000_000, -6_500_000, -3_600_000], [31_000_000, -4_500_000, -1_600_000]);
}

/// Exporting the same board again gives the same bytes. Moving the top part 2 mm and exporting
/// again changes that part's transform and nothing else: the board and the bottom part read back
/// as before.
#[test]
fn refreshing_after_a_move_changes_only_that_parts_transform() {
    if no_kernel() {
        return;
    }
    let (mut session, ids) = two_sided_board();
    let [a, b] = models();
    let bindings = bindings(&ids);
    let before = BoardAssembly::build(view(&session), "two-sided", &[&a, &b], &bindings).unwrap();
    let again = BoardAssembly::build(view(&session), "two-sided", &[&a, &b], &bindings).unwrap();
    assert_eq!(before.step, again.step, "an unchanged board writes the same bytes");

    apply(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":ids[0],"x_nm":12_000_000,"y_nm":6_000_000,"rotation":90}));
    let after = BoardAssembly::build(view(&session), "two-sided", &[&a, &b], &bindings).unwrap();
    assert_ne!(before.step, after.step);
    assert_eq!(after.components[0].transform.translation_nm, [12_000_000, -6_000_000, 200_000]);
    assert_eq!(after.components[0].transform.axes, before.components[0].transform.axes);
    assert_eq!(after.components[1], before.components[1]);

    let (old, new) = (read_back(&before.step), read_back(&after.step));
    assert_eq!(old.keys().collect::<Vec<_>>(), new.keys().collect::<Vec<_>>());
    for (name, (transform, bounds)) in &old {
        let (moved, moved_bounds) = &new[name];
        if name == "model-a" {
            assert_eq!(moved.translation_nm[0] - transform.translation_nm[0], 2_000_000);
            assert_eq!(moved_bounds.min_nm[0] - bounds.min_nm[0], 2_000_000);
        } else {
            assert_near(moved, transform);
            assert_eq!(moved_bounds, bounds, "{name} is unchanged");
        }
    }
}

/// A footprint without a binding is listed and gets no body; the export still completes.
#[test]
fn lists_unbound_footprints_without_a_body() {
    if no_kernel() {
        return;
    }
    let (session, ids) = two_sided_board();
    let [a, _] = models();
    let only_top = BTreeMap::from([(ids[0].clone(), Binding { model: 0, alignment: Transform::IDENTITY })]);
    let assembly = BoardAssembly::build(view(&session), "two-sided", &[&a], &only_top).unwrap();
    assert_eq!(assembly.unbound, [ids[1].clone()]);
    assert_eq!(read_back(&assembly.step).keys().collect::<Vec<_>>(), ["Board substrate", "model-a"]);
}

/// An unused model that STEP export would refuse does not block export or leak into the assembly.
#[test]
fn ignores_an_unused_model_that_step_cannot_export() {
    if no_kernel() {
        return;
    }
    let (session, ids) = two_sided_board();
    let [valid, unused] = models();
    // STEP import accepts a tab encoded in a B-rep name; export refuses control characters.
    let step = String::from_utf8(write_step("fixture", &[unused]).unwrap()).unwrap();
    let step = step.replacen("MANIFOLD_SOLID_BREP('model-b'", r"MANIFOLD_SOLID_BREP('model-\X2\0009\X0\b'", 1);
    let read = read_step(step.as_bytes()).unwrap();
    assert!(read.refused.is_empty());
    assert_eq!(read.bodies[0].name(), "model-\tb");
    let non_exportable = &read.bodies[0];
    let models = [non_exportable, &valid];

    let top = |model| BTreeMap::from([(ids[0].clone(), Binding { model, alignment: Transform::IDENTITY })]);
    let baseline = BoardAssembly::build(view(&session), "two-sided", &[&valid], &top(0)).unwrap();
    let assembly = BoardAssembly::build(view(&session), "two-sided", &models, &top(1)).unwrap();
    assert_eq!(assembly.components[0].model, 1);
    assert_eq!(assembly.step, baseline.step);
    assert_eq!(read_back(&assembly.step).keys().collect::<Vec<_>>(), ["Board substrate", "model-a"]);
    let Err(AssemblyError::Geometry(error)) = BoardAssembly::build(view(&session), "two-sided", &models, &top(0)) else {
        panic!("the same model must be refused when bound");
    };
    assert_eq!(error.code(), ErrorCode::InvalidParams);
}

/// No nominal thickness stands in for an undeclared one, and a binding must name a placed
/// footprint and a supplied model. Each refusal is reported before any geometry is made.
#[test]
fn refuses_undeclared_thickness_and_stale_bindings() {
    let mut session = board(None);
    draw(&mut session, &[[0, 0], [40_000_000, 0], [40_000_000, 20_000_000], [0, 20_000_000]], true);
    let none = BTreeMap::new();
    assert_eq!(BoardAssembly::build(view(&session), "board", &[], &none).unwrap_err(), AssemblyError::NoFinishedThickness);

    let (session, ids) = two_sided_board();
    let stale = BTreeMap::from([("footprint-99".to_owned(), Binding { model: 0, alignment: Transform::IDENTITY })]);
    assert_eq!(BoardAssembly::build(view(&session), "board", &[], &stale).unwrap_err(), AssemblyError::UnknownFootprint("footprint-99".into()));
    let missing = BTreeMap::from([(ids[0].clone(), Binding { model: 0, alignment: Transform::IDENTITY })]);
    assert_eq!(
        BoardAssembly::build(view(&session), "board", &[], &missing).unwrap_err(),
        AssemblyError::UnknownModel { footprint: ids[0].clone(), model: 0 }
    );
}

/// Quarter turns are exact, and an odd angle composes with a bottom-side turn-over in the
/// documented order: the alignment first, then the side, then the turn and the position.
#[test]
fn component_pose_composes_in_the_documented_order() {
    let at = Point { x_nm: 5_000_000, y_nm: 2_000_000 };
    let offset = Transform::translation([1_000_000, 0, 0]);
    // Bottom, 30°: the offset +1 mm on X is reversed by the turn-over, then turned by 30°.
    let pose = component_pose(at, 30, true, 1_000_000, &offset);
    let (s, c) = 30f64.to_radians().sin_cos();
    let expected = Transform {
        axes: [[-c, -s, 0.0], [-s, c, 0.0], [0.0, 0.0, -1.0]],
        translation_nm: [5_000_000 - (c * 1_000_000.0).round() as i64, -2_000_000 - (s * 1_000_000.0).round() as i64, -1_000_000],
    };
    assert_near(&pose, &expected);
    let quarter = component_pose(at, 270, false, 1_000_000, &Transform::IDENTITY);
    assert_eq!(quarter.axes, [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    assert_eq!(quarter.translation_nm, [5_000_000, -2_000_000, 0]);
}
