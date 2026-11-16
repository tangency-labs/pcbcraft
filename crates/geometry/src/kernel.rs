//! The solidcore implementation of the kernel boundary. Nothing here is public: the crate root
//! converts between these millimetre values and its nanometre types.

use solidcore_step::write::product::{ExportError, Header, solidcore_bodies};

use crate::guard::guard;
use crate::{ErrorCode, GeometryError, RefusedItem, Result};

/// Written into the STEP header as the originating system.
const ORIGINATING_SYSTEM: &str = "PCBcraft";

/// A solidcore body.
pub(crate) struct KernelBody(solidcore_ops::Body);

/// An axis-aligned block from its least corner and positive sizes, in millimetres.
pub(crate) fn cuboid(origin: [f64; 3], size: [f64; 3]) -> Result<KernelBody> {
    guard("cuboid", || solidcore_ops::make_box(origin, size).map(KernelBody).map_err(|e| ops_error("cuboid", e)))
}

/// The body's exact box in millimetres, least corner first.
pub(crate) fn bounds(body: &KernelBody) -> Result<([f64; 3], [f64; 3])> {
    guard("bounding box", || body.0.bounds().map_err(|e| ops_error("bounding box", e)))
}

/// STEP text of `bodies` as one AP242 part named `part_name`.
pub(crate) fn write(part_name: &str, bodies: &[(&str, &KernelBody)]) -> Result<String> {
    guard("STEP write", || {
        let named: Vec<(&str, &solidcore_ops::Body)> = bodies.iter().map(|(name, body)| (*name, &body.0)).collect();
        let header = Header { file_name: part_name.to_string(), originating_system: ORIGINATING_SYSTEM.to_string(), ..Header::default() };
        solidcore_bodies(&named, &header).map_err(export_error)
    })
}

/// The named bodies read from a file, and the solid items refused.
pub(crate) type Read = (Vec<(String, KernelBody)>, Vec<RefusedItem>);

/// The named bodies of a STEP text, and the solid items refused. A panic while one solid is
/// handed to the kernel refuses that solid only.
pub(crate) fn read(text: &str) -> Result<Read> {
    guard("STEP read", || {
        let exchange = solidcore_step::p21::parse(text).map_err(|e| GeometryError::new(ErrorCode::ModelUnreadable, String::from(e)))?;
        let solids = solidcore_step::topology::read_solids(&exchange);
        let mut refused: Vec<RefusedItem> =
            solids.failures.into_iter().map(|f| RefusedItem { entity: f.item, code: reader_refusal(&f.message), message: f.message }).collect();
        let mut bodies = Vec::with_capacity(solids.solids.len());
        for solid in solids.solids {
            let made = guard("STEP solid", || {
                solidcore_ops::Body::from_record(&solid.brep, false).map(|(body, _)| body).map_err(|e| ops_error("STEP solid", e))
            });
            match made {
                Ok(body) => bodies.push((solid.name, KernelBody(body))),
                Err(e) => {
                    let code = match e.code() {
                        ErrorCode::KernelInternal => ErrorCode::KernelInternal,
                        _ => ErrorCode::ModelUnsupportedEntity,
                    };
                    refused.push(RefusedItem { entity: solid.item, code, message: e.message().to_string() });
                }
            }
        }
        refused.sort_by_key(|r| r.entity);
        Ok((bodies, refused))
    })
}

/// A deliberate panic inside a solidcore call, as the kernel's session catches it.
pub(crate) fn crash_test() -> Result<()> {
    guard("crash test", || solidcore_ops::session::crash_test().map_err(|e| ops_error("crash test", e)))
}

/// The reader's refusal of a solid as a code: what it does not support yet, or a malformed file.
fn reader_refusal(message: &str) -> ErrorCode {
    if message.contains("not supported yet") { ErrorCode::ModelUnsupportedEntity } else { ErrorCode::ModelUnreadable }
}

/// A solidcore-ops failure as ours: a panic its session caught stays an internal error.
fn ops_error(what: &str, e: solidcore_ops::Error) -> GeometryError {
    match e {
        solidcore_ops::Error::Internal(message) => GeometryError::new(ErrorCode::KernelInternal, format!("{what}: {message}")),
        solidcore_ops::Error::Argument => GeometryError::new(ErrorCode::InvalidParams, format!("{what}: the kernel refused an argument")),
        other => GeometryError::new(ErrorCode::OperationFailed, format!("{what}: {other}")),
    }
}

/// The STEP writer's refusal as ours.
fn export_error(e: ExportError) -> GeometryError {
    let code = match e {
        ExportError::Nothing => ErrorCode::InvalidParams,
        _ => ErrorCode::OperationFailed,
    };
    GeometryError::new(code, format!("STEP write: {e}"))
}
