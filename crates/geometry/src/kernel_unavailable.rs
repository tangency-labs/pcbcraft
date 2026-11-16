//! The kernel boundary for builds without a kernel (`wasm32`): every operation answers
//! [`ErrorCode::KernelUnavailable`], and no body can exist.

use crate::{ErrorCode, GeometryError, RefusedItem, Result};

/// No body can be made without a kernel.
pub(crate) enum KernelBody {}

fn unavailable(what: &str) -> GeometryError {
    GeometryError::new(ErrorCode::KernelUnavailable, format!("{what}: this build has no geometry kernel"))
}

pub(crate) fn cuboid(_origin: [f64; 3], _size: [f64; 3]) -> Result<KernelBody> {
    Err(unavailable("cuboid"))
}

pub(crate) fn bounds(body: &KernelBody) -> Result<([f64; 3], [f64; 3])> {
    match *body {}
}

pub(crate) fn write(_part_name: &str, _bodies: &[(&str, &KernelBody)]) -> Result<String> {
    Err(unavailable("STEP write"))
}

pub(crate) fn read(_text: &str) -> Result<(Vec<(String, KernelBody)>, Vec<RefusedItem>)> {
    Err(unavailable("STEP read"))
}

pub(crate) fn crash_test() -> Result<()> {
    Err(unavailable("crash test"))
}
