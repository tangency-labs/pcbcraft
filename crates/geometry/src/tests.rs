use super::*;
use crate::guard::guard;

/// The round-trip tolerance for bounding boxes: one micrometre.
const ROUND_TRIP_TOLERANCE_NM: u64 = 1_000;

fn block() -> (Body, BoundingBox) {
    let origin = [-2_000_000, 3_500_000, 0];
    let size = [12_500_000, 8_000_000, 1_600_000];
    let expected = BoundingBox { min_nm: origin, max_nm: [10_500_000, 11_500_000, 1_600_000] };
    (Body::cuboid("Block", origin, size).unwrap(), expected)
}

#[test]
fn error_codes_keep_their_names() {
    let names: Vec<&str> = [
        ErrorCode::InvalidParams,
        ErrorCode::ModelUnreadable,
        ErrorCode::ModelUnsupportedEntity,
        ErrorCode::OperationFailed,
        ErrorCode::KernelUnavailable,
        ErrorCode::KernelInternal,
    ]
    .into_iter()
    .map(ErrorCode::as_str)
    .collect();
    assert_eq!(
        names,
        ["invalid_params", "model_unreadable", "model_unsupported_entity", "operation_failed", "kernel_unavailable", "kernel_internal"]
    );
}

#[test]
fn cuboid_round_trips_through_step_within_one_micrometre() {
    let (body, expected) = block();
    assert!(body.bounding_box().unwrap().max_deviation_nm(&expected) <= ROUND_TRIP_TOLERANCE_NM);

    let bytes = write_step("fixture", &[body]).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with("ISO-10303-21;"));
    assert!(text.contains("AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF"));
    assert!(text.contains("MANIFOLD_SOLID_BREP('Block'"));

    let read = read_step(&bytes).unwrap();
    assert!(read.refused.is_empty(), "{:?}", read.refused);
    assert_eq!(read.bodies.len(), 1);
    assert_eq!(read.bodies[0].name(), "Block");
    let back = read.bodies[0].bounding_box().unwrap();
    assert!(back.max_deviation_nm(&expected) <= ROUND_TRIP_TOLERANCE_NM, "{back:?} against {expected:?}");
}

#[test]
fn every_body_of_a_part_reads_back() {
    let (first, _) = block();
    let second = Body::cuboid("Spacer", [20_000_000, 0, 0], [3_000_000, 3_000_000, 5_000_000]).unwrap();
    let bytes = write_step("fixture", &[first, second]).unwrap();
    let read = read_step(&bytes).unwrap();
    let mut names: Vec<&str> = read.bodies.iter().map(Body::name).collect();
    names.sort_unstable();
    assert_eq!(names, ["Block", "Spacer"]);
    assert!(read.refused.is_empty());
}

#[test]
fn writing_the_same_bodies_twice_gives_the_same_bytes() {
    let (body, _) = block();
    let bodies = [body];
    assert_eq!(write_step("fixture", &bodies).unwrap(), write_step("fixture", &bodies).unwrap());
}

#[test]
fn a_kernel_panic_is_reported_as_kernel_internal() {
    let error = kernel_crash_test().unwrap_err();
    assert_eq!(error.code(), ErrorCode::KernelInternal);

    // A panic outside solidcore's own session guard, as in the STEP reader, is caught here.
    let error = guard("probe", || -> Result<()> { panic!("probe failure") }).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KernelInternal);
    assert_eq!(error.message(), "probe: probe failure");

    // The kernel still works after both panics.
    let (body, expected) = block();
    assert!(body.bounding_box().unwrap().max_deviation_nm(&expected) <= ROUND_TRIP_TOLERANCE_NM);
}

#[test]
fn unreadable_bytes_are_model_unreadable() {
    let inputs: [&[u8]; 3] = [b"\xff\xfe\x00", b"not a STEP file", b""];
    for bytes in inputs {
        let error = read_step(bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelUnreadable, "{bytes:?}: {error}");
    }
}

#[test]
fn a_file_without_solids_reads_as_empty() {
    let read = read_step(b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;").unwrap();
    assert!(read.bodies.is_empty());
    assert!(read.refused.is_empty());
}

#[test]
fn an_unsupported_surface_refuses_only_its_solid() {
    let (body, _) = block();
    let bytes = write_step("fixture", &[body]).unwrap();
    let text = String::from_utf8(bytes).unwrap().replace("=PLANE(", "=UNKNOWN_SURFACE(");
    let read = read_step(text.as_bytes()).unwrap();
    assert!(read.bodies.is_empty());
    assert_eq!(read.refused.len(), 1);
    assert_eq!(read.refused[0].code, ErrorCode::ModelUnsupportedEntity, "{}", read.refused[0].message);
    assert!(read.refused[0].entity > 0);
}

#[test]
fn bad_arguments_are_refused_before_the_kernel() {
    for size in [[0, 1, 1], [1, -1, 1]] {
        assert_eq!(Body::cuboid("x", [0; 3], size).unwrap_err().code(), ErrorCode::InvalidParams);
    }
    assert_eq!(Body::cuboid("x", [MAX_LENGTH_NM, 0, 0], [1, 1, 1]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(Body::cuboid("x", [i64::MAX, 0, 0], [1, 1, 1]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(write_step("fixture", &[]).unwrap_err().code(), ErrorCode::InvalidParams);
}

#[test]
fn millimetre_conversion_rounds_once_to_the_nanometre() {
    assert_eq!(point_nm(point_mm([1, -2_540_001, MAX_LENGTH_NM]).unwrap()).unwrap(), [1, -2_540_001, MAX_LENGTH_NM]);
    assert_eq!(point_nm([0.000_000_4, -0.000_000_6, 1.5]).unwrap(), [0, -1, 1_500_000]);
    assert_eq!(point_nm([f64::NAN, 0.0, 0.0]).unwrap_err().code(), ErrorCode::OperationFailed);
    assert_eq!(point_nm([2e6, 0.0, 0.0]).unwrap_err().code(), ErrorCode::OperationFailed);
}

#[test]
fn a_transformed_copy_moves_and_leaves_the_original() {
    let (body, expected) = block();
    // A quarter turn about z (x to y, y to -x), then a move.
    let turn = Transform { axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], translation_nm: [1_000_000, 0, 5_000_000] };
    let moved = body.transformed(&turn).unwrap();
    assert_eq!(moved.name(), "Block");
    let turned = BoundingBox { min_nm: [-10_500_000, -2_000_000, 5_000_000], max_nm: [-2_500_000, 10_500_000, 6_600_000] };
    let got = moved.bounding_box().unwrap();
    assert!(got.max_deviation_nm(&turned) <= ROUND_TRIP_TOLERANCE_NM, "{got:?}");
    assert!(body.bounding_box().unwrap().max_deviation_nm(&expected) <= ROUND_TRIP_TOLERANCE_NM);

    let flat = Transform { axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]], translation_nm: [0; 3] };
    assert_eq!(body.transformed(&flat).unwrap_err().code(), ErrorCode::InvalidParams);
    let not_finite = Transform { axes: [[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation_nm: [0; 3] };
    assert_eq!(body.transformed(&not_finite).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(body.transformed(&Transform::translation([MAX_LENGTH_NM + 1, 0, 0])).unwrap_err().code(), ErrorCode::InvalidParams);
}

#[test]
fn bodies_outside_any_assembly_are_placed_once_where_they_are() {
    let (body, expected) = block();
    let read = read_step(&write_step("fixture", &[body]).unwrap()).unwrap();
    assert_eq!(read.placements, [Placement { body: 0, transform: Transform::IDENTITY }]);
    let whole = read.bounding_box().unwrap().unwrap();
    assert!(whole.max_deviation_nm(&expected) <= ROUND_TRIP_TOLERANCE_NM, "{whole:?}");
    assert_eq!(read_step(b"ISO-10303-21;DATA;ENDSEC;END-ISO-10303-21;").unwrap().bounding_box().unwrap(), None);
}

#[test]
fn a_union_holds_both_boxes() {
    let a = BoundingBox { min_nm: [0, -5, 2], max_nm: [10, 0, 3] };
    let b = BoundingBox { min_nm: [-1, -2, 4], max_nm: [3, 7, 9] };
    assert_eq!(a.union(&b), BoundingBox { min_nm: [-1, -5, 2], max_nm: [10, 7, 9] });
}
