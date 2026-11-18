//! Stop waiting for an open stalled on an unresponsive disk, so the editor and its live clients
//! can carry on (#409). Saves are never abandoned: a late write could land after the editor
//! already reports the project clean, so a stalled save is only escaped by closing.

use std::sync::mpsc::{Receiver, TryRecvError};

use super::{Job, Outcome, ProjectFiles, inspection::Finished};

/// Abandoned opens whose workers may still be running. Each holds a thread blocked in the
/// file system; past this many, a live client's abandon is refused until one of them ends.
const MAX_STALLED: usize = 4;

/// The workers of abandoned opens, kept only to count those still running.
pub(super) type Stalled = Vec<Receiver<Result<Outcome, String>>>;

impl ProjectFiles {
    /// Whether the running job is an open, which [`Self::abandon`] can release.
    pub(crate) fn abandonable(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.operation == "open")
    }

    /// Stop waiting for the running open and return the notice to show. Its receiver moves to
    /// the stalled list, so a late result is never adopted; the current project and history are
    /// unchanged. Inspection reports the job `abandoned`. Refused for saves.
    ///
    /// A live client passes the `file.operation.id` it observed, so a job that replaced it is
    /// never abandoned by mistake, and is refused once [`MAX_STALLED`] abandoned workers still
    /// run. The person closing PCBCraft passes `None`: no ID race exists there, and the limit
    /// must not keep them from quitting.
    pub(crate) fn abandon(&mut self, expected: Option<u64>) -> Result<String, String> {
        match &self.job {
            None => return Err("No file operation is running.".into()),
            Some(_) if expected.is_some_and(|id| id != self.completed + 1) => {
                return Err(format!("File operation {} is no longer running.", expected.unwrap_or_default()));
            }
            Some(job) if job.operation != "open" => {
                return Err(
                    "A save can't be abandoned while it may still be writing. Wait for it, or close PCBCraft without waiting in the editor.".into()
                );
            }
            Some(_) => {}
        }
        self.release_stalled();
        if expected.is_some() && self.stalled.len() >= MAX_STALLED {
            return Err("Too many earlier opens are still stalled. Wait for one to end, or restart PCBCraft.".into());
        }
        let Job { receiver, operation, destination, .. } = self.job.take().ok_or("No file operation is running.")?;
        self.stalled.push(receiver);
        let notice = "Stopped waiting for the open. The current project is unchanged.";
        self.completed += 1;
        self.last = Some(Finished { operation, status: "abandoned", path: destination, error: Some(notice.to_owned()) });
        Ok(notice.to_owned())
    }

    /// Forget abandoned workers that ended, dropping any late result (a whole decoded project)
    /// at once rather than holding it for the session. Called on every frame's poll.
    pub(super) fn release_stalled(&mut self) {
        self.stalled.retain(|worker| matches!(worker.try_recv(), Err(TryRecvError::Empty)));
    }
}
