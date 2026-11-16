//! Solid bodies and STEP (ISO 10303-21) exchange for PCBcraft's 3D features.
//!
//! This crate is the boundary to the B-rep kernel. It is implemented with SolidCraft's
//! `solidcore` kernel and its `solidcore-ops` and `solidcore-step` crates, at the SolidCraft
//! revision the workspace already pins, but no kernel type appears in its public API: callers see
//! [`Body`], [`BoundingBox`], [`Transform`] and [`GeometryError`] only, so the kernel can be replaced without
//! touching them. Lengths are whole nanometres; the kernel's millimetre floats stay inside.
//!
//! Every kernel operation this crate calls (making, measuring, reading and writing) runs under a
//! guard: a panic inside it becomes an error with the code [`ErrorCode::KernelInternal`] instead
//! of ending the application, and the caller's own data is unchanged. Dropping a [`Body`] frees
//! its kernel data outside the guard. Errors carry a stable snake_case code ([`ErrorCode::as_str`]) for MCP and CLI
//! replies.
//!
//! The kernel is built for native targets only. On `wasm32` the same API compiles, no [`Body`]
//! can be made, and every operation that reaches the kernel answers
//! [`ErrorCode::KernelUnavailable`]; arguments refused before the kernel keep their own codes.
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

    /// The smallest box holding both `self` and `other`.
    pub fn union(&self, other: &BoundingBox) -> BoundingBox {
        let mut out = *self;
        for ((lo, hi), (other_lo, other_hi)) in out.min_nm.iter_mut().zip(out.max_nm.iter_mut()).zip(other.min_nm.iter().zip(&other.max_nm)) {
            *lo = (*lo).min(*other_lo);
            *hi = (*hi).max(*other_hi);
        }
        out
    }
}

/// An affine placement: a point `p` goes to `p.x·axes[0] + p.y·axes[1] + p.z·axes[2] +
/// translation`.
///
/// The axes are the images of the x, y and z unit vectors, as unitless ratios; the translation is
/// in nanometres. Placements read from a file are rotations, possibly mirrored, with at most a
/// uniform scale; a non-uniform scale or a shear is accepted by [`Body::transformed`] and turns
/// the body's surfaces into B-splines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// The images of the x, y and z unit vectors.
    pub axes: [[f64; 3]; 3],
    /// Where the origin goes, in nanometres.
    pub translation_nm: [i64; 3],
}

impl Transform {
    /// The placement that leaves every point where it is.
    pub const IDENTITY: Transform = Transform { axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation_nm: [0; 3] };

    /// A move by `offset_nm` without turning.
    pub const fn translation(offset_nm: [i64; 3]) -> Transform {
        Transform { axes: Transform::IDENTITY.axes, translation_nm: offset_nm }
    }

    /// The kernel's affine matrix in millimetres: rows 0 to 2 the axes, row 3 the translation,
    /// the fourth column `(0, 0, 0, 1)`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] for an axis that is not finite or a translation more than
    /// [`MAX_LENGTH_NM`] from zero.
    fn matrix_mm(&self) -> Result<[[f64; 4]; 4]> {
        if self.axes.iter().flatten().any(|v| !v.is_finite()) {
            return Err(GeometryError::new(ErrorCode::InvalidParams, "a transform axis is not finite"));
        }
        let t = point_mm(self.translation_nm)?;
        let [x, y, z] = self.axes;
        Ok([[x[0], x[1], x[2], 0.0], [y[0], y[1], y[2], 0.0], [z[0], z[1], z[2], 0.0], [t[0], t[1], t[2], 1.0]])
    }

    /// The placement of a kernel matrix in millimetres, laid out as [`Transform::matrix_mm`]
    /// gives it, or `None` when an entry is not finite or the translation lies more than
    /// [`MAX_LENGTH_NM`] from zero.
    fn from_matrix_mm(m: &[[f64; 4]; 4]) -> Option<Transform> {
        let [x, y, z, t] = m;
        let axes = [[x[0], x[1], x[2]], [y[0], y[1], y[2]], [z[0], z[1], z[2]]];
        if axes.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        Some(Transform { axes, translation_nm: point_nm([t[0], t[1], t[2]]).ok()? })
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
    /// [`ErrorCode::InvalidParams`] when the name has an ASCII control character, a size is not
    /// positive or a coordinate lies more than [`MAX_LENGTH_NM`] from zero;
    /// [`ErrorCode::KernelUnavailable`] without a kernel; otherwise the kernel's failure.
    pub fn cuboid(name: impl Into<String>, origin_nm: [i64; 3], size_nm: [i64; 3]) -> Result<Body> {
        let name = name.into();
        check_name("body name", &name)?;
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
        Ok(Body { name, solid })
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

    /// A copy of the body, with the same name, moved by `transform`. The body itself is
    /// unchanged.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] for an axis that is not finite, a translation more than
    /// [`MAX_LENGTH_NM`] from zero, or axes the kernel does not accept as a regular map (a
    /// flattening, or a scale too uneven to prove regular); otherwise the kernel's failure.
    pub fn transformed(&self, transform: &Transform) -> Result<Body> {
        let matrix = transform.matrix_mm()?;
        let solid = kernel::transformed(&self.solid, &matrix)?;
        Ok(Body { name: self.name.clone(), solid })
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

/// One use of a body in the file's world frame: an assembly that places a part twice gives two
/// placements of the same body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The body placed (index into [`StepRead::bodies`]).
    pub body: usize,
    /// Where it goes, from the body's own representation frame to the file's world frame.
    pub transform: Transform,
}

/// What [`read_step`] found: every body it read, in ascending entity-number order, where the
/// file places each, and every solid item it refused. For a file written by [`write_step`] the
/// body order is the order written.
#[derive(Debug, Default)]
pub struct StepRead {
    /// The bodies read, in ascending entity-number order, each in its own representation's frame.
    pub bodies: Vec<Body>,
    /// Every use of a body: those its product structure places, in the order the assembly tree
    /// lists them, then each body no product places, once and where it is.
    pub placements: Vec<Placement>,
    /// The solid items refused, with their reasons.
    pub refused: Vec<RefusedItem>,
}

impl StepRead {
    /// One body for each placement, moved into the file's world frame, in placement order.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] when a placement names no body of this read; otherwise the
    /// failures of [`Body::transformed`].
    pub fn placed_bodies(&self) -> Result<Vec<Body>> {
        self.placements.iter().map(|p| self.placed(p)).collect()
    }

    /// The box of every placed body in the file's world frame; `None` when nothing was read.
    /// Each body is moved before it is measured, so a turned body gives the box of its turned
    /// surfaces, not the turned box of the body.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] when a placement names no body of this read; otherwise the
    /// failures of [`Body::transformed`] and [`Body::bounding_box`].
    pub fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        let mut out: Option<BoundingBox> = None;
        for placement in &self.placements {
            let b = self.placed(placement)?.bounding_box()?;
            out = Some(out.map_or(b, |o| o.union(&b)));
        }
        Ok(out)
    }

    /// The body `placement` names, moved by it.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidParams`] when the placement names no body of this read; otherwise the
    /// failures of [`Body::transformed`].
    fn placed(&self, placement: &Placement) -> Result<Body> {
        let body = self
            .bodies
            .get(placement.body)
            .ok_or_else(|| GeometryError::new(ErrorCode::InvalidParams, format!("placement of body {} out of range", placement.body)))?;
        body.transformed(&placement.transform)
    }
}

/// Reads the solids and surface models of a STEP file (AP203, AP214 or AP242) in the units its
/// representation context states. Each body stays in its own representation's frame;
/// [`StepRead::placements`] lists where the file's product structure (parts, assemblies and
/// their placements, mapped items) puts it, and [`StepRead::placed_bodies`] applies them.
///
/// A solid that cannot be read is listed in [`StepRead::refused`] and the others are still read.
/// Reading has no side effects.
///
/// # Errors
///
/// [`ErrorCode::ModelUnreadable`] when the bytes are not UTF-8 text or not an ISO 10303-21
/// exchange structure, its product structure is malformed (a cycle, a broken reference, nesting
/// deeper than 32 levels or more than 100,000 placed instances), or a placement is not finite or
/// moves a body more than [`MAX_LENGTH_NM`] from the origin;
/// [`ErrorCode::KernelUnavailable`] without a kernel;
/// [`ErrorCode::KernelInternal`] when the reader panics.
pub fn read_step(bytes: &[u8]) -> Result<StepRead> {
    let text = std::str::from_utf8(bytes).map_err(|e| GeometryError::new(ErrorCode::ModelUnreadable, format!("STEP text is not UTF-8: {e}")))?;
    let read = kernel::read(text)?;
    let bodies = read.bodies.into_iter().map(|(name, solid)| Body { name, solid }).collect();
    let placements = read
        .placements
        .iter()
        .map(|(body, matrix)| {
            let transform = Transform::from_matrix_mm(matrix)
                .ok_or_else(|| GeometryError::new(ErrorCode::ModelUnreadable, format!("the file places body {body} outside the supported range")))?;
            Ok(Placement { body: *body, transform })
        })
        .collect::<Result<_>>()?;
    Ok(StepRead { bodies, placements, refused: read.refused })
}

/// Writes `bodies` as one part named `part_name` in an AP242 STEP file, exactly as the kernel
/// holds them, without colours. The output depends only on the input: the header carries no time
/// stamp, so writing the same bodies twice gives the same bytes. Body names are written
/// unchanged, including spaces, quotes, backslashes and non-ASCII characters. The part name is
/// written as given except that trailing `.step` suffixes and then trailing `.stp` suffixes
/// (in any case) are dropped, and a part name that is then empty or only spaces is written as
/// `Part`. A body with an empty name
/// reads back with its part's name.
///
/// # Errors
///
/// [`ErrorCode::InvalidParams`] when `bodies` is empty or the part name or a body name has an
/// ASCII control character, which STEP strings cannot carry; [`ErrorCode::OperationFailed`] when the
/// writer refuses a body; [`ErrorCode::KernelUnavailable`] without a kernel;
/// [`ErrorCode::KernelInternal`] when the kernel panics.
pub fn write_step(part_name: &str, bodies: &[Body]) -> Result<Vec<u8>> {
    if bodies.is_empty() {
        return Err(GeometryError::new(ErrorCode::InvalidParams, "there are no bodies to write"));
    }
    check_name("part name", part_name)?;
    for body in bodies {
        check_name("body name", &body.name)?;
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

/// Refuses a name that STEP cannot carry unchanged: its strings have no ASCII control characters
/// (ISO 10303-21 allows only printable characters in a string), and the writer would drop them.
fn check_name(what: &str, name: &str) -> Result<()> {
    if name.chars().any(|c| c.is_ascii_control()) {
        return Err(GeometryError::new(ErrorCode::InvalidParams, format!("the {what} has a control character")));
    }
    Ok(())
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

// Two attributes, not cfg(all(..)): Clippy treats a module as test code only under a plain
// #[cfg(test)], and the test-only lint allowances depend on it.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
#[cfg(test)]
#[cfg(target_arch = "wasm32")]
mod wasm_tests;
