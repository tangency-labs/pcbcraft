//! One source-keyed, nonblocking derived slot shared by immutable history projections.
use super::*;
use crate::{CommandError, Project, Session};
use std::sync::{
    OnceLock, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(super) type Slot = Arc<OnceLock<Result<Arc<Fill>, String>>>;

/// Weak allocation identities prevent ABA while allowing old project history to be released.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    suppressed: bool,
    authority: Arc<()>,
    target: Option<BoardTarget>,
    board: Weak<super::super::Board>,
    components: Weak<BTreeMap<u64, crate::PartInstance>>,
    sheets: Weak<Vec<crate::sheets::SchematicSheet>>,
    nets: Weak<crate::nets::Registry>,
    classes: Weak<crate::net_classes::Catalog>,
    slot: Slot,
    memo: Arc<Memo>,
}
impl Cache {
    /// Preview reads may inspect a different temporary board; they must not replace the live
    /// job's slot or discard a completed result for the actual session snapshot.
    fn matches(&self, project: &Project) -> bool {
        self.target == project.board_target()
            && self.board.as_ptr() == Arc::as_ptr(&project.board)
            && self.components.as_ptr() == Arc::as_ptr(&project.components)
            && self.sheets.as_ptr() == Arc::as_ptr(&project.sheets)
            && self.nets.as_ptr() == Arc::as_ptr(&project.net_registry)
            && self.classes.as_ptr() == Arc::as_ptr(&project.net_classes)
    }
    /// Capture a slot while holding only the short cache lock; calculation runs after release.
    fn slot(&mut self, project: &Project, refresh: bool) -> Slot {
        if refresh {
            self.suppressed = false;
            self.authority = Arc::new(());
        }
        if refresh || !self.matches(project) {
            self.target = project.board_target();
            self.board = Arc::downgrade(&project.board);
            self.components = Arc::downgrade(&project.components);
            self.sheets = Arc::downgrade(&project.sheets);
            self.nets = Arc::downgrade(&project.net_registry);
            self.classes = Arc::downgrade(&project.net_classes);
            self.slot = Arc::default();
        }
        self.slot.clone()
    }
}
impl Project {
    pub(super) fn zone_geometry_memo(&self) -> Arc<Memo> {
        self.board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner).memo.clone()
    }

    /// Derived pour/unfill changes invalidate read-only jobs independently of authored revision.
    pub(crate) fn fill_authority(&self) -> Arc<()> {
        self.board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner).authority.clone()
    }
    /// A slot contains no snapshot reference. An old calculation can finish into its old slot
    /// without replacing the one now addressed by a newer geometry or electrical input.
    fn fill_slot(&self, refresh: bool) -> Slot {
        self.board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner).slot(self, refresh)
    }
}
impl BoardView<'_> {
    /// Whether this board permits derived pouring. Explicit removal suppresses all automatic
    /// and checker-driven fill until Pour copper; zone records and native bytes are untouched.
    pub fn zone_fill_enabled(self) -> bool {
        !self.project().board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner).suppressed
    }

    /// Inspect only already completed current-source fill; canvas calls never start calculation
    /// or wait on a worker. Failed/unavailable fill cannot provide conductive connectivity.
    pub fn filled_zones(self) -> Option<Result<Arc<Fill>, String>> {
        let cache = self.project().board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (!cache.suppressed && cache.matches(self.project())).then(|| cache.slot.get().cloned()).flatten()
    }

    /// Drain the same calculation synchronously for explicit headless checking/refill. UI
    /// owners schedule FillJob instead of calling this from layout or paint.
    pub(crate) fn fill_for_check(self) -> Result<Arc<Fill>, String> {
        if !self.zone_fill_enabled() {
            return Err("Copper pours are removed. Use Pour copper to fill zones.".into());
        }
        let slot = self.project().fill_slot(false);
        if let Some(result) = slot.get() {
            return result.clone();
        }
        let result = compute(self);
        let _ = slot.set(result.clone());
        result
    }
}

/// Captured fill work shares the checker source authority and bounded host worker pool.
/// A result is retained only in its captured slot, outside saved documents and undo history.
pub struct FillJob {
    pub(super) worker_hash: Option<String>,
    pub(super) source: super::super::work::Source,
    pub(super) slot: Slot,
    output: Option<Result<Arc<Fill>, String>>,
    complete: bool,
    canceled: Arc<AtomicBool>,
}
impl Session {
    /// Remove current derived pours and revoke in-flight fill/check/edit authority without
    /// changing zone outlines, project bytes, revision or undo. Only an explicit pour resumes.
    pub(crate) fn unfill_zones(&self, params: &serde_json::Value) -> Result<serde_json::Value, CommandError> {
        self.validate_project_command(super::UNFILL_ZONES, params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(serde_json::Value::as_str))?;
        {
            let mut cache = project.board_fill.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !cache.suppressed {
                cache.suppressed = true;
                cache.authority = Arc::new(());
                cache.slot = Arc::default();
            }
        }
        self.zone_fill_status(target)
    }
    /// Record a terminal host/transport failure only for a still-current source. Ready material
    /// is never overwritten, and obsolete host work cannot poison a replacement document.
    pub fn fail_zone_fill(&self, source: &super::super::work::Source, message: String) -> bool {
        if !source.is_current(self) {
            return false;
        }
        source.snapshot.fill_slot(false).set(Err(message)).is_ok()
    }

    /// Validate the shared refill command, then capture its read-only runtime work for a host.
    pub fn prepare_zone_refill(&self, params: &serde_json::Value) -> Result<FillJob, CommandError> {
        self.validate_project_command(super::REFILL_ZONES, params)?;
        let target = self.open_project()?.resolve_board(params.get("board").and_then(serde_json::Value::as_str))?;
        self.prepare_zone_fill(target, true)
    }
    /// Inspect current source-derived fill state without waiting, calculating or editing.
    pub fn zone_fill_status(&self, target: BoardTarget) -> Result<serde_json::Value, CommandError> {
        let project = self.open_project()?;
        let view = project.board_view(target).ok_or_else(|| CommandError::InvalidParams("Unknown board.".into()))?;
        let fill = if !view.zone_fill_enabled() {
            serde_json::json!({"state":"unfilled"})
        } else {
            match view.filled_zones() {
                None => serde_json::json!({"state":"pending"}),
                Some(Err(message)) => serde_json::json!({"state":"unavailable","message":message}),
                Some(Ok(fill)) => {
                    serde_json::json!({"state":"ready","zones":fill.zones.iter().map(|z|serde_json::json!({"id":z.id,"net":z.net,"layer":z.layer,"islands":z.islands.len(),"dropped":z.dropped})).collect::<Vec<_>>(),"problems":fill.problems})
                }
            }
        };
        Ok(serde_json::json!({"revision":self.revision(),"zone_fill":{"board":target,"result":fill}}))
    }

    /// Headless refill drains the same captured job as GUI/worker hosts. It replaces only a
    /// derived slot and returns the current state without adding undo or dirtying native files.
    pub(crate) fn refill_zones(&self, params: &serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let mut job = self.prepare_zone_refill(params)?;
        let target = job.source().target();
        job.step(1);
        job.take_result().map_err(CommandError::Unavailable)?.ok_or_else(|| CommandError::Unavailable("Zone refill was canceled.".into()))?;
        self.zone_fill_status(target)
    }

    /// Capture one physical board and immutable inputs without encoding project files. Refresh
    /// replaces only the derived slot; it never changes the revision, dirtiness or history.
    pub fn prepare_zone_fill(&self, target: BoardTarget, refresh: bool) -> Result<FillJob, CommandError> {
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("No project is open.".into()))?;
        let view = snapshot.board_view(target).ok_or_else(|| CommandError::InvalidParams("Unknown board.".into()))?;
        if !refresh && !view.zone_fill_enabled() {
            return Err(CommandError::Unavailable("Copper pours are removed. Use Pour copper to fill zones.".into()));
        }
        let effective = view.effective_rules();
        let slot = snapshot.fill_slot(refresh);
        Ok(FillJob {
            worker_hash: None,
            source: super::super::work::Source {
                fill_authority: snapshot.fill_authority(),
                snapshot,
                session: self.clearance_identity.clone(),
                opening: self.history_root_revision(),
                revision: self.revision(),
                target,
                effective,
            },
            slot,
            output: None,
            complete: false,
            canceled: Arc::new(AtomicBool::new(false)),
        })
    }
}
impl FillJob {
    /// A completed source slot needs no browser worker or repeated serialization.
    pub fn is_ready(&self) -> bool {
        self.slot.get().is_some()
    }
    /// Authority used by every host before exposing progress/completion in a particular view.
    pub fn source(&self) -> &super::super::work::Source {
        &self.source
    }
    /// Shared cancellation is sampled between zones and kernel operations, rather than only
    /// after the entire result. Publication authority is revoked by the host immediately.
    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.canceled.clone()
    }
    /// Current stage, without implying a wall-clock bound on a kernel operation.
    pub fn progress(&self) -> super::super::work::Progress {
        super::super::work::Progress { stage: "Filling copper zones", completed: usize::from(self.complete), total: 1 }
    }
    /// Execute on the host's worker. No result is set on cancellation, and stale work can only
    /// finish into its own old slot. Browser hosts must isolate this work from foreground UI.
    pub fn step(&mut self, budget: usize) -> bool {
        if self.complete || budget == 0 {
            return self.complete;
        }
        if self.canceled.load(Ordering::Acquire) || !self.source.fill_current() {
            self.complete = true;
            return true;
        }
        if let Some(result) = self.slot.get() {
            self.output = Some(result.clone());
            self.complete = true;
            return true;
        }
        let result = self
            .source
            .snapshot
            .board_view(self.source.target)
            .ok_or_else(|| "Fill board is unavailable.".to_owned())
            .and_then(|view| super::compute_cancellable(view, Some(&self.canceled)));
        if !self.canceled.load(Ordering::Acquire) && self.source.fill_current() {
            let _ = self.slot.set(result.clone());
            self.output = Some(result);
        }
        self.complete = true;
        true
    }
    /// Take a complete result once; a canceled job has no partial output.
    pub fn take_result(&mut self) -> Result<Option<Arc<Fill>>, String> {
        self.output.take().transpose()
    }
}
