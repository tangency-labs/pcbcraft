use super::*;
use std::sync::Arc;
/// Authoring histories never advance or dirty the project's document session.
#[test]
fn library_documents_are_independent_and_revision_checked() {
    let mut app = PcbApp::default();
    let revision = app.session.revision();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    assert!(app.library_editors.edit(id, 0, Action::Undo).is_err());
    app.library_editors.edit(id, 1, Action::Undo).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert_eq!(app.session.revision(), revision);
    assert!(app.session.project().is_none());
    assert!(app.library_editors.close(id, false).is_err());
    app.library_editors.close(id, true).unwrap();
    let next = app.library_editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    assert!(next > id);
    assert!(app.library_editors.edit(id, 0, Action::Undo).is_err());
}

use egui_kittest::{Harness, kittest::Queryable};
/// The actual compact and expanded forms share one draft; a bad length is retained and cannot
/// change geometry, undo history or the project. Close and Quit both protect unapplied input.
#[test]
fn properties_forms_validate_atomically_and_protect_unsaved_fields() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Pad 1 · 1");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("bad length");
    h.run();
    crate::tests::click(&mut h, "Apply");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert!(h.state().library_editors.documents[&id].pending());
    crate::tests::click(&mut h, "More properties…");
    assert!(matches!(h.state().dialog, Dialog::LibraryProperties(_)));
    h.state_mut().begin_project_close(&Context::default());
    h.run();
    assert!(matches!(h.state().dialog, Dialog::LibraryClose(None)));
    crate::tests::click(&mut h, "Cancel");
    assert!(h.state().library_editors.unsaved());
    crate::tests::click(&mut h, "Reset");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("0.7 mm");
    h.run();
    crate::tests::click(&mut h, "Apply");
    let changed = h.state().library_editors.documents[&id].session.snapshot();
    assert_ne!(changed, before);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    crate::tests::click(&mut h, "Redo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot(), changed);
    crate::tests::click(&mut h, "Layers");
    crate::tests::click(&mut h, "All copper");
    h.get_by_label("Drill (mm)").click();
    h.run();
    h.get_by_label("Drill (mm)").type_text("0.3");
    h.run();
    crate::tests::click(&mut h, "Apply");
    let item = h.state().library_editors.documents[&id].session.snapshot();
    let pcbcraft_engine::authoring::Primitive::Pad { pad } = &item.objects[&1] else { panic!("expected pad") };
    assert_eq!(pad.drill_nm, Some(300_000));
    assert_eq!(h.state().session.revision(), 0);
}
/// Live discovery exposes the same authoring actions, requires library revisions and preserves
/// project authority. Unknown IDs and malformed/extra fields fail before any mutation.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_library_commands_share_the_gui_documents() {
    let mut app = PcbApp::default();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_new", &json!({"kind":"symbol"})).unwrap();
    let id = app.library_editors.active.unwrap();
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"delete","objects":[3]}}))
        .unwrap();
    assert!(app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"undo"}})).is_err());
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":1,"action":{"operation":"undo"}})).unwrap();
    assert!(
        app.run_window_command(
            &ctx,
            "app.library_edit",
            &json!({"document":id,"expected_revision":2,"action":{"operation":"undo","unexpected":true}})
        )
        .is_err()
    );
    let inspected = crate::window_commands::inspect_editor(&app);
    assert_eq!(inspected["library_editors"]["documents"][0]["state"]["revision"], 2);
    assert!(app.run_window_command(&ctx, "app.library_close", &json!({"document":id})).is_err());
    app.run_window_command(&ctx, "app.library_focus", &json!({})).unwrap();
    assert_eq!(app.library_editors.active, None);
    assert!(app.library_editors.unsaved());
    app.run_window_command(&ctx, "app.library_close", &json!({"document":id,"discard":true})).unwrap();
    assert!(!app.library_editors.has_documents());
    assert_eq!(app.session.revision(), 0);
}

/// Exercise real native Close delivery and cancellation while an authoring document owns dirty
/// state. The ordinary UI tests verify decisions; this checks that the OS surface survives them.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "opens a native window; run with an available Wayland display"]
fn native_library_close_preserves_unsaved_authoring() -> eframe::Result {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    use winit::platform::wayland::EventLoopBuilderExtWayland as _;
    struct Native {
        app: PcbApp,
        stage: usize,
        due: Instant,
        started: Instant,
        passed: Arc<AtomicBool>,
    }
    impl eframe::App for Native {
        /// Render the real workspace, deliver Close, then verify cancellation on a later native frame.
        fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            let closing = ctx.input(|input| input.viewport().close_requested());
            self.app.frame(ui);
            assert!(self.started.elapsed() < Duration::from_secs(20), "native authoring close did not progress");
            match self.stage {
                0 if Instant::now() >= self.due => {
                    self.stage = 1;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                1 if closing => {
                    assert!(matches!(self.app.dialog, Dialog::LibraryClose(None)));
                    assert!(self.app.library_editors.unsaved());
                    assert!(!self.app.close_allowed);
                    self.stage = 2;
                    self.due = Instant::now() + Duration::from_millis(300);
                }
                2 if Instant::now() >= self.due => {
                    assert!(!closing, "native Close was not cancelled");
                    assert!(self.app.library_editors.active.is_some());
                    self.passed.store(true, Ordering::SeqCst);
                    self.app.close_allowed = true;
                    self.stage = 3;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                _ => {}
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    let passed = Arc::new(AtomicBool::new(false));
    let evidence = passed.clone();
    eframe::run_native(
        "PCBcraft library close regression",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
            persist_window: false,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_wayland().with_any_thread(true);
            })),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = PcbApp::new(cc);
            app.new_library_item(Kind::Footprint);
            Ok(Box::new(Native { app, stage: 0, due: Instant::now() + Duration::from_millis(500), started: Instant::now(), passed: evidence }))
        }),
    )?;
    assert!(passed.load(Ordering::SeqCst));
    Ok(())
}

/// The rendered canvas selects copper ahead of an enclosing unfilled graphic, commits a move
/// once, and cancels a two-click drawing without consuming history.
#[test]
fn canvas_selection_move_and_cancel_use_library_history() {
    fn click_at(h: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
        h.hover_at(point);
        h.run();
        h.drag_at(point);
        h.run();
        h.drop_at(point);
        h.run();
    }
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    let area = h.get_by_label("Library canvas").rect();
    let start = h.state().library_editors.documents[&id].session.snapshot().objects[&1].anchor();
    let at = h.state().library_editors.documents[&id].camera.screen(start, area);
    click_at(&mut h, at);
    assert_eq!(h.state().library_editors.documents[&id].selected, Some(1));
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Move");
    let target = pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 1_000_000 };
    let at = h.state().library_editors.documents[&id].camera.screen(target, area);
    h.hover_at(at);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    click_at(&mut h, at);
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects[&1].anchor(), target);
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().tool =
        canvas::Tool::Draw { kind: canvas::Place::Outline, start: None, revision: 2 };
    click_at(&mut h, at);
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 2);
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert_eq!(h.state().session.revision(), 0);
}

/// A completed old save cannot clear newer edits, a busy owner's close is refused, and a lost
/// worker releases the slot so file handling remains usable after failure.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_completions_keep_their_owner_and_release_disconnected_workers() {
    let mut editors = Editors::default();
    let id = editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let old = editors.documents[&id].session.snapshot();
    let send = editors.files.stand_in();
    assert!(editors.close(id, true).is_err());
    editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    let other = editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    send.send(Ok(io::Outcome::Saved(id, old))).unwrap();
    editors.poll();
    assert!(!editors.busy());
    assert!(editors.documents[&id].session.dirty());
    assert!(editors.documents[&other].session.dirty());
    let send = editors.files.stand_in();
    drop(send);
    editors.poll();
    assert!(!editors.busy());
    assert!(editors.notice.as_ref().unwrap().contains("worker stopped"));
    let before = editors.documents[&id].session.snapshot();
    let send = editors.files.stand_in();
    send.send(Ok(io::Outcome::Opened(Box::new((*before).clone())))).unwrap();
    editors.poll();
    assert_eq!(editors.documents.len(), 2);
    assert_eq!(editors.documents[&id].session.snapshot(), before);
    assert!(editors.notice.as_ref().unwrap().contains("already open"));
}

/// Explicit source view IDs select the correct immutable definition, preserve project bytes,
/// and focusing a project pane returns from authoring without discarding its documents.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn definition_forks_and_project_focus_keep_document_authority_separate() {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Library fork test"}));
    app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    app.select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
    let project = app.session.snapshot().unwrap();
    let view = app.views.active.id();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_fork_selected", &json!({"kind":"symbol","view":view.0})).unwrap();
    let id = app.library_editors.active.unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().kind, Kind::Symbol);
    assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &project));
    assert!(app.run_window_command(&ctx, "app.library_fork_selected", &json!({"kind":"footprint","view":view.0})).is_err());
    app.run_window_command(&ctx, "view.focus", &json!({"view":view.0})).unwrap();
    assert!(app.library_editors.active.is_none());
    assert!(app.library_editors.documents.contains_key(&id));
    app.run_window_command(&ctx, "app.library_focus", &json!({"document":id})).unwrap();
    app.run_window_command(&ctx, "app.set_home", &json!({"shown":true})).unwrap();
    assert!(app.library_editors.active.is_none());
    assert!(app.home);
    let send = app.library_editors.files.stand_in();
    use pcbcraft_mcp::live::Editor as _;
    assert!(crate::live_control::LiveEditor { app: &mut app, ctx: &ctx }.refusal("project.rename").is_none());
    drop(send);
}

/// Live view actions share selection/navigation handlers while preserving definition bytes,
/// revisions and history; incompatible camera requests fail before changing selection.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_library_view_actions_leave_authored_data_unchanged() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let id = app.library_editors.active.unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"selected":1,"zoom_percent":200,"grid_spacing_nm":50_000})).unwrap();
    assert_eq!(app.library_editors.documents[&id].selected, Some(1));
    assert_eq!(app.library_editors.documents[&id].camera.zoom(), 2.0);
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"selected":999,"zoom_percent":100})).is_err());
    assert_eq!(app.library_editors.documents[&id].camera.zoom(), 2.0);
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"definition":true,"selected":2})).is_err());
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"fit":true,"zoom_percent":100})).is_err());
    assert!(Arc::ptr_eq(&before, &app.library_editors.documents[&id].session.snapshot()));
    assert_eq!(app.library_editors.documents[&id].session.revision(), 0);
}
/// A completed exact save can release the existing close request, while cancel leaves no
/// remembered approval that could close a later dirty document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn completed_save_releases_close_without_reusing_a_cancelled_decision() {
    let mut app = PcbApp::default();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let snapshot = app.library_editors.documents[&id].session.snapshot();
    let ctx = Context::default();
    let send = app.library_editors.files.stand_in();
    app.begin_project_close(&ctx);
    assert!(matches!(app.dialog, Dialog::LibraryClose(None)));
    app.dialog = Dialog::None;
    send.send(Ok(io::Outcome::Saved(id, snapshot))).unwrap();
    app.library_editors.poll();
    assert!(!app.close_allowed);
    app.library_close_dialog(&ctx, None);
    assert!(app.close_allowed);
    let mut app = PcbApp::default();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), true).unwrap();
    app.library_close_dialog(&ctx, Some(id));
    assert!(!app.library_editors.has_documents());
    assert!(!app.close_allowed);
}
