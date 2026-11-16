use super::*;
use crate::guard::guard;

/// The round-trip tolerance for bounding boxes: one micrometre.
const ROUND_TRIP_TOLERANCE_NM: u64 = 1_000;

/// A 12.5 x 8 x 1.6 mm block named `Block` off the origin, and the box it must have.
fn block() -> (Body, BoundingBox) {
    let origin = [-2_000_000, 3_500_000, 0];
    let size = [12_500_000, 8_000_000, 1_600_000];
    let expected = BoundingBox { min_nm: origin, max_nm: [10_500_000, 11_500_000, 1_600_000] };
    (Body::cuboid("Block", origin, size).unwrap(), expected)
}

/// The error codes keep the names that callers and saved results rely on.
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

/// A block written as AP242 and read back keeps its name and its box within one micrometre.
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

/// Both bodies of a part read back by name, in the order written, in either order.
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

/// The writer is deterministic: the same bodies give the same bytes.
#[test]
fn writing_the_same_bodies_twice_gives_the_same_bytes() {
    let (body, _) = block();
    let bodies = [body];
    assert_eq!(write_step("fixture", &bodies).unwrap(), write_step("fixture", &bodies).unwrap());
}

/// A panic inside or outside the kernel's own guard is `kernel_internal`, and the kernel keeps working.
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

/// Bytes that are not UTF-8, text that is not STEP and an empty input are `model_unreadable`.
#[test]
fn unreadable_bytes_are_model_unreadable() {
    // Each input fails one check only, and the message names that check.
    let inputs: [(&[u8], &str); 3] = [
        (b"\xff\xfe\x00", "STEP text is not UTF-8"),
        (b"not a STEP file", "not a STEP file (no ISO-10303-21 header)"),
        (b"", "STEP syntax error at line 1: expected a keyword"),
    ];
    for (bytes, reason) in inputs {
        let error = read_step(bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelUnreadable, "{bytes:?}: {error}");
        assert!(error.message().contains(reason), "{bytes:?}: {error}");
    }
}

/// A STEP file with no solid item reads as no bodies and no refusals.
#[test]
fn a_file_without_solids_reads_as_empty() {
    let read = read_step(b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;").unwrap();
    assert!(read.bodies.is_empty());
    assert!(read.refused.is_empty());
}

/// A solid on a surface entity the reader does not know is refused with its entity number.
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

/// Non-positive sizes, out-of-range corners and an empty write are `invalid_params`.
#[test]
fn bad_arguments_are_refused_before_the_kernel() {
    // Each call differs from an accepted one in one argument only, so only that argument's check
    // can refuse it, and the message names which check did.
    let mm = NM_PER_MM;
    let refused_for = |result: Result<Body>, reason: &str| {
        let error = result.unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidParams, "{error}");
        assert!(error.message().contains(reason), "{error}");
    };
    assert!(Body::cuboid("x", [0; 3], [mm; 3]).is_ok());
    refused_for(Body::cuboid("x", [0; 3], [0, mm, mm]), "every cuboid size must be positive");
    refused_for(Body::cuboid("x", [0; 3], [mm, -mm, mm]), "every cuboid size must be positive");
    // The least corner is in range; only the far corner, one nanometre past the end, is not.
    assert!(Body::cuboid("x", [MAX_LENGTH_NM - mm, 0, 0], [mm; 3]).is_ok());
    refused_for(Body::cuboid("x", [MAX_LENGTH_NM - mm + 1, 0, 0], [mm; 3]), "a coordinate lies outside the supported range");
    refused_for(Body::cuboid("x", [i64::MAX, 0, 0], [mm; 3]), "cuboid corner overflows");

    let (body, _) = block();
    assert!(write_step("fixture", &[body]).is_ok());
    let error = write_step("fixture", &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams);
    assert!(error.message().contains("there are no bodies to write"), "{error}");
}

/// Millimetres convert to nanometres with one rounding, and non-finite or huge values are refused.
#[test]
fn millimetre_conversion_rounds_once_to_the_nanometre() {
    assert_eq!(point_nm(point_mm([1, -2_540_001, MAX_LENGTH_NM]).unwrap()).unwrap(), [1, -2_540_001, MAX_LENGTH_NM]);
    assert_eq!(point_nm([0.000_000_4, -0.000_000_6, 1.5]).unwrap(), [0, -1, 1_500_000]);
    for mm in [[f64::NAN, 0.0, 0.0], [2e6, 0.0, 0.0]] {
        let error = point_nm(mm).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OperationFailed, "{error}");
        assert!(error.message().contains("the kernel gave a coordinate outside the supported range"), "{error}");
    }
}

/// Blocks touching either end of the coordinate range are exact, and one nanometre past is refused.
#[test]
fn both_ends_of_the_coordinate_range_are_accepted_and_one_past_is_refused() {
    // One-millimetre blocks: a one-nanometre block a kilometre out is below the kernel's
    // tolerance there, and the kernel refuses its box.
    let mm = NM_PER_MM;
    let top = Body::cuboid("top", [MAX_LENGTH_NM - mm, 0, 0], [mm, mm, mm]).unwrap();
    let bottom = Body::cuboid("bottom", [-MAX_LENGTH_NM, 0, 0], [mm, mm, mm]).unwrap();
    assert_eq!(top.bounding_box().unwrap().max_nm[0], MAX_LENGTH_NM);
    assert_eq!(bottom.bounding_box().unwrap().min_nm[0], -MAX_LENGTH_NM);

    let range = "a coordinate lies outside the supported range";
    for (origin, size) in [([MAX_LENGTH_NM, 0, 0], [1, 1, 1]), ([-MAX_LENGTH_NM - 1, 0, 0], [1, 1, 1])] {
        let error = Body::cuboid("x", origin, size).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidParams, "{error}");
        assert!(error.message().contains(range), "{error}");
    }

    let limit_mm = MAX_LENGTH_NM as f64 / NM_PER_MM as f64;
    let one_nm_mm = 1.0 / NM_PER_MM as f64;
    assert_eq!(point_nm([limit_mm, -limit_mm, 0.0]).unwrap(), [MAX_LENGTH_NM, -MAX_LENGTH_NM, 0]);
    for mm in [[limit_mm + one_nm_mm, 0.0, 0.0], [0.0, -limit_mm - one_nm_mm, 0.0]] {
        let error = point_nm(mm).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OperationFailed, "{error}");
        assert!(error.message().contains("the kernel gave a coordinate outside the supported range"), "{error}");
    }
}

/// The body names read back from one block named `body_name` in a part named `part_name`.
fn names_after_round_trip(part_name: &str, body_name: &str) -> Vec<String> {
    let body = Body::cuboid(body_name, [0; 3], [1_000_000; 3]).unwrap();
    let read = read_step(&write_step(part_name, &[body]).unwrap()).unwrap();
    assert!(read.refused.is_empty(), "{:?}", read.refused);
    read.bodies.iter().map(|b| b.name().to_string()).collect()
}

/// Body and part names survive the round trip, with the writer's rules for empty and suffixed names.
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

/// Names with ASCII control characters, which STEP strings cannot carry, are `invalid_params`.
#[test]
fn names_with_control_characters_are_refused() {
    // The other arguments are valid, so only the name can be refused, and the message says so.
    let mm = NM_PER_MM;
    assert!(Body::cuboid("clean", [0; 3], [mm; 3]).is_ok());
    for name in ["tab\there", "line\nbreak", "nul\0", "delete\u{7f}"] {
        let error = Body::cuboid(name, [0; 3], [mm; 3]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidParams, "{name:?}");
        assert!(error.message().contains("the body name has a control character"), "{name:?}: {error}");
        let (body, _) = block();
        let error = write_step(name, &[body]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidParams, "{name:?}");
        assert!(error.message().contains("the part name has a control character"), "{name:?}: {error}");
    }
}

/// A kernel failure while making a body's STEP record keeps its code: a caught panic is
/// `kernel_internal`, a refusal is `operation_failed`.
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

/// A surface model listing its closed shells out of entity order reads them back in ascending
/// entity-number order.
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
