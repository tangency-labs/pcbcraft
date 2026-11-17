//! The 3D/STEP boundary on `wasm32`: every operation that reaches it
//! answers [`ErrorCode::KernelUnavailable`], and no body can exist.

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
