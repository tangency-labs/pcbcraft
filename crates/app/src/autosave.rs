//! Timed recovery snapshots of unsaved work, and the opt-in timed save to the project folder.
//!
//! The recorder attempts to journal committed edits on its worker thread. A snapshot on the
//! Preferences interval replaces the journal with the current state. Encoding or I/O failures
//! can leave notifications unrecorded; a detected removed entry stays disabled. Timed project
//! saves use File › Save's path, including its refusal when the project changed on disk.

use std::{path::PathBuf, sync::Arc, thread::JoinHandle, time::Duration};

use pcbcraft_storage::recovery::{self, JournalRecorder, Schedule};

use crate::{Dialog, PcbApp};

/// Never wake more often than this for a timed save that is due but held back by an open
/// dialog or a running file operation; those end on input or a repaint of their own.
const HELD_RECHECK: Duration = Duration::from_millis(500);

/// The recovery recorder for the open project and the timers for snapshots and saves.
#[derive(Debug, Default)]
pub(crate) struct Autosave {
    /// Where entries are written. `None` keeps recovery off, as in tests that do not set one.
    pub(crate) dir: Option<PathBuf>,
    /// Set once the folder has been looked up, so it is looked up only once.
    resolved: bool,
    /// The recorder attached to the session, and the project it records.
    recorder: Option<(Arc<JournalRecorder>, uuid::Uuid)>,
    snapshots: Schedule,
    saves: Schedule,
    /// The session revision at the last frame; a different one is a change.
    revision: u64,
    /// The longest command that ran since the last frame.
    took: Duration,
    /// The project a recorder could not be started for, so it is tried and logged once.
    failed: Option<uuid::Uuid>,
    /// Recorders of replaced projects being stopped and deleted off the frame; a clean exit
    /// waits for them.
    discarding: Vec<JoinHandle<Result<(), String>>>,
}

impl Autosave {
    /// Note how long a command took; one of at least [`recovery::LONG_OPERATION`] asks for a
    /// snapshot at the next frame whatever the interval.
    pub(crate) fn command_took(&mut self, took: Duration) {
        self.took = self.took.max(took);
    }

    /// Stop recording and attempt to delete this run's entry, for a clean exit: unsaved work
    /// was already saved or dismissed at the close prompt. Cleanup failures are logged.
    pub(crate) fn finish(&mut self, session: &mut pcbcraft_engine::Session) {
        session.observe_edits(None);
        if let Some((recorder, _)) = self.recorder.take() {
            log_discard(discard(recorder));
        }
        for result in self.wait_for_discards() {
            log_discard(result);
        }
    }

    /// Attempt to stop `recorder` and delete its entry off the frame. A stalled worker can
    /// keep this cleanup thread and its handle alive; repeated replacements have no cap.
    /// Finished handles are joined on replacement. Spawn or cleanup failures are logged.
    fn discard_in_background(&mut self, recorder: Arc<JournalRecorder>) {
        let mut pending = Vec::new();
        for handle in self.discarding.drain(..) {
            if handle.is_finished() {
                log_discard(handle.join().unwrap_or_else(|_| Err("The recovery discard stopped unexpectedly.".to_owned())));
            } else {
                pending.push(handle);
            }
        }
        self.discarding = pending;
        match std::thread::Builder::new().name("recovery-discard".into()).spawn(move || discard(recorder)) {
            Ok(handle) => self.discarding.push(handle),
            Err(error) => log::warn!("recovery: could not remove a replaced project's entry: {error}"),
        }
    }

    /// Wait for every background discard and return their results.
    fn wait_for_discards(&mut self) -> Vec<Result<(), String>> {
        self.discarding
            .drain(..)
            .map(|handle| handle.join().unwrap_or_else(|_| Err("The recovery discard stopped unexpectedly.".to_owned())))
            .collect()
    }
}

/// Stop a recorder that is no longer attached to the session and delete its entry. A recorder
/// something else still holds keeps writing and is left alone.
fn discard(recorder: Arc<JournalRecorder>) -> Result<(), String> {
    match Arc::into_inner(recorder) {
        Some(recorder) => recorder.discard(),
        None => Ok(()),
    }
}

/// Log a failed discard; the entry may remain wholly or partly in the store.
fn log_discard(result: Result<(), String>) {
    if let Err(error) = result {
        log::warn!("recovery: could not remove a finished entry: {error}");
    }
}

/// The per-user recovery folder. Tests leave it unset unless they give one, so they never write
/// in the person's own folder.
fn default_dir() -> Option<PathBuf> {
    if cfg!(test) { None } else { recovery::default_dir() }
}

impl PcbApp {
    /// Once a frame: keep the recorder on the open project, take a snapshot or start a save when
    /// one is due, and ask to be woken in time for the next.
    pub(crate) fn autosave_frame(&mut self, ctx: &egui::Context) {
        let now = Duration::from_secs_f64(ctx.input(|input| input.time).max(0.0));
        self.follow_project();
        let autosave = &mut self.autosave;
        autosave.snapshots.set_minutes(self.preferences.autosave_minutes);
        autosave.saves.set_minutes(self.preferences.save_to_project_minutes);
        let revision = self.session.revision();
        let took = std::mem::take(&mut autosave.took);
        if revision != autosave.revision {
            autosave.revision = revision;
            autosave.snapshots.long_operation(took, now);
            autosave.saves.changed(now);
        }
        if autosave.snapshots.due(now) {
            if let (Some((recorder, _)), Some(project)) = (&autosave.recorder, self.session.snapshot()) {
                recorder.snapshot(project);
            }
            autosave.snapshots.taken();
        }
        let save_wait = self.timed_save(ctx, now);
        let wait = [self.autosave.snapshots.wait(now), save_wait].into_iter().flatten().min();
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait.max(HELD_RECHECK));
        }
    }

    /// Start the timed save when it is on and due and nothing else is in the way; return how
    /// long until it is due. A project that is saved, Untitled, or not to be saved automatically
    /// has nothing pending. A refused save leaves the project dirty, and the next try comes one
    /// interval later.
    fn timed_save(&mut self, ctx: &egui::Context, now: Duration) -> Option<Duration> {
        let wanted = self.preferences.save_to_project_minutes > 0 && self.files.dirty(&self.session) && self.files.has_location(&self.session);
        let free = !self.files.busy() && matches!(self.dialog, Dialog::None) && !self.project_action_pending();
        let saves = &mut self.autosave.saves;
        if !wanted {
            saves.taken();
            return None;
        }
        if !saves.is_pending() {
            saves.changed(now);
        }
        if free && saves.due(now) {
            saves.taken();
            self.save_project_file(None, ctx);
            return None;
        }
        saves.wait(now)
    }

    /// Attach a recorder to the open project, replacing the one for a project that was replaced.
    /// Attempt to delete the old entry off the frame after the unsaved-work prompt. Spawn or
    /// cleanup failures can leave the entry in the store and are logged.
    fn follow_project(&mut self) {
        let autosave = &mut self.autosave;
        if !autosave.resolved {
            autosave.resolved = true;
            if autosave.dir.is_none() {
                autosave.dir = default_dir();
            }
        }
        let Some(project) = self.session.project() else { return };
        let id = project.id();
        if autosave.recorder.as_ref().is_some_and(|(_, recorded)| *recorded == id) || autosave.failed == Some(id) {
            return;
        }
        let Some(dir) = autosave.dir.clone() else { return };
        let entry = recovery::new_entry_id(project);
        self.session.observe_edits(None);
        if let Some((old, _)) = autosave.recorder.take() {
            autosave.discard_in_background(old);
        }
        match JournalRecorder::start(dir, entry) {
            Ok(recorder) => {
                let recorder = Arc::new(recorder);
                self.session.observe_edits(Some(recorder.clone()));
                autosave.recorder = Some((recorder, id));
                autosave.failed = None;
            }
            Err(error) => {
                log::warn!("recovery: unsaved work is not being recorded: {error}");
                autosave.failed = Some(id);
            }
        }
    }

    /// The recovery entry recording the open project, for tests.
    #[cfg(test)]
    pub(crate) fn recovery_entry(&self) -> Option<(PathBuf, String)> {
        self.autosave.recorder.as_ref().map(|(recorder, _)| (recorder.dir().to_path_buf(), recorder.id().to_owned()))
    }

    /// Wait for the background discards of replaced projects, for tests.
    #[cfg(test)]
    pub(crate) fn wait_for_recovery_discards(&mut self) -> Vec<Result<(), String>> {
        self.autosave.wait_for_discards()
    }

    /// Wait until the recorder has written everything queued so far, for tests.
    #[cfg(test)]
    pub(crate) fn sync_recovery(&self) -> Result<(), String> {
        self.autosave.recorder.as_ref().map_or(Ok(()), |(recorder, _)| recorder.sync())
    }
}
