//! Read-only board inspection hosted in an ordinary editor pane.

#[cfg(not(target_arch = "wasm32"))]
mod copper;
#[cfg(not(target_arch = "wasm32"))]
mod gpu;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::{Browse, State, Worker};

/// The browser build keeps the workspace unavailable until its geometry backend is qualified.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(super) struct State;

#[cfg(target_arch = "wasm32")]
impl State {
    /// Preserve the placeholder type in the shared pane lifecycle.
    pub(super) fn fork(&self) -> Self {
        Self
    }
}
