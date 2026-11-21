//! Desktop recovery of unsaved work and opt-in ordinary timed Save. The editor owns scheduling
//! and project identity; the recorder worker owns encoding, I/O and entry locks.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use pcbcraft_engine::{
    Project, Session,
    edits::{CommittedEdit, EditObserver},
};
use pcbcraft_storage::recovery::{self, JournalRecorder, RecorderExit, RecorderOptions, Retention, Schedule};

use crate::{Dialog, PcbApp};

const HELD_RECHECK: Duration = Duration::from_millis(500);
const RETRY: Duration = Duration::from_secs(5);
const EXIT_BUDGET: Duration = Duration::from_secs(2);
/// Entries left by earlier runs that no running window holds are kept this long, and at most
/// this many, until the startup offer to restore them exists.
const RETENTION: Retention = Retention { max_age: Duration::from_secs(14 * 24 * 60 * 60), max_entries: 16 };

/// A recorder sees only its attached project. File opens invalidate it synchronously, including
/// same-UUID opens; cross-project edits or Undo/Redo also invalidate it before anything is queued.
#[derive(Debug)]
struct ProjectRecorder {
    id: uuid::Uuid,
    root: u64,
    obsolete: AtomicBool,
    writer: JournalRecorder,
}

impl ProjectRecorder {
    /// Fence the outgoing entry on the first notification from a different project.
    fn accepts(&self, project: &Project) -> bool {
        if project.id() != self.id {
            self.obsolete.store(true, Ordering::Relaxed);
        }
        !self.obsolete.load(Ordering::Relaxed)
    }
}

impl EditObserver for ProjectRecorder {
    /// Attachment and save metadata adoption both describe the same project.
    fn opened(&self, project: Arc<Project>) {
        if self.accepts(&project) {
            self.writer.opened(project);
        }
    }

    /// A new history root is never sent to the outgoing recorder.
    fn replaced(&self, _project: Arc<Project>) {
        self.obsolete.store(true, Ordering::Relaxed);
    }

    /// Queue only edits that still belong to this entry's project and history root.
    fn committed(&self, edit: CommittedEdit) {
        if self.accepts(&edit.project) {
            self.writer.committed(edit);
        }
    }
}

/// Recovery state for one editor. At most one recorder (active or retiring) exists at a time.
#[derive(Debug, Default)]
pub(crate) struct Autosave {
    pub(crate) dir: Option<PathBuf>,
    resolved: bool,
    recorder: Option<Arc<ProjectRecorder>>,
    retiring: Option<RecorderExit>,
    snapshots: Schedule,
    saves: Schedule,
    revision: u64,
    took: Duration,
    epoch: Option<Instant>,
    retry_at: Option<Duration>,
    notice: Option<String>,
}

impl Autosave {
    /// Accumulate the longest synchronous revision-changing GUI command since the last tick.
    pub(crate) fn command_took(&mut self, took: Duration) {
        self.took = self.took.max(took);
    }

    /// Control the timer with a monotonic clock independent of egui's latest painted input.
    pub(crate) fn now(&mut self) -> Duration {
        self.epoch.get_or_insert_with(Instant::now).elapsed()
    }

    /// Make the native clock reach the scheduled interval without wall-clock sleeps in tests.
    #[cfg(test)]
    pub(crate) fn advance_native_clock(&mut self, by: Duration) {
        self.epoch = Instant::now().checked_sub(by);
    }

    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Detach notifications, then drain the owned recorder. Only an approved close can request
    /// deletion. Quit waits at most one budget; a worker still draining keeps its recovery entry.
    pub(crate) fn finish(&mut self, session: &mut Session, approved: bool) {
        let deadline = Instant::now() + EXIT_BUDGET;
        session.observe_edits(None);
        if let Some(recorder) = self.recorder.take().and_then(Arc::into_inner) {
            self.retiring = Some(recorder.writer.retire(approved.then_some(deadline)));
        }
        if let Some(retiring) = self.retiring.take() {
            log_retire(retiring.wait(deadline.saturating_duration_since(Instant::now())));
        }
    }
}

/// Errors in retired entries are not errors in the current project's Save operation.
fn log_retire(result: Result<(), String>) {
    if let Err(error) = result {
        log::warn!("recovery: could not finish a recovery entry: {error}");
    }
}

impl PcbApp {
    /// Drive both timers from native logic, not from a painted view. The test harness can supply
    /// a deterministic clock without sleeping for a whole interval.
    pub(crate) fn autosave_tick(&mut self, ctx: &egui::Context, now: Duration) {
        self.follow_project(ctx, now);
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
        if let Some(recorder) = &autosave.recorder {
            let fault = recorder.writer.fault();
            autosave.notice = fault.as_ref().map(|fault| format!("Unsaved work isn’t being kept for recovery: {}", fault.message));
            let removed = fault.as_ref().is_some_and(|fault| fault.removed);
            if removed {
                autosave.retry_at = None;
                autosave.snapshots.taken();
            } else {
                if fault.is_some() {
                    autosave.retry_at.get_or_insert(now.saturating_add(RETRY));
                } else {
                    autosave.retry_at = None;
                }
                let due = autosave.retry_at.map_or_else(|| autosave.snapshots.due(now), |retry| now >= retry);
                if due && let Some(project) = self.session.snapshot() {
                    recorder.writer.snapshot(project);
                    autosave.snapshots.taken();
                    if autosave.retry_at.is_some() {
                        autosave.retry_at = Some(now.saturating_add(RETRY));
                    }
                }
            }
        }
        let save_wait = self.timed_save(ctx, now);
        let autosave = &self.autosave;
        let snapshot_wait = autosave
            .retry_at
            .map(|retry| retry.saturating_sub(now))
            .or_else(|| autosave.recorder.as_ref().and_then(|_| autosave.snapshots.wait(now)));
        if let Some(wait) = [snapshot_wait, save_wait].into_iter().flatten().min() {
            ctx.request_repaint_after(wait.max(HELD_RECHECK));
        }
    }

    /// Run the ordinary Save only for a located dirty project with no other project action.
    /// Recheck once a file operation settles, including after ordinary stale-file refusal.
    fn timed_save(&mut self, ctx: &egui::Context, now: Duration) -> Option<Duration> {
        let wanted = self.preferences.save_to_project_minutes > 0 && self.files.dirty(&self.session) && self.files.has_location(&self.session);
        let free = !self.close_allowed && !self.files.busy() && matches!(self.dialog, Dialog::None) && !self.project_action_pending();
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
            return Some(HELD_RECHECK);
        }
        saves.wait(now)
    }

    /// Detach an outgoing observer before retiring it. If retirement stalls we don't multiply
    /// workers; when it completes, attach to the newest state, not to an obsolete intermediate
    /// project. A failed worker spawn is retried even without a new edit. With snapshots Off no
    /// recorder is attached, and turning them Off removes this window's entry.
    fn follow_project(&mut self, ctx: &egui::Context, now: Duration) {
        let enabled = self.preferences.autosave_minutes > 0;
        let autosave = &mut self.autosave;
        if !autosave.resolved {
            autosave.resolved = true;
            if autosave.dir.is_none() && !cfg!(test) {
                autosave.dir = recovery::default_dir();
            }
        }
        let target = self.session.project().filter(|_| enabled).map(|project| (project.id(), self.session.history_root_revision()));
        if let Some(recorder) = &autosave.recorder {
            if target == Some((recorder.id, recorder.root)) && !recorder.obsolete.load(Ordering::Relaxed) {
                return;
            }
            self.session.observe_edits(None);
            if let Some(recorder) = autosave.recorder.take().and_then(Arc::into_inner) {
                autosave.retiring = Some(recorder.writer.retire(Some(Instant::now() + EXIT_BUDGET)));
            }
            autosave.snapshots.taken();
            autosave.saves.taken();
            autosave.retry_at = None;
        }
        if let Some(retiring) = &autosave.retiring {
            let Some(result) = retiring.poll() else {
                autosave.notice = enabled.then(|| "Recovery resumes when the previous project finishes closing.".into());
                return;
            };
            log_retire(result);
            autosave.retiring = None;
        }
        if !enabled {
            autosave.notice = None;
            autosave.retry_at = None;
            return;
        }
        let (Some(project), Some((id, root)), Some(dir)) = (self.session.project(), target, autosave.dir.clone()) else { return };
        if autosave.retry_at.is_some_and(|retry| now < retry) || self.close_allowed {
            return;
        }
        let entry = recovery::new_entry_id(project);
        let wake = ctx.clone();
        let options = RecorderOptions { coalesce_edits: true, retention: Some(RETENTION) };
        match JournalRecorder::start_with(dir, entry, options, move || wake.request_repaint_of(egui::ViewportId::ROOT)) {
            Ok(writer) => {
                let recorder = Arc::new(ProjectRecorder { id, root, obsolete: AtomicBool::new(false), writer });
                self.session.observe_edits(Some(recorder.clone()));
                autosave.recorder = Some(recorder);
                autosave.retry_at = None;
                autosave.notice = None;
            }
            Err(error) => {
                autosave.notice = Some(format!("Unsaved work isn’t being kept for recovery: {error}"));
                autosave.retry_at = Some(now.saturating_add(RETRY));
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn recovery_entry(&self) -> Option<(PathBuf, String)> {
        self.autosave.recorder.as_ref().map(|recorder| (recorder.writer.dir().to_path_buf(), recorder.writer.id().to_owned()))
    }

    #[cfg(test)]
    pub(crate) fn wait_for_recovery_retirement(&mut self) -> Option<Result<(), String>> {
        self.autosave.retiring.take().map(|retiring| retiring.wait(EXIT_BUDGET))
    }

    #[cfg(test)]
    pub(crate) fn sync_recovery(&self) -> Result<(), String> {
        self.autosave.recorder.as_ref().map_or(Ok(()), |recorder| recorder.writer.sync())
    }
}
