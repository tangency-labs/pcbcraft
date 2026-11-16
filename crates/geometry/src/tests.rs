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
    // The reader keeps the file's entity order, which is the order the bodies were written in.
    let names: Vec<&str> = read.bodies.iter().map(Body::name).collect();
    assert_eq!(names, ["Block", "Spacer"]);
    assert!(read.refused.is_empty());

    let (first, _) = block();
    let second = Body::cuboid("Spacer", [20_000_000, 0, 0], [3_000_000, 3_000_000, 5_000_000]).unwrap();
    let read = read_step(&write_step("fixture", &[second, first]).unwrap()).unwrap();
    let names: Vec<&str> = read.bodies.iter().map(Body::name).collect();
    assert_eq!(names, ["Spacer", "Block"]);
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
    assert!(read.placements.is_empty(), "a refused solid is not placed");
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
    assert!(body.transformed(&Transform::translation([-MAX_LENGTH_NM, 0, MAX_LENGTH_NM])).is_ok());
    assert_eq!(body.transformed(&Transform::translation([MAX_LENGTH_NM + 1, 0, 0])).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(body.transformed(&Transform::translation([0, -MAX_LENGTH_NM - 1, 0])).unwrap_err().code(), ErrorCode::InvalidParams);
}

#[test]
fn a_placement_of_a_missing_body_is_refused() {
    let (body, _) = block();
    let mut read = read_step(&write_step("fixture", &[body]).unwrap()).unwrap();
    read.placements.push(Placement { body: 1, transform: Transform::IDENTITY });
    assert_eq!(read.placed_bodies().unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(read.bounding_box().unwrap_err().code(), ErrorCode::InvalidParams);
    read.placements.truncate(1);
    assert_eq!(read.placed_bodies().unwrap().len(), 1);
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

#[test]
fn both_ends_of_the_coordinate_range_are_accepted_and_one_past_is_refused() {
    // One-millimetre blocks: a one-nanometre block a kilometre out is below the kernel's
    // tolerance there, and the kernel refuses its box.
    let mm = NM_PER_MM;
    let top = Body::cuboid("top", [MAX_LENGTH_NM - mm, 0, 0], [mm, mm, mm]).unwrap();
    let bottom = Body::cuboid("bottom", [-MAX_LENGTH_NM, 0, 0], [mm, mm, mm]).unwrap();
    assert_eq!(top.bounding_box().unwrap().max_nm[0], MAX_LENGTH_NM);
    assert_eq!(bottom.bounding_box().unwrap().min_nm[0], -MAX_LENGTH_NM);

    assert_eq!(Body::cuboid("x", [MAX_LENGTH_NM, 0, 0], [1, 1, 1]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(Body::cuboid("x", [-MAX_LENGTH_NM - 1, 0, 0], [1, 1, 1]).unwrap_err().code(), ErrorCode::InvalidParams);

    let limit_mm = MAX_LENGTH_NM as f64 / NM_PER_MM as f64;
    let one_nm_mm = 1.0 / NM_PER_MM as f64;
    assert_eq!(point_nm([limit_mm, -limit_mm, 0.0]).unwrap(), [MAX_LENGTH_NM, -MAX_LENGTH_NM, 0]);
    assert_eq!(point_nm([limit_mm + one_nm_mm, 0.0, 0.0]).unwrap_err().code(), ErrorCode::OperationFailed);
    assert_eq!(point_nm([0.0, -limit_mm - one_nm_mm, 0.0]).unwrap_err().code(), ErrorCode::OperationFailed);
}

fn names_after_round_trip(part_name: &str, body_name: &str) -> Vec<String> {
    let body = Body::cuboid(body_name, [0; 3], [1_000_000; 3]).unwrap();
    let read = read_step(&write_step(part_name, &[body]).unwrap()).unwrap();
    assert!(read.refused.is_empty(), "{:?}", read.refused);
    read.bodies.iter().map(|b| b.name().to_string()).collect()
}

#[test]
fn names_read_back_unchanged() {
    let awkward = "O'Brien \\ bracket Ø 中 🚀";
    assert_eq!(names_after_round_trip("fixture", awkward), [awkward]);
    assert_eq!(names_after_round_trip("fixture", "  padded  "), ["  padded  "]);
    assert_eq!(names_after_round_trip(awkward, ""), [awkward]);
    assert_eq!(names_after_round_trip(awkward, "Block"), ["Block"]);
    assert_eq!(names_after_round_trip("", "Block"), ["Block"]);
    // A body without a name of its own takes its part's name, as Body::name says, and the writer
    // names a part without a name "Part".
    assert_eq!(names_after_round_trip("fixture", ""), ["fixture"]);
    assert_eq!(names_after_round_trip("", ""), ["Part"]);
    // The writer treats the part name as a file name: trailing STEP suffixes go, and a blank
    // result is "Part".
    assert_eq!(names_after_round_trip("board.STEP", ""), ["board"]);
    assert_eq!(names_after_round_trip("board.stp.STEP", ""), ["board"]);
    // `.step` suffixes go first, then `.stp` ones, so a `.step` before a `.stp` stays.
    assert_eq!(names_after_round_trip("board.step.stp", ""), ["board.step"]);
    assert_eq!(names_after_round_trip(".step", ""), ["Part"]);
    assert_eq!(names_after_round_trip("  ", ""), ["Part"]);
}

#[test]
fn names_with_control_characters_are_refused() {
    for name in ["tab\there", "line\nbreak", "nul\0", "delete\u{7f}"] {
        assert_eq!(Body::cuboid(name, [0; 3], [1; 3]).unwrap_err().code(), ErrorCode::InvalidParams, "{name:?}");
        let (body, _) = block();
        assert_eq!(write_step(name, &[body]).unwrap_err().code(), ErrorCode::InvalidParams, "{name:?}");
    }
}

#[test]
fn a_body_record_the_kernel_cannot_make_keeps_its_code() {
    // A valid body cannot be made to fail its record here, so the record step is replaced.
    let bodies = [("Block", ())];
    let emit = |_: &[(&str, ())]| -> Result<String> { Ok(String::new()) };
    let caught = kernel::write_with(&bodies, |_| Err(solidcore_ops::Error::Internal("probe".into())), emit);
    assert_eq!(caught.unwrap_err().code(), ErrorCode::KernelInternal);
    let panicked = kernel::write_with(&bodies, |_| -> solidcore_ops::Result<()> { panic!("probe") }, emit);
    assert_eq!(panicked.unwrap_err().code(), ErrorCode::KernelInternal);
    let refused = kernel::write_with(&bodies, |_| Err(solidcore_ops::Error::Unsupported("probe")), emit);
    assert_eq!(refused.unwrap_err().code(), ErrorCode::OperationFailed);
}

/// Each `#id=MANIFOLD_SOLID_BREP('name',#shell);` line of a STEP text: id, shell, name.
fn solid_lines(text: &str) -> Vec<(u64, u64, String)> {
    text.lines()
        .filter_map(|line| {
            let (id, body) = line.strip_prefix('#')?.split_once('=')?;
            let rest = body.strip_prefix("MANIFOLD_SOLID_BREP('")?;
            let (name, shell) = rest.rsplit_once("',#")?;
            Some((id.parse().ok()?, shell.strip_suffix(");")?.parse().ok()?, name.to_string()))
        })
        .collect()
}

#[test]
fn a_surface_model_with_shells_out_of_order_reads_back_in_entity_order() {
    let a = Body::cuboid("A", [0; 3], [1_000_000; 3]).unwrap();
    let b = Body::cuboid("B", [20_000_000, 0, 0], [1_000_000; 3]).unwrap();
    let text = String::from_utf8(write_step("fixture", &[a, b]).unwrap()).unwrap();
    let solids = solid_lines(&text);
    assert_eq!(solids.len(), 2, "{text}");
    let face = text
        .lines()
        .find_map(|line| {
            let (id, body) = line.strip_prefix('#')?.split_once('=')?;
            body.starts_with("ADVANCED_FACE(").then(|| id.to_string())
        })
        .unwrap();
    let (first, second) = (&solids[0], &solids[1]);
    let (low, high) = (first.1.min(second.1), first.1.max(second.1));
    // The first solid becomes an open shell of one face, and the second a surface model holding
    // both closed shells, the higher-numbered first, and that open shell. The reader expands the
    // model into its closed shells in that order, then the model.
    let text: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with(&format!("#{}=", first.0)) {
                format!("#{}=OPEN_SHELL('',(#{face}));", first.0)
            } else if line.starts_with(&format!("#{}=", second.0)) {
                format!("#{}=SHELL_BASED_SURFACE_MODEL('',(#{high},#{low},#{}));", second.0, first.0)
            } else {
                line.to_string()
            }
        })
        .collect();
    let read = read_step(text.join("\n").as_bytes()).unwrap();
    assert!(read.bodies.len() >= 2, "{:?} {:?}", read.bodies, read.refused);
    let x_of = |shell: u64| if solids.iter().any(|s| s.1 == shell && s.2 == "A") { 0 } else { 20_000_000 };
    for (body, shell) in read.bodies.iter().zip([low, high]) {
        let min_x = body.bounding_box().unwrap().min_nm[0];
        assert!((min_x - x_of(shell)).abs() <= ROUND_TRIP_TOLERANCE_NM as i64, "{min_x} for shell #{shell}");
    }
}
