//! The crate on `wasm32`, which has no kernel: run by CI with
//! `cargo test -p pcbcraft-geometry --target wasm32-wasip1 --lib`.

use super::*;

/// Without a kernel, making a body, reading STEP and the crash test are `kernel_unavailable`.
#[test]
fn every_operation_that_reaches_the_kernel_is_kernel_unavailable() {
    let made = Body::cuboid("Block", [0; 3], [1_000_000; 3]);
    assert_eq!(made.unwrap_err().code(), ErrorCode::KernelUnavailable);

    let text = b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
    assert_eq!(read_step(text).unwrap_err().code(), ErrorCode::KernelUnavailable);

    assert_eq!(kernel_crash_test().unwrap_err().code(), ErrorCode::KernelUnavailable);
}

/// Without a kernel, bad arguments keep their own codes, since they are refused first.
#[test]
fn arguments_are_still_refused_before_the_kernel() {
    // No body can be made here, so write_step can only be given an empty list, which it refuses
    // before reaching the kernel.
    assert_eq!(write_step("fixture", &[]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(Body::cuboid("Block", [0; 3], [0, 1, 1]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(Body::cuboid("tab\there", [0; 3], [1; 3]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(read_step(&[0xff, 0xfe]).unwrap_err().code(), ErrorCode::ModelUnreadable);
}

#[test]
fn placements_need_no_kernel_until_a_body_is_moved() {
    let empty = StepRead::default();
    assert!(empty.placed_bodies().unwrap().is_empty());
    assert_eq!(empty.bounding_box().unwrap(), None);

    let mut missing = StepRead::default();
    missing.placements.push(Placement { body: 0, transform: Transform::IDENTITY });
    assert_eq!(missing.placed_bodies().unwrap_err().code(), ErrorCode::InvalidParams);

    let a = BoundingBox { min_nm: [0, 0, 0], max_nm: [1, 1, 1] };
    let b = BoundingBox { min_nm: [-1, 2, 0], max_nm: [0, 3, 1] };
    assert_eq!(a.union(&b), BoundingBox { min_nm: [-1, 0, 0], max_nm: [1, 3, 1] });
}
