//! The panic guard around kernel calls.
//!
//! Adapted from SolidCraft's `guard` in `crates/kernel/src/lib.rs` at revision
//! `73bdb6cdd39efc3899fb54e7217289a630a89efe` (MIT OR Apache-2.0, copyright (c) 2026 SolidCraft
//! contributors). PCBcraft's version returns [`ErrorCode::KernelInternal`] and keeps no
//! per-thread depth, since no crash reporter here asks whether a call is guarded.

use crate::{ErrorCode, GeometryError, Result};

/// Runs `f`, a sequence of kernel calls named `what`, and turns a panic inside it into
/// [`ErrorCode::KernelInternal`] with the panic's message. `f`'s own result is returned unchanged.
///
/// The kernel's state after a caught panic belongs to the kernel: solidcore's session replaces its
/// kernel when one of its calls panics, and `f` must not hand out partly built bodies.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn guard<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let message =
                payload.downcast_ref::<&str>().map(|s| (*s).to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
            log::warn!("kernel panic in {what}: {message}");
            Err(GeometryError::new(ErrorCode::KernelInternal, format!("{what}: {message}")))
        }
    }
}
