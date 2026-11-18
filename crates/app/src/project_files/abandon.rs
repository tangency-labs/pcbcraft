//! Stop waiting for a file job stalled on an unresponsive disk, so the editor and its live
//! clients can carry on (#409).

use super::{Job, ProjectFiles, inspection::Finished};

impl ProjectFiles {
    /// Whether the running job reads or writes a known path, so [`Self::abandon`] can release
    /// it. A job waiting on the native picker has none and ends when the person answers it.
    pub(crate) fn abandonable(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.destination.is_some())
    }

    /// Stop waiting for the running job with a known path and return the notice to show. Its
    /// receiver is dropped, so a late open is never adopted and a late save never clears the
    /// unsaved state. The worker may still finish on disk; the save locks and on-disk checks
    /// keep a later save from overwriting it unnoticed. Inspection reports the job `abandoned`.
    pub(crate) fn abandon(&mut self) -> Result<String, String> {
        if !self.abandonable() {
            let reason = if self.busy() {
                "The file operation is waiting for the file picker; answer it in the editor."
            } else {
                "No file operation is running."
            };
            return Err(reason.to_owned());
        }
        let Job { operation, destination, .. } = self.job.take().ok_or("No file operation is running.")?;
        let notice = if operation == "save" {
            "Stopped waiting for the save. It may still finish on disk; the project stays unsaved."
        } else {
            "Stopped waiting for the open. The current project is unchanged."
        };
        self.completed += 1;
        self.last = Some(Finished { operation, status: "abandoned", path: destination, error: Some(notice.to_owned()) });
        Ok(notice.to_owned())
    }
}
