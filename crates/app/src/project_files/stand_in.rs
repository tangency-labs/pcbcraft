//! Test-only control of desktop file jobs: saves wait for the test instead of a native picker.

use std::sync::mpsc;

use super::{Job, Outcome, ProjectFiles, StandIn, StandInSave, native};

impl ProjectFiles {
    /// Send later saves to a stand-in the test finishes, instead of a worker with a native picker.
    pub(crate) fn use_stand_in(&mut self) {
        self.stand_in = Some(StandIn::default());
    }

    /// Deliver the real loader's result through the ordinary completion path without an OS picker.
    pub(crate) fn open_path_for_test(&mut self, path: &std::path::Path, revision: u64) {
        let (send, receiver) = mpsc::channel();
        let _ = send.send(native::load(path).map(Outcome::Opened));
        self.job = Some(Job { receiver, revision, operation: "open", destination: Some(path.display().to_string()) });
    }

    /// Whether a save is waiting for the test to finish it.
    pub(crate) fn stand_in_waiting(&self) -> bool {
        self.stand_in.as_ref().is_some_and(|stand_in| stand_in.pending.is_some())
    }

    /// Finish the waiting save as the worker would: in place when it has a destination, otherwise
    /// as though the person picked `path` as a new project folder.
    pub(crate) fn finish_stand_in(&mut self, path: std::path::PathBuf) {
        let Some(StandInSave { snapshot, previous, format, send }) = self.stand_in.as_mut().and_then(|stand_in| stand_in.pending.take()) else {
            return;
        };
        let result = match previous {
            Some(previous) if format.is_none() => native::save(snapshot, Some(previous), None),
            previous => native::save_new(snapshot, previous.as_ref(), format.unwrap_or_default(), path),
        };
        let _ = send.send(result);
    }

    /// End the waiting save as a dismissed picker (`None`) or a failure with that message.
    pub(crate) fn end_stand_in(&mut self, error: Option<&str>) {
        let Some(StandInSave { send, .. }) = self.stand_in.as_mut().and_then(|stand_in| stand_in.pending.take()) else { return };
        let _ = send.send(error.map_or(Ok(Outcome::Cancelled), |error| Err(error.to_owned())));
    }
}
