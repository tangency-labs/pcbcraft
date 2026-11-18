//! Notification of committed project changes for hosts that keep their own record of them,
//! such as a crash-recovery journal. Observers see the same commands from the GUI, the CLI
//! and MCP through shared session notification hooks, including [`crate::Session::apply`]
//! and prepared copper commits.

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

    /// A file open installed a new history root, even if its project ID matches the last one.
    /// By default it is recorded just like attachment or save metadata adoption.
    fn replaced(&self, project: Arc<Project>) {
        self.opened(project);
    }

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
    struct Log(Mutex<Vec<(String, u64)>>, Mutex<Vec<Value>>);

    impl EditObserver for Log {
        fn opened(&self, project: Arc<Project>) {
            self.0.lock().unwrap().push((format!("opened {}", project.name), 0));
        }

        /// Keep parameters as received, separately from the ordered root/command events.
        fn committed(&self, edit: CommittedEdit) {
            self.0.lock().unwrap().push((edit.command, edit.revision));
            self.1.lock().unwrap().push(edit.params);
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
        let refused = session.apply(PLACE_SYMBOL, &stale).unwrap_err();
        assert!(matches!(&refused, crate::CommandError::StaleRevision { current } if *current == created), "{refused}");
        assert!(refused.to_string().starts_with("project changed"), "{refused}");
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

    /// A prepared copper edit is reported as its command with its parameters once committed, and
    /// save metadata adopted into the live project is reported as a new root.
    #[test]
    fn copper_commits_and_adopted_saves_are_reported() {
        use crate::board::track::{
            ADD_TRACKS,
            tests::{copper_layers, routed_fixture, view},
        };
        let mut session = routed_fixture();
        let log = Arc::new(Log::default());
        session.observe_edits(Some(log.clone()));
        let params = json!({"board":view(&session).target.board,"layer":copper_layers(&session)[0],"width_nm":250_000,
            "points":[{"x_nm":-20_000_000,"y_nm":-20_000_000},{"x_nm":-10_000_000,"y_nm":-20_000_000}],"expected_revision":session.revision()});
        let mut job = session.prepare_copper_edit(ADD_TRACKS, &params).unwrap();
        while !job.step(1024) {}
        let revision = session.commit_copper_edit(job.take_result().unwrap().unwrap()).unwrap();
        let original = session.snapshot().unwrap();
        let saved = Arc::new(Project { name: "Saved".to_owned(), ..Project::clone(&original) });
        assert!(session.adopt_saved_metadata(&original, &saved));
        assert!(!session.adopt_saved_metadata(&original, &saved));
        assert_eq!(*log.1.lock().unwrap(), [params]);
        let log = log.0.lock().unwrap().clone();
        let name = original.name.clone();
        assert_eq!(log, [(format!("opened {name}"), 0), (ADD_TRACKS.to_owned(), revision), ("opened Saved".to_owned(), 0)]);
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

    /// Synchronous copper dispatch reports each publication once; no-ops and rejections are
    /// silent, while direct prepared commits retain their own single notification.
    #[test]
    fn synchronous_copper_and_direct_prepared_edits_report_once() {
        use crate::board::track::tests::{routed_fixture, view};
        let mut session = routed_fixture();
        let log = Arc::new(Log::default());
        session.observe_edits(Some(log.clone()));
        let add = json!({"at":{"x_nm":-20_000_000,"y_nm":-20_000_000},"expected_revision":session.revision()});
        let added = session.apply(crate::ADD_VIA, &add).unwrap();
        let id = view(&session).vias()[0].id;
        let registry = session.project().unwrap().inspect_net_registry().unwrap();
        let set = json!({"vias":[id],"target":{"kind":"derived","anchor":registry["nets"][0]["anchor"]},"expected_revision":session.revision()});
        let assigned = session.apply(crate::SET_VIA_NET, &set).unwrap();
        let expected = vec![
            (format!("opened {}", session.project().unwrap().name), 0),
            (crate::ADD_VIA.to_owned(), added),
            (crate::SET_VIA_NET.to_owned(), assigned),
        ];
        assert_eq!(*log.0.lock().unwrap(), expected);
        assert_eq!(*log.1.lock().unwrap(), [add, set.clone()]);
        let mut noop = set;
        noop["expected_revision"] = json!(session.revision());
        assert_eq!(session.apply(crate::SET_VIA_NET, &noop).unwrap(), assigned);
        assert!(session.apply(crate::SET_VIA_NET, &json!({"vias":[],"automatic":true,"expected_revision":assigned})).is_err());
        assert_eq!(*log.0.lock().unwrap(), expected);
        let automatic = json!({"vias":[id],"automatic":true,"expected_revision":assigned});
        let mut job = session.prepare_copper_edit(crate::SET_VIA_NET, &automatic).unwrap();
        while !job.step(1024) {}
        let cleared = session.commit_copper_edit(job.take_result().unwrap().unwrap()).unwrap();
        assert_eq!(log.0.lock().unwrap().last(), Some(&(crate::SET_VIA_NET.to_owned(), cleared)));
        assert_eq!(log.0.lock().unwrap().len(), expected.len() + 1);
        let mut noop = automatic;
        noop["expected_revision"] = json!(cleared);
        let mut job = session.prepare_copper_edit(crate::SET_VIA_NET, &noop).unwrap();
        while !job.step(1024) {}
        assert_eq!(session.commit_copper_edit(job.take_result().unwrap().unwrap()).unwrap(), cleared);
        assert_eq!(log.0.lock().unwrap().len(), expected.len() + 1);
    }
}
