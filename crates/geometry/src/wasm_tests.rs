//! The 3D and STEP boundary on wasm32 without the optional `wasm-kernel` feature.
//! Run by CI with
//! `cargo test -p pcbcraft-geometry --target wasm32-wasip1 --lib`.

use super::*;

/// Without a kernel, making a body, reading STEP and the crash test are `kernel_unavailable`.
#[test]
fn every_operation_that_reaches_the_kernel_is_kernel_unavailable() {
    // Each call is valid, so only the missing kernel can refuse it, and the message names it.
    let unavailable = |result: Result<()>, what: &str| {
        let error = result.unwrap_err();
        assert_eq!(error.code(), ErrorCode::KernelUnavailable, "{error}");
        assert_eq!(error.message(), format!("{what}: this build has no 3D/STEP kernel"));
    };
    unavailable(Body::cuboid("Block", [0; 3], [1_000_000; 3]).map(|_| ()), "cuboid");
    let text = b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
    unavailable(read_step(text).map(|_| ()), "STEP read");
    unavailable(kernel_crash_test(), "crash test");
}

/// Without a kernel, bad arguments keep their own codes, since they are refused first.
#[test]
fn arguments_are_still_refused_before_the_kernel() {
    // No body can be made here, so write_step can only be given an empty list, which it refuses
    // before reaching the kernel.
    let refused = |result: Result<()>, code: ErrorCode, reason: &str| {
        let error = result.unwrap_err();
        assert_eq!(error.code(), code, "{error}");
        assert!(error.message().contains(reason), "{error}");
    };
    refused(write_step("fixture", &[]).map(|_| ()), ErrorCode::InvalidParams, "there are no bodies to write");
    refused(write_step_assembly("fixture", &[], &[]).map(|_| ()), ErrorCode::InvalidParams, "there are no occurrences to write");
    refused(Body::cuboid("Block", [0; 3], [0, 1, 1]).map(|_| ()), ErrorCode::InvalidParams, "every cuboid size must be positive");
    refused(Body::cuboid("tab\there", [0; 3], [1; 3]).map(|_| ()), ErrorCode::InvalidParams, "the body name has a control character");
    refused(read_step(&[0xff, 0xfe]).map(|_| ()), ErrorCode::ModelUnreadable, "STEP text is not UTF-8");
}

/// Without a kernel, an empty read has no placed bodies and no box, a placement of a missing
/// body is `invalid_params`, and boxes still unite.
#[test]
fn placements_need_no_kernel_until_a_body_is_moved() {
    let empty = StepRead::default();
    assert!(empty.placed_bodies().unwrap().is_empty());
    assert_eq!(empty.bounding_box().unwrap(), None);

    let mut missing = StepRead::default();
    missing.placements.push(Placement { body: 0, transform: Transform::IDENTITY });
    let error = missing.placed_bodies().unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams, "{error}");
    assert!(error.message().contains("placement of body 0 out of range"), "{error}");

    let a = BoundingBox { min_nm: [0, 0, 0], max_nm: [1, 1, 1] };
    let b = BoundingBox { min_nm: [-1, 2, 0], max_nm: [0, 3, 1] };
    assert_eq!(a.union(&b), BoundingBox { min_nm: [-1, 0, 0], max_nm: [1, 3, 1] });
}
