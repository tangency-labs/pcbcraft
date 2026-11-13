//! Snapshot authority and progress shared by checker and copper preparation jobs.
use super::view::BoardTarget;
use crate::{Project, Session};
use std::sync::Arc;

/// Runtime authority for a report, independent of display names or persisted net indices.
#[derive(Clone, Debug)]
pub struct Source {
    pub(crate) snapshot: Arc<Project>,
    pub(crate) session: Arc<()>,
    pub(crate) opening: u64,
    pub(crate) revision: u64,
    pub(crate) target: BoardTarget,
    pub(crate) effective: super::rules::Effective,
}

impl Source {
    /// Address of the captured physical board.
    pub fn target(&self) -> BoardTarget {
        self.target
    }
    /// Revision checked, including rule edits and Undo/Redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Reject a different session, project reopening, changed rules or replaced snapshot.
    pub fn is_current(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.session, &session.clearance_identity)
            && self.opening == session.history_root_revision()
            && self.revision == session.revision()
            && session.snapshot().is_some_and(|p| Arc::ptr_eq(&p, &self.snapshot))
            && self.snapshot.board_view(self.target).is_some()
    }
}

/// Monotonic stage progress. Pair counts are work units, not a wall-clock estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub stage: &'static str,
    pub completed: usize,
    pub total: usize,
}
