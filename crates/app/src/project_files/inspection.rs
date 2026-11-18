//! What live clients learn about the project's file: its location, the running job and how the
//! last one ended.

use pcbcraft_engine::Session;
use serde_json::{Value, json};

use super::ProjectFiles;

/// How a finished file job ended: `succeeded`, `failed` or `cancelled`, with the path it
/// opened or saved when known and the failure shown to the person.
#[derive(Debug)]
pub(super) struct Finished {
    pub(super) operation: &'static str,
    pub(super) status: &'static str,
    pub(super) path: Option<String>,
    pub(super) error: Option<String>,
}

impl ProjectFiles {
    /// Report the open project's location and format, the running file operation and how the
    /// last one ended, so live clients can follow a save or open to completion.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn inspect(&self, session: &Session) -> Value {
        #[cfg(not(target_arch = "wasm32"))]
        let location = self
            .saved
            .as_ref()
            .filter(|_| self.has_location(session))
            .map(|saved| json!({"path": saved.target.path().display().to_string(), "format": saved.target.format()}));
        #[cfg(target_arch = "wasm32")]
        let location: Option<Value> = {
            let _ = session;
            None
        };
        json!({
            "location": location,
            "operation": self.job.as_ref().map(|job| json!({"kind": job.operation, "destination": job.destination})),
            "last": self.last.as_ref().map(|last| json!({"operation": last.operation, "status": last.status, "path": last.path, "error": last.error})),
        })
    }
}
