//! Solid bodies and STEP (ISO 10303-21) exchange for PCBcraft's 3D features.
//!
//! This crate is the boundary to the B-rep kernel. It is implemented with SolidCraft's
//! `solidcore` kernel and its `solidcore-ops` and `solidcore-step` crates, at the SolidCraft
//! revision the workspace already pins, but no kernel type appears in its public API: callers see
//! [`Body`], [`BoundingBox`] and [`GeometryError`] only, so the kernel can be replaced without
//! touching them. Lengths are whole nanometres; the kernel's millimetre floats stay inside.
//!
//! Every kernel call runs under a guard: a panic inside the kernel becomes an error with the code
//! [`ErrorCode::KernelInternal`] instead of ending the application, and the caller's own data is
//! unchanged. Errors carry a stable snake_case code ([`ErrorCode::as_str`]) for MCP and CLI
//! replies.
//!
//! The kernel is built for native targets only. On `wasm32` the same API compiles and every
//! operation answers [`ErrorCode::KernelUnavailable`].
//!
//! `docs/geometry-kernel.md` records the kernel choice, its qualification and its known limits.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod guard;
#[cfg(not(target_arch = "wasm32"))]
mod kernel;
#[cfg(target_arch = "wasm32")]
#[path = "kernel_unavailable.rs"]
mod kernel;

use std::fmt;

use pcbcraft_units::{MAX_LENGTH_NM, NM_PER_MM};

/// Why a geometry operation failed, as a stable code.
///
/// The text of [`ErrorCode::as_str`] is part of the command contract: codes are never renamed or
/// reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// An argument was refused before the kernel saw it (`invalid_params`).
    InvalidParams,
    /// The model bytes are not a readable STEP exchange structure, or a solid in it is malformed
    /// (`model_unreadable`).
    ModelUnreadable,
    /// The model uses an entity or construct the reader does not support yet
    /// (`model_unsupported_entity`).
    ModelUnsupportedEntity,
    /// The kernel could not complete an operation on valid input (`operation_failed`).
    OperationFailed,
    /// This build has no geometry kernel, as in the browser (`kernel_unavailable`).
    KernelUnavailable,
    /// The kernel panicked; the panic was caught and nothing the caller owns changed
    /// (`kernel_internal`).
    KernelInternal,
}

impl ErrorCode {
    /// The code's stable snake_case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::ModelUnreadable => "model_unreadable",
            Self::ModelUnsupportedEntity => "model_unsupported_entity",
            Self::OperationFailed => "operation_failed",
            Self::KernelUnavailable => "kernel_unavailable",
            Self::KernelInternal => "kernel_internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed geometry operation: its stable code and a message for people and logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeometryError {
    code: ErrorCode,
    message: String,
}

impl GeometryError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    /// The stable code clients branch on.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The human-readable explanation. Its wording may change between releases.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for GeometryError {}

/// The result of a geometry operation.
pub type Result<T> = std::result::Result<T, GeometryError>;

/// An axis-aligned box in nanometres: the least corner, then the greatest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    /// The least x, y and z.
    pub min_nm: [i64; 3],
    /// The greatest x, y and z.
    pub max_nm: [i64; 3],
}

impl BoundingBox {
    /// The largest difference between corresponding corner coordinates of `self` and `other`, in
    /// nanometres. Round-trip checks compare this with their tolerance.
    pub fn max_deviation_nm(&self, other: &BoundingBox) -> u64 {
        let corners = self.min_nm.iter().zip(&other.min_nm).chain(self.max_nm.iter().zip(&other.max_nm));
        corners.map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0)
    }
}

/// A named solid (or sheet) held by the kernel.
///
/// A body is derived data: projects store the STEP bytes or the dimensions it was built from,
/// never the kernel's own representation, so a kernel change cannot break a saved file.
pub struct Body {
    name: String,
    solid: kernel::KernelBody,
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body").field("name", &self.name).finish_non_exhaustive()
    }
}

impl Body {
    /// An axis-aligned rectangular block from its least corner and its edge lengths.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] when a size is not positive or a coordinate lies more than
    /// [`MAX_LENGTH_NM`] from zero; [`ErrorCode::KernelUnavailable`] without a kernel; otherwise
    /// the kernel's failure.
    pub fn cuboid(name: impl Into<String>, origin_nm: [i64; 3], size_nm: [i64; 3]) -> Result<Body> {
        if size_nm.iter().any(|&s| s <= 0) {
            return Err(GeometryError::new(ErrorCode::InvalidParams, "every cuboid size must be positive"));
        }
        let mut far = [0; 3];
        for ((f, o), s) in far.iter_mut().zip(origin_nm).zip(size_nm) {
            *f = o.checked_add(s).ok_or_else(|| GeometryError::new(ErrorCode::InvalidParams, "cuboid corner overflows"))?;
        }
        let origin = point_mm(origin_nm)?;
        point_mm(far)?;
        let size = point_mm(size_nm)?;
        let solid = kernel::cuboid(origin, size)?;
        Ok(Body { name: name.into(), solid })
    }

    /// The body's name: the STEP solid's own name, else its product's; empty when it has none.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The body's exact axis-aligned box, each coordinate rounded once to the nearest nanometre.
    /// The box bounds the exact curves and surfaces, not only the vertices.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::OperationFailed`] when the kernel has no finite box or it lies outside
    /// [`MAX_LENGTH_NM`]; [`ErrorCode::KernelInternal`] when the kernel panics.
    pub fn bounding_box(&self) -> Result<BoundingBox> {
        let (lo, hi) = kernel::bounds(&self.solid)?;
        Ok(BoundingBox { min_nm: point_nm(lo)?, max_nm: point_nm(hi)? })
    }
}

/// A STEP solid item the reader did not turn into a [`Body`]; the rest of the file is still read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefusedItem {
    /// The item's entity instance number in the file (`#N`).
    pub entity: u64,
    /// Why, as a stable code.
    pub code: ErrorCode,
    /// Why, in words.
    pub message: String,
}

/// What [`read_step`] found: every body it read, in the file's entity order, and every solid
/// item it refused.
#[derive(Debug, Default)]
pub struct StepRead {
    /// The bodies read, in the file's entity order.
    pub bodies: Vec<Body>,
    /// The solid items refused, with their reasons.
    pub refused: Vec<RefusedItem>,
}

/// Reads the solids and surface models of a STEP file (AP203, AP214 or AP242) in the units its
/// representation context states. Assembly placements are not applied yet: each body is in its
/// own representation's frame.
///
/// A solid that cannot be read is listed in [`StepRead::refused`] and the others are still read.
/// Reading has no side effects.
///
/// # Errors
///
/// [`ErrorCode::ModelUnreadable`] when the bytes are not UTF-8 text or not an ISO 10303-21
/// exchange structure; [`ErrorCode::KernelUnavailable`] without a kernel;
/// [`ErrorCode::KernelInternal`] when the reader panics.
pub fn read_step(bytes: &[u8]) -> Result<StepRead> {
    let text = std::str::from_utf8(bytes).map_err(|e| GeometryError::new(ErrorCode::ModelUnreadable, format!("STEP text is not UTF-8: {e}")))?;
    let (bodies, refused) = kernel::read(text)?;
    let bodies = bodies.into_iter().map(|(name, solid)| Body { name, solid }).collect();
    Ok(StepRead { bodies, refused })
}

/// Writes `bodies` as one part named `part_name` in an AP242 STEP file, exactly as the kernel
/// holds them, without colours. The output depends only on the input: the header carries no time
/// stamp, so writing the same bodies twice gives the same bytes.
///
/// # Errors
///
/// [`ErrorCode::InvalidParams`] when `bodies` is empty; [`ErrorCode::OperationFailed`] when the
/// writer refuses a body; [`ErrorCode::KernelUnavailable`] without a kernel;
/// [`ErrorCode::KernelInternal`] when the kernel panics.
pub fn write_step(part_name: &str, bodies: &[Body]) -> Result<Vec<u8>> {
    if bodies.is_empty() {
        return Err(GeometryError::new(ErrorCode::InvalidParams, "there are no bodies to write"));
    }
    let named: Vec<(&str, &kernel::KernelBody)> = bodies.iter().map(|b| (b.name.as_str(), &b.solid)).collect();
    kernel::write(part_name, &named).map(String::into_bytes)
}

/// Makes the kernel panic inside one of its own calls and reports what the guard returns. It is
/// the crash-report check for diagnostics, not a modelling operation, and changes no body.
///
/// # Errors
///
/// Always: [`ErrorCode::KernelInternal`] when the panic was caught as it should be, or
/// [`ErrorCode::KernelUnavailable`] without a kernel.
pub fn kernel_crash_test() -> Result<()> {
    kernel::crash_test()
}

/// A point in nanometres as the kernel's millimetres.
///
/// # Errors
///
/// [`ErrorCode::InvalidParams`] for a coordinate more than [`MAX_LENGTH_NM`] from zero.
fn point_mm(nm: [i64; 3]) -> Result<[f64; 3]> {
    if nm.iter().any(|v| v.unsigned_abs() > MAX_LENGTH_NM.unsigned_abs()) {
        return Err(GeometryError::new(ErrorCode::InvalidParams, "a coordinate lies outside the supported range"));
    }
    // Every value within MAX_LENGTH_NM is exact in f64, so the division rounds once.
    Ok(nm.map(|v| v as f64 / NM_PER_MM as f64))
}

/// A point in the kernel's millimetres as nanometres, each coordinate rounded once.
///
/// # Errors
///
/// [`ErrorCode::OperationFailed`] for a coordinate that is not finite or lies more than
/// [`MAX_LENGTH_NM`] from zero.
fn point_nm(mm: [f64; 3]) -> Result<[i64; 3]> {
    let mut out = [0; 3];
    for (o, v) in out.iter_mut().zip(mm) {
        let nm = (v * NM_PER_MM as f64).round();
        if !nm.is_finite() || nm.abs() > MAX_LENGTH_NM as f64 {
            return Err(GeometryError::new(ErrorCode::OperationFailed, "the kernel gave a coordinate outside the supported range"));
        }
        *o = nm as i64;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
