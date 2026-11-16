//! Notification of committed project changes for hosts that keep their own record of them,
//! such as a crash-recovery journal. Observers see the same commands from the GUI, the CLI
//! and MCP because every edit passes through [`crate::Session::apply`].

use std::sync::Arc;

use serde_json::Value;

use crate::Project;

/// One command that changed the project, reported after the session has adopted its result.
#[derive(Clone, Debug)]
pub struct CommittedEdit {
    /// Catalog id of the command, such as `schematic.place_symbol` or `edit.undo`.
    pub command: String,
    /// The parameters the command was given, as received.
    pub params: Value,
    /// Session revision after the change. It is runtime-only and never repeats within a session.
    pub revision: u64,
    /// The project after the change, shared with the session without copying.
    pub project: Arc<Project>,
}

/// Receives the session's history roots and committed edits, in order.
///
/// Calls are made on the thread that runs the command, while the session is borrowed, so an
/// implementation must return quickly and do any encoding or I/O elsewhere. It cannot refuse
/// or undo the change it is told about.
pub trait EditObserver: Send + Sync + std::fmt::Debug {
    /// A file or recovered snapshot replaced the history root; later edits build on `project`.
    fn opened(&self, project: Arc<Project>);

    /// A command changed the project. No-op commands and queries are not reported.
    fn committed(&self, edit: CommittedEdit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PLACE_SYMBOL, Session};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct Log(Mutex<Vec<(String, u64)>>);

    impl EditObserver for Log {
        fn opened(&self, project: Arc<Project>) {
            self.0.lock().unwrap().push((format!("opened {}", project.name), 0));
        }

        fn committed(&self, edit: CommittedEdit) {
            self.0.lock().unwrap().push((edit.command, edit.revision));
        }
    }

    /// Committed edits, undo and opens are reported in order with the revision they produced;
    /// refused commands, no-ops and queries are not, and detaching stops the reports.
    #[test]
    fn only_committed_changes_are_reported() {
        let log = Arc::new(Log::default());
        let mut session = Session::default();
        session.observe_edits(Some(log.clone()));
        session.apply("project.new", &json!({"name":"Observed"})).unwrap();
        let created = session.revision();
        let stale = json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":created + 100});
        assert!(session.apply(PLACE_SYMBOL, &stale).is_err());
        session.apply("project.rename", &json!({"name":"Observed"})).unwrap();
        session.execute(crate::no_connect::CHECK, &json!({})).unwrap();
        session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":created})).unwrap();
        let placed = session.revision();
        session.apply("edit.undo", &json!({})).unwrap();
        let undone = session.revision();
        let snapshot = session.snapshot().unwrap();
        session.open_snapshot(snapshot);
        session.observe_edits(None);
        session.apply("edit.redo", &json!({})).ok();
        session.apply("project.rename", &json!({"name":"Unobserved"})).unwrap();
        let log = log.0.lock().unwrap().clone();
        assert_eq!(
            log,
            [
                ("project.new".to_owned(), created),
                (PLACE_SYMBOL.to_owned(), placed),
                ("edit.undo".to_owned(), undone),
                ("opened Observed".to_owned(), 0),
            ]
        );
    }

    /// Attaching to a session with an open project reports that project first.
    #[test]
    fn attaching_reports_the_open_project() {
        let log = Arc::new(Log::default());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Open"})).unwrap();
        session.observe_edits(Some(log.clone()));
        assert_eq!(*log.0.lock().unwrap(), [("opened Open".to_owned(), 0)]);
    }
}
