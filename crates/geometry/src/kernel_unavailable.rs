//! The kernel boundary for builds without a kernel (`wasm32`): every operation that reaches it
//! answers [`ErrorCode::KernelUnavailable`], and no body can exist.

use crate::{ErrorCode, GeometryError, RefusedItem, Result};

/// No body can be made without a kernel.
pub(crate) enum KernelBody {}

/// The `kernel_unavailable` error for the operation `what`.
fn unavailable(what: &str) -> GeometryError {
    GeometryError::new(ErrorCode::KernelUnavailable, format!("{what}: this build has no geometry kernel"))
}

/// No block can be made: always `kernel_unavailable`.
pub(crate) fn cuboid(_origin: [f64; 3], _size: [f64; 3]) -> Result<KernelBody> {
    Err(unavailable("cuboid"))
}

/// Never called, since no [`KernelBody`] exists.
pub(crate) fn bounds(body: &KernelBody) -> Result<([f64; 3], [f64; 3])> {
    match *body {}
}

/// Never reached with a body, since none exists: always `kernel_unavailable`.
pub(crate) fn write(_part_name: &str, _bodies: &[(&str, &KernelBody)]) -> Result<String> {
    Err(unavailable("STEP write"))
}

/// The named bodies read from a file, and the solid items refused.
pub(crate) type Read = (Vec<(String, KernelBody)>, Vec<RefusedItem>);

/// No STEP text can be read: always `kernel_unavailable`.
pub(crate) fn read(_text: &str) -> Result<Read> {
    Err(unavailable("STEP read"))
}

/// No kernel to crash: always `kernel_unavailable`.
pub(crate) fn crash_test() -> Result<()> {
    Err(unavailable("crash test"))
}
