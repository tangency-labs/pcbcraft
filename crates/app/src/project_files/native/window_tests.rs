//! File-worker completion and the single native project-close decision.

use super::*;
use crate::{
    Dialog, PcbApp,
    project_files::Job,
    unsaved::{Then, Waiting},
};
use pcbcraft_engine::PLACE_SYMBOL;
use serde_json::json;
use std::sync::mpsc;

/// An edited project with two views, constructed exclusively through shared commands.
fn app() -> PcbApp {
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({"name":"Windows"})).unwrap();
    app.session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()})).unwrap();
    app.views.sync(&app.session);
    app.views.detach_view(app.views.active.id());
    app
}

/// Deliver a worker result through production polling, then evaluate the waiting close request.
fn finish(app: &mut PcbApp, result: Result<Outcome, String>) {
    let (send, receiver) = mpsc::channel();
    app.files.job = Some(Job { receiver, revision: app.session.revision() });
    send.send(result).unwrap();
    app.files.poll(&mut app.session, &egui::Context::default());
    app.finish_waiting(&egui::Context::default());
}

/// Deliver explicit Quit, which ends the project even while several editor windows survive.
fn request_close(app: &mut PcbApp, ctx: &egui::Context) {
    app.request_project_close(ctx);
    let mut input = egui::RawInput::default();
    input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let mut output = ctx.run_ui(input, |ui| app.handle_close_request(ui.ctx()));
    output.textures_delta.clear();
}

/// Save-and-close may exit only after the currently edited snapshot is durably saved. Later
/// edits remain dirty, and every refused/cancelled/disconnected save leaves both views alive.
#[test]
fn close_waits_for_current_save_and_keeps_failed_or_cancelled_saves_open() {
    let dir = tempfile::tempdir().unwrap();
    for later_edit in [false, true] {
        let mut app = app();
        let snapshot = app.session.snapshot().unwrap();
        let result = save_new(snapshot, None, SaveFormat::Folder, dir.path().join(if later_edit { "earlier" } else { "current" }));
        assert!(result.is_ok());
        if later_edit {
            app.session.apply("project.rename", &json!({"name":"Later edit"})).unwrap();
        }
        app.waiting = Some(Waiting::ForSave(Then::Close));
        finish(&mut app, result);
        assert_eq!(app.close_allowed, !later_edit);
        assert_eq!(app.files.dirty(&app.session), later_edit);
        assert!(!app.views.detached_hosts().is_empty());
        assert!(matches!(app.dialog, Dialog::None));
    }
    for result in [Ok(Outcome::Cancelled), Err("disk full".to_owned())] {
        let mut app = app();
        let before = app.session.snapshot().unwrap();
        app.waiting = Some(Waiting::ForSave(Then::Close));
        finish(&mut app, result);
        assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
        assert!(matches!(app.dialog, Dialog::None));
        assert!(!app.close_allowed);
        assert_eq!(app.waiting, None);
        assert!(!app.views.detached_hosts().is_empty());
    }
    let mut app = app();
    let (send, receiver) = mpsc::channel();
    app.files.job = Some(Job { receiver, revision: app.session.revision() });
    app.waiting = Some(Waiting::ForSave(Then::Close));
    drop(send);
    assert!(matches!(app.files.poll(&mut app.session, &egui::Context::default()), Some(crate::project_files::Update::Error(_))));
    app.finish_waiting(&egui::Context::default());
    assert!(matches!(app.dialog, Dialog::None));
    assert!(!app.close_allowed);
}

/// Closing while a worker is still saving waits without abandoning it or making a second job.
/// The saved project reopens through the normal decoder, retaining the shared document.
#[test]
fn close_during_a_file_job_waits_and_reopen_rebinds_both_views() {
    let mut app = app();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let result = save_new(app.session.snapshot().unwrap(), None, SaveFormat::Folder, root.clone()).unwrap();
    let (send, receiver) = mpsc::channel();
    app.files.job = Some(Job { receiver, revision: app.session.revision() });
    let ctx = egui::Context::default();
    request_close(&mut app, &ctx);
    app.finish_waiting(&ctx);
    assert_eq!(app.waiting, Some(Waiting::ToDecide(Then::Close)));
    assert!(app.files.busy());
    assert!(matches!(app.dialog, Dialog::None));
    send.send(Ok(result)).unwrap();
    app.files.poll(&mut app.session, &ctx);
    app.finish_waiting(&ctx);
    assert!(app.close_allowed);

    let old = app.views.active.input(&app.session).unwrap();
    let detached = app.views.hosts[&app.views.detached_hosts()[0]].focused;
    let other_old = app.views.view(detached).unwrap().input(&app.session).unwrap();
    let loaded = load(&root).unwrap();
    assert_eq!(
        pcbcraft_engine::persistence::encode_documents(loaded.snapshot.clone()).unwrap(),
        pcbcraft_engine::persistence::encode_documents(app.session.snapshot().unwrap()).unwrap()
    );
    app.session.open_snapshot(loaded.snapshot);
    app.views.sync(&app.session);
    assert!(!app.views.active.accepts(old, &app.session));
    assert!(!app.views.view(detached).unwrap().accepts(other_old, &app.session));
    assert!(app.views.active.shares_sheet(app.views.view(detached).unwrap(), &app.session));
}

/// A slow picker or worker must not trap the user in shutdown. Withdrawing the close request
/// keeps the same worker alive, and its later result is handled by the usual file controller.
#[test]
fn waiting_for_a_file_job_allows_canceling_close_without_abandoning_the_job() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = app();
    let before = app.session.snapshot().unwrap();
    let (send, receiver) = mpsc::channel();
    app.files.job = Some(Job { receiver, revision: app.session.revision() });
    app.waiting = Some(Waiting::ToDecide(Then::Close));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), PcbApp::default());
    h.run();
    *h.state_mut() = app;
    h.run();
    h.get_by_label("Cancel close").click();
    h.step();
    assert!(!h.state().project_action_pending());
    assert!(h.state().files.busy());
    assert!(!h.state().views.detached_hosts().is_empty());
    send.send(Ok(Outcome::Cancelled)).unwrap();
    h.run();
    assert!(!h.state().files.busy());
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Repeated OS requests during an answered Save/Don't Save must retain that decision while
/// the existing worker finishes; they neither replace its receiver nor create another save.
#[test]
fn repeated_close_preserves_the_pending_save_decision() {
    for waiting in [Waiting::ForSave(Then::Close), Waiting::Discarding(Then::Close)] {
        let mut app = app();
        let (send, receiver) = mpsc::channel();
        app.files.job = Some(Job { receiver, revision: app.session.revision() });
        app.waiting = Some(waiting.clone());
        let ctx = egui::Context::default();
        request_close(&mut app, &ctx);
        assert_eq!(app.waiting, Some(waiting.clone()));
        assert!(app.files.busy());
        send.send(Ok(Outcome::Cancelled)).unwrap();
        app.files.poll(&mut app.session, &ctx);
        app.finish_waiting(&ctx);
        assert_eq!(app.close_allowed, matches!(waiting, Waiting::Discarding(_)));
        assert!(matches!(app.dialog, Dialog::None));
    }
}
