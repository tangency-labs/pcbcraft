//! The 3D and STEP boundary on wasm32 when the optional kernel feature is disabled.
//! Operations that reach it return [`ErrorCode::KernelUnavailable`].

use std::collections::BTreeSet;

use crate::substrate::{BoardMesh, Region, RoundDrill};
use crate::{ErrorCode, GeometryError, RefusedItem, Result};

/// No body can be made without a kernel.
pub(crate) enum KernelBody {}

/// The `kernel_unavailable` error for the operation `what`.
fn unavailable(what: &str) -> GeometryError {
    GeometryError::new(ErrorCode::KernelUnavailable, format!("{what}: this build has no 3D/STEP kernel"))
}

/// No block can be made: always `kernel_unavailable`.
pub(crate) fn cuboid(_origin: [f64; 3], _size: [f64; 3]) -> Result<KernelBody> {
    Err(unavailable("cuboid"))
}

/// Valid geometry still cannot produce a substrate without a kernel.
pub(crate) fn substrate(_regions: &[Region], _drills: BTreeSet<RoundDrill>, _thickness_nm: i64) -> Result<KernelBody> {
    Err(unavailable("build board substrate"))
}

/// Never called, since no [`KernelBody`] exists.
pub(crate) fn substrate_mesh(body: &KernelBody, _chord_nm: i64) -> Result<BoardMesh> {
    match *body {}
}

/// Never called, since no [`KernelBody`] exists.
pub(crate) fn bounds(body: &KernelBody) -> Result<([f64; 3], [f64; 3])> {
    match *body {}
}

/// Never called, since no [`KernelBody`] exists.
pub(crate) fn transformed(body: &KernelBody, _matrix: &[[f64; 4]; 4]) -> Result<KernelBody> {
    match *body {}
}

/// Never reached with a body, since none exists: always `kernel_unavailable`.
pub(crate) fn write(_part_name: &str, _bodies: &[(&str, &KernelBody)]) -> Result<String> {
    Err(unavailable("STEP write"))
}

/// What a file holds: its named bodies, their placements and the solid items refused. Never
/// built without a kernel.
#[allow(dead_code)]
pub(crate) struct Read {
    pub(crate) bodies: Vec<(String, KernelBody)>,
    pub(crate) placements: Vec<(usize, [[f64; 4]; 4])>,
    pub(crate) refused: Vec<RefusedItem>,
}

/// No STEP text can be read: always `kernel_unavailable`.
pub(crate) fn read(_text: &str) -> Result<Read> {
    Err(unavailable("STEP read"))
}

/// No kernel to crash: always `kernel_unavailable`.
pub(crate) fn crash_test() -> Result<()> {
    Err(unavailable("crash test"))
}
