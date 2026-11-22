//! Recovery snapshots are off until chosen, then go to the per-user recovery folder and never
//! touch the project; the timed save to the project is off until chosen, then saves through the ordinary Save and is refused
//! like it when the folder changed on disk. Time is driven through the frame clock, so nothing
//! here waits a real minute.

use std::{collections::BTreeMap, path::Path};

use pcbcraft_engine::{PLACE_SYMBOL, Session, persistence::encode_documents, schematic::GRID_NM};

use super::*;

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

/// Run frames starting at `seconds` on the frame clock.
fn run_at(h: &mut Harness<'_, PcbApp>, seconds: f64) {
    h.input_mut().time = Some(seconds);
    h.run();
}

/// Place a resistor through the shared command, as a click on the canvas would.
fn place(h: &mut Harness<'_, PcbApp>, x: i64) {
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x * GRID_NM,"y_nm":0,"expected_revision":revision}));
    assert_eq!(h.state().message, None);
}

/// The recovery snapshot's documents, read straight from the entry's `.pc` file.
fn snapshot_documents(h: &Harness<'_, PcbApp>) -> pcbcraft_engine::persistence::Documents {
    h.state().sync_recovery().unwrap();
    let (dir, id) = h.state().recovery_entry().unwrap();
    let snapshot_only = tempfile::tempdir().unwrap();
    std::fs::copy(dir.join(format!("{id}.pc")), snapshot_only.path().join(format!("{id}.pc"))).unwrap();
    pcbcraft_storage::recovery::recover(snapshot_only.path(), &id).unwrap().documents
}

/// Run frames until the running file operation has finished.
fn finish_file_job(h: &mut Harness<'_, PcbApp>) {
    for _ in 0..600 {
        if !h.state().files.busy() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run();
    }
    panic!("the save did not finish");
}

/// The done test for timed snapshots and saves, on a project folder saved through the shared
/// commands and the folder codec.
#[test]
fn snapshots_never_touch_the_project_and_the_timed_save_is_opt_in() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let recovery = work.path().join("recovery");
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Autosave"})).unwrap();
    session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(session.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();

    let mut h = harness();
    h.state_mut().autosave.dir = Some(recovery.clone());
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), revision);
    run_at(&mut h, 1.0);
    assert_eq!(h.state().session.project().unwrap().name, "Autosave");
    let opened = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let saved = tree(&root);
    assert_eq!(h.state().preferences.autosave_minutes, 0, "snapshots are off by default");
    assert_eq!(h.state().preferences.save_to_project_minutes, 0, "saving to the project is off by default");
    assert!(h.state().recovery_entry().is_none(), "nothing records while snapshots are off");
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"autosave_minutes": 1})).unwrap();
    run_at(&mut h, 70.0);
    assert_eq!(snapshot_documents(&h), opened);

    // A 1-minute interval: nothing before the minute is up, the current state after it.
    run_at(&mut h, 100.0);
    place(&mut h, 20);
    run_at(&mut h, 100.5);
    let edited = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    run_at(&mut h, 150.0);
    assert_eq!(snapshot_documents(&h), opened, "not yet a minute since the edit");
    h.state_mut().native_project_tick(&ctx, std::time::Duration::from_secs(162));
    assert_eq!(snapshot_documents(&h), edited);
    assert_eq!(tree(&root), saved, "a snapshot never writes in the project");
    assert!(!recovery.starts_with(&root));

    // Save to project Off: however long it waits, the project stays as saved and dirty.
    run_at(&mut h, 1000.0);
    assert_eq!(tree(&root), saved);
    assert!(h.state().files.dirty(&h.state().session));
    assert!(!h.state().files.busy());

    // Save to project every minute: the ordinary Save runs once the minute is up.
    let error = h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"save_to_project_minutes": 2})).unwrap_err();
    assert!(error.contains("must be 0 (Off), 1, 5 or 10 minutes"), "{error}");
    h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"save_to_project_minutes": 1})).unwrap();
    assert_eq!(crate::window_commands::inspect_editor(h.state())["preferences"]["save_to_project_minutes"], 1);
    run_at(&mut h, 1001.0);
    run_at(&mut h, 1050.0);
    assert!(!h.state().files.busy(), "not yet a minute since saving was turned on");
    h.state_mut().native_project_tick(&ctx, std::time::Duration::from_secs(1062));
    assert!(h.state().files.busy(), "timed Save starts without a paint pass");
    finish_file_job(&mut h);
    assert_eq!(h.state().message, None);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(pcbcraft_storage::load(&root).unwrap(), edited);

    // Saved and unchanged: nothing more is written.
    let resaved = tree(&root);
    run_at(&mut h, 2000.0);
    assert!(!h.state().files.busy());
    assert_eq!(tree(&root), resaved);

    // Another program changed the folder: the timed save is refused and nothing is lost.
    let document = resaved.keys().find(|path| path.extension().is_some_and(|ext| ext == "json") && !path.ends_with("project.json")).unwrap();
    std::fs::write(document, b"external").unwrap();
    place(&mut h, 40);
    run_at(&mut h, 2001.0);
    run_at(&mut h, 2062.0);
    finish_file_job(&mut h);
    let message = h.state().message.clone().unwrap_or_default();
    assert!(message.contains("Project folder changed on disk"), "{message}");
    assert!(h.state().files.dirty(&h.state().session));
    assert_eq!(std::fs::read(document).unwrap(), b"external");

    // Don't Save approval removes this run's entry, even though the project is dirty.
    h.state_mut().waiting = Some(crate::unsaved::Waiting::Discarding(crate::unsaved::Then::Close));
    h.state_mut().finish_waiting(&ctx);
    assert!(h.state().close_allowed);
    eframe::App::on_exit(h.state_mut());
    let left: Vec<_> = std::fs::read_dir(&recovery).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from(".entries.lock")]);
}

/// Enabling timed saves on an Untitled project never starts a file job or opens a picker.
#[test]
fn an_untitled_project_waits_for_an_explicit_first_save() {
    let mut h = harness();
    h.state_mut().start_new_project();
    h.state_mut().preferences.save_to_project_minutes = 1;
    place(&mut h, 0);
    run_at(&mut h, 1.0);
    run_at(&mut h, 3600.0);
    assert!(h.state().files.dirty(&h.state().session));
    assert!(!h.state().files.has_location(&h.state().session));
    assert!(!h.state().files.busy());
    assert!(matches!(h.state().dialog, Dialog::None));
}

/// A painted native callback pair handles Ctrl/Cmd+O before a timed Save due on the same pass.
#[test]
fn visible_open_takes_precedence_over_due_timed_save() {
    use std::time::Duration;
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Open precedence"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
    let mut h = harness();
    let rev = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), rev);
    run_at(&mut h, 1.0);
    let saved = tree(&root);
    let mut app = std::mem::take(h.state_mut());
    let mut frame = eframe::Frame::_new_kittest();
    let ctx = egui::Context::default();
    app.preferences.save_to_project_minutes = 1;
    let revision = app.session.revision();
    app.run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    app.autosave_tick(&ctx, Duration::ZERO);
    app.autosave.advance_native_clock(Duration::from_secs(61));
    let modifiers = egui::Modifiers::COMMAND;
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..egui::RawInput::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        eframe::App::logic(&mut app, ui.ctx(), &mut frame);
        eframe::App::ui(&mut app, ui, &mut frame);
    });
    // There is no renderer in this test; explicitly discard the generated textures.
    output.textures_delta.clear();
    assert!(matches!(app.dialog, Dialog::SaveChanges(crate::unsaved::Then::Open)));
    assert!(!app.files.busy(), "explicit Open must hold off the due Save");
    assert_eq!(tree(&root), saved);
}

/// A due timed save waits for a dialog and never starts another job behind an explicit Save.
#[test]
fn a_dialog_and_a_running_save_hold_back_the_timed_save() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Held save"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(session.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
    let mut h = harness();
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), revision);
    run_at(&mut h, 1.0);
    let saved = tree(&root);
    h.state_mut().preferences.save_to_project_minutes = 1;
    place(&mut h, 0);
    run_at(&mut h, 2.0);
    h.state_mut().dialog = Dialog::OpenLocation;
    run_at(&mut h, 64.0);
    assert_eq!(tree(&root), saved);
    assert!(!h.state().files.busy());
    assert!(h.state().files.dirty(&h.state().session));
    h.state_mut().dialog = Dialog::None;
    let ctx = h.ctx.clone();
    h.state_mut().save_project_file(None, &ctx);
    assert!(h.state().files.busy());
    h.state_mut().autosave_tick(&ctx, std::time::Duration::from_secs(64));
    assert_eq!(h.state().message, None, "no second file request was refused as already running");
    finish_file_job(&mut h);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(pcbcraft_storage::load(&root).unwrap(), encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// An edit that runs for a long time is followed by a snapshot at once, without waiting for
/// the interval; an ordinary edit waits for it.
#[test]
fn a_long_operation_takes_a_snapshot_before_the_interval() {
    let work = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(work.path().to_path_buf());
    h.state_mut().preferences.autosave_minutes = 10;
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    let created = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot_documents(&h), created);
    place(&mut h, 0);
    run_at(&mut h, 2.0);
    run_at(&mut h, 60.0);
    assert_eq!(snapshot_documents(&h), created, "an ordinary edit waits for the interval");
    place(&mut h, 20);
    h.state_mut().autosave.command_took(pcbcraft_storage::recovery::LONG_OPERATION);
    run_at(&mut h, 61.0);
    assert_eq!(snapshot_documents(&h), encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    eframe::App::on_exit(h.state_mut());
}

/// Time spent in a frame with no change does not make the next edit count as a long operation.
#[test]
fn a_long_operation_without_a_change_does_not_make_a_later_edit_urgent() {
    let work = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(work.path().to_path_buf());
    h.state_mut().preferences.autosave_minutes = 10;
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    let created = snapshot_documents(&h);
    h.state_mut().autosave.command_took(pcbcraft_storage::recovery::LONG_OPERATION);
    run_at(&mut h, 2.0);
    place(&mut h, 0);
    run_at(&mut h, 3.0);
    assert_eq!(snapshot_documents(&h), created);
    eframe::App::on_exit(h.state_mut());
}

/// The names of the files in `dir`.
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Off attaches no recorder and writes nothing, even after a long operation; turning snapshots
/// Off removes the running project's entry.
#[test]
fn snapshots_off_record_nothing_and_remove_the_entry() {
    let work = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(work.path().to_path_buf());
    assert_eq!(h.state().preferences.autosave_minutes, 0, "Off by default");
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    place(&mut h, 0);
    h.state_mut().autosave.command_took(pcbcraft_storage::recovery::LONG_OPERATION);
    run_at(&mut h, 2.0);
    run_at(&mut h, 10_000.0);
    assert!(h.state().recovery_entry().is_none());
    assert!(file_names(work.path()).is_empty(), "nothing is written with snapshots Off");

    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"autosave_minutes": 5})).unwrap();
    run_at(&mut h, 10_001.0);
    let (_, id) = h.state().recovery_entry().unwrap();
    h.state().sync_recovery().unwrap();
    assert!(work.path().join(format!("{id}.pc")).exists());

    h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"autosave_minutes": 0})).unwrap();
    run_at(&mut h, 10_002.0);
    assert!(h.state().recovery_entry().is_none());
    if let Some(finished) = h.state_mut().wait_for_recovery_retirement() {
        finished.unwrap();
    }
    assert_eq!(file_names(work.path()), [".entries.lock"], "turning Off removes the entry");
    assert_eq!(h.state().autosave.notice(), None);
    eframe::App::on_exit(h.state_mut());
}

/// Replacing the open project starts a new entry and removes the old one off the frame.
#[test]
fn replacing_the_project_removes_its_entry_in_the_background() {
    let work = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().preferences.autosave_minutes = 5;
    h.state_mut().autosave.dir = Some(work.path().to_path_buf());
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    let (_, old) = h.state().recovery_entry().unwrap();
    let original = h.state().session.snapshot().unwrap();
    // Opening a same-UUID copy is a new history root. It and its edits must not reach the
    // outgoing recorder, even before the next logic pass fences off the running worker.
    let mut copy = pcbcraft_engine::Project::clone(&original);
    copy.name = "Same UUID copy".into();
    let copy = std::sync::Arc::new(copy);
    h.state_mut().session.open_snapshot(copy);
    place(&mut h, 30);
    h.state().sync_recovery().unwrap();
    let old_entry = pcbcraft_storage::recovery::recover(work.path(), &old).unwrap();
    assert_eq!(old_entry.documents, encode_documents(original).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().autosave_tick(&ctx, std::time::Duration::from_secs(2));
    if let Some(finished) = h.state_mut().wait_for_recovery_retirement() {
        finished.unwrap();
    }
    h.state_mut().autosave_tick(&ctx, std::time::Duration::from_secs(3));
    let (_, new) = h.state().recovery_entry().unwrap();
    assert_ne!(old, new);
    h.state().sync_recovery().unwrap();
    assert_eq!(pcbcraft_storage::recovery::recover(work.path(), &new).unwrap().project.name, "Same UUID copy");
    let left: Vec<_> = std::fs::read_dir(work.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(!left.iter().any(|name| name.starts_with(&old)), "{left:?}");
    assert!(work.path().join(format!("{new}.lock")).exists());
    eframe::App::on_exit(h.state_mut());
}

/// An unapproved framework exit preserves dirty recovery; closing after an ordinary
/// successful Save removes the owned entry through the approved close continuation.
#[test]
fn only_an_approved_exit_discards_recovery() {
    use crate::unsaved::{Then, Waiting};
    for approved in [false, true] {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("project");
        let dir = work.path().join("recovery");
        let mut source = Session::default();
        source.apply("project.new", &json!({"name":"Close"})).unwrap();
        pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
        let mut h = harness();
        h.state_mut().autosave.dir = Some(dir.clone());
        h.state_mut().preferences.autosave_minutes = 5;
        let rev = h.state().session.revision();
        h.state_mut().files.open_path_for_test(&root.join("project.json"), rev);
        run_at(&mut h, 1.0);
        place(&mut h, 0);
        let expected = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let (_, entry) = h.state().recovery_entry().unwrap();
        if approved {
            let ctx = h.ctx.clone();
            h.state_mut().save_project_file(None, &ctx);
            h.state_mut().waiting = Some(Waiting::ForSave(Then::Close));
            finish_file_job(&mut h);
        }
        assert_eq!(h.state().close_allowed, approved);
        eframe::App::on_exit(h.state_mut());
        if approved {
            assert!(!dir.join(format!("{entry}.pc")).exists());
        } else {
            assert_eq!(pcbcraft_storage::recovery::recover(&dir, &entry).unwrap().documents, expected);
        }
    }
}

/// Close without waiting ends the process with the save still running, so it keeps the
/// recovery entry with the edits that save might not have written.
#[test]
fn closing_without_waiting_for_a_save_keeps_recovery() {
    use crate::unsaved::{Then, Waiting};
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let dir = work.path().join("recovery");
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Close now"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(dir.clone());
    h.state_mut().preferences.autosave_minutes = 5;
    let rev = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), rev);
    run_at(&mut h, 1.0);
    place(&mut h, 0);
    let expected = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let (_, entry) = h.state().recovery_entry().unwrap();
    h.state_mut().files.use_stand_in();
    let ctx = h.ctx.clone();
    h.state_mut().save_project_file(None, &ctx);
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().waiting = Some(Waiting::ForSave(Then::Close));
    h.state_mut().close_without_waiting(&ctx);
    assert!(h.state().close_allowed);
    assert!(h.state().files.busy(), "the save is still running at exit");
    eframe::App::on_exit(h.state_mut());
    assert_eq!(pcbcraft_storage::recovery::recover(&dir, &entry).unwrap().documents, expected);
    h.state_mut().files.end_stand_in(Some("ended with the process"));
}

/// Closing without waiting must still keep recovery if the pending save fails and is polled on
/// the closing frame before `on_exit` runs, when `files.busy()` has already turned false.
#[test]
fn close_without_waiting_keeps_recovery_when_pending_save_fails_before_exit() {
    use crate::unsaved::{Then, Waiting};
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let dir = work.path().join("recovery");
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Close fail"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(dir.clone());
    h.state_mut().preferences.autosave_minutes = 5;
    let rev = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), rev);
    run_at(&mut h, 1.0);
    place(&mut h, 0);
    let expected = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let (_, entry) = h.state().recovery_entry().unwrap();
    h.state_mut().files.use_stand_in();
    let ctx = h.ctx.clone();
    h.state_mut().save_project_file(None, &ctx);
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().waiting = Some(Waiting::ForSave(Then::Close));
    h.state_mut().close_without_waiting(&ctx);
    h.state_mut().files.end_stand_in(Some("Disk full"));
    run_at(&mut h, 2.0);
    assert!(h.state().close_allowed);
    assert!(!h.state().files.busy(), "the closing frame already polled the failed save");
    eframe::App::on_exit(h.state_mut());
    assert_eq!(pcbcraft_storage::recovery::recover(&dir, &entry).unwrap().documents, expected);
}

/// Interrupting Preferences with a dirty-project close cannot activate a staged timed Save.
#[test]
fn interrupted_preferences_do_not_enable_timed_save() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Preferences"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();
    let mut h = harness();
    let rev = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), rev);
    run_at(&mut h, 1.0);
    place(&mut h, 0);
    let mut preferences = crate::preferences::PreferencesWindow::new(h.state().preferences);
    preferences.saving = Some((5, 1));
    h.state_mut().dialog = Dialog::Preferences(preferences);
    let ctx = h.ctx.clone();
    h.state_mut().begin_project_close(&ctx);
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(crate::unsaved::Then::Close)));
    h.state_mut().dialog = Dialog::None; // Cancel the close and continue editing.
    run_at(&mut h, 2.0);
    run_at(&mut h, 1000.0);
    assert_eq!(h.state().preferences.save_to_project_minutes, 0);
    assert!(h.state().files.dirty(&h.state().session));
    assert!(!h.state().files.busy());
}

/// A transient recorder fault is visible and repaired by a logic-only retry without more edits.
#[test]
fn a_logic_only_pass_retries_recovery_without_another_edit() {
    use std::time::Duration;
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("blocked");
    std::fs::write(&dir, b"not a directory").unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(dir.clone());
    h.state_mut().preferences.autosave_minutes = 10;
    h.state_mut().start_new_project();
    let ctx = h.ctx.clone();
    h.state_mut().native_project_tick(&ctx, Duration::ZERO);
    assert!(h.state().sync_recovery().unwrap_err().contains("Could not create the recovery folder"));
    h.state_mut().native_project_tick(&ctx, Duration::from_secs(1));
    assert!(h.state().autosave.notice().is_some());
    std::fs::remove_file(&dir).unwrap();
    h.state_mut().native_project_tick(&ctx, Duration::from_secs(7));
    h.state().sync_recovery().unwrap();
    h.state_mut().native_project_tick(&ctx, Duration::from_secs(8));
    assert!(h.state().autosave.notice().is_none());
    let (_, id) = h.state().recovery_entry().unwrap();
    assert!(pcbcraft_storage::recovery::recover(&dir, &id).is_ok());
    eframe::App::on_exit(h.state_mut());
}

/// `app.set_preferences` takes exactly the intervals Preferences offers, publishes the same
/// list in its schema, and an invalid interval changes no setting.
#[test]
fn interval_settings_take_only_the_offered_minutes() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let catalog = crate::window_commands::window_catalog();
    let command = catalog.iter().find(|command| command["id"] == "app.set_preferences").unwrap();
    for name in ["autosave_minutes", "save_to_project_minutes"] {
        let schema = &command["params"]["properties"][name];
        assert_eq!((&schema["type"], &schema["enum"]), (&json!("integer"), &json!([0, 1, 5, 10])), "{name}");
        for minutes in [0, 1, 5, 10] {
            h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({ name: minutes })).unwrap();
            assert_eq!(crate::window_commands::inspect_editor(h.state())["preferences"][name], minutes, "{name}");
        }
        for (spelling, expected) in [("1.0", 1), ("1e0", 1), ("-0", 0), ("5.0", 5), ("1e1", 10)] {
            let number: Value = serde_json::from_str(spelling).unwrap();
            h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({ name: number })).unwrap();
            assert_eq!(crate::window_commands::inspect_editor(h.state())["preferences"][name], expected, "{name} {spelling}");
        }
        let before = h.state().preferences;
        for bad in [
            json!(-1),
            json!(2),
            json!(9),
            json!(11),
            json!(256),
            json!(1.5),
            json!("5"),
            json!(null),
            json!(true),
            json!([]),
            json!({}),
            json!(i64::MAX - 1),
            json!(i64::MAX),
            json!(u64::MAX),
            json!(9_007_199_254_740_993_u64),
        ] {
            let error = h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({ name: bad, "theme": "light" })).unwrap_err();
            assert!(error.contains("must be 0 (Off), 1, 5 or 10 minutes"), "{name} {bad}: {error}");
            assert_eq!(h.state().preferences, before, "{name} {bad}");
        }
    }
}

/// Open Preferences on the Saving page through the app bar and the page list.
fn open_saving_page(h: &mut Harness<'_, PcbApp>) {
    click(h, "Preferences");
    // The page list is left of the page heading of the same name.
    h.get_all_by_label("Saving").min_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
    h.run();
    assert!(h.query_by_label("Recovery snapshots").is_some());
}

/// The segment `label` in the Saving page's `row`: 0 for Recovery snapshots, 1 for Save to project.
fn interval_segment<'a>(h: &'a Harness<'_, PcbApp>, row: usize, label: &'a str) -> egui_kittest::Node<'a> {
    let mut segments: Vec<_> = h.get_all_by_label(label).collect();
    assert_eq!(segments.len(), 2, "{label} appears once in each row");
    segments.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    segments.swap_remove(row)
}

fn intervals(h: &Harness<'_, PcbApp>) -> (u8, u8) {
    (h.state().preferences.autosave_minutes, h.state().preferences.save_to_project_minutes)
}

fn apply_enabled(h: &Harness<'_, PcbApp>) -> bool {
    !h.get_by_label("Apply").accesskit_node().is_disabled()
}

/// Preferences → Saving changes nothing until Apply or OK; Cancel drops an unapplied choice and
/// Restore page defaults turns both intervals Off again.
#[test]
fn the_saving_page_takes_effect_only_on_apply() {
    let mut h = harness();
    open_saving_page(&mut h);
    assert!(!apply_enabled(&h));
    interval_segment(&h, 0, "5 min").click();
    h.run();
    interval_segment(&h, 1, "10 min").click();
    h.run();
    assert!(apply_enabled(&h), "a changed interval enables Apply");
    assert_eq!(intervals(&h), (0, 0), "nothing changes before Apply");
    click(&mut h, "Apply");
    assert_eq!(intervals(&h), (5, 10));
    assert!(!apply_enabled(&h));

    interval_segment(&h, 0, "1 min").click();
    h.run();
    assert!(apply_enabled(&h));
    click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(intervals(&h), (5, 10), "Cancel drops the unapplied choice");

    open_saving_page(&mut h);
    click(&mut h, "Restore page defaults");
    assert!(apply_enabled(&h));
    assert_eq!(intervals(&h), (5, 10), "defaults are a draft until applied");
    click(&mut h, "OK");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(intervals(&h), (0, 0), "both are Off by default");
}

/// Each interval segment takes keyboard focus and is chosen with Space.
#[test]
fn the_saving_intervals_work_from_the_keyboard() {
    let mut h = harness();
    open_saving_page(&mut h);
    interval_segment(&h, 0, "10 min").focus();
    h.run();
    assert!(interval_segment(&h, 0, "10 min").is_focused());
    h.key_press(Key::Space);
    h.run();
    assert!(apply_enabled(&h), "Space chose the focused interval");
    click(&mut h, "OK");
    assert_eq!(intervals(&h), (10, 0));
}

/// When recovery cannot be recorded the status bar says so in words, and the notice goes once a
/// retry succeeds.
#[test]
fn the_status_bar_says_when_unsaved_work_is_not_kept() {
    const SHOWN: &str = "Unsaved work isn’t being kept";
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("blocked");
    std::fs::write(&dir, b"not a directory").unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(dir.clone());
    h.state_mut().preferences.autosave_minutes = 10;
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    assert!(h.state().sync_recovery().unwrap_err().contains("Could not create the recovery folder"));
    run_at(&mut h, 2.0);
    run_at(&mut h, 2.5);
    let notice = h.state().autosave.notice().unwrap().to_owned();
    assert!(notice.starts_with("Unsaved work isn’t being kept for recovery: Could not create the recovery folder"), "{notice}");
    assert!(h.query_by_label_contains(SHOWN).is_some(), "the reason is shown in the status bar, not only on hover");

    std::fs::remove_file(&dir).unwrap();
    run_at(&mut h, 7.5);
    h.state().sync_recovery().unwrap();
    run_at(&mut h, 8.0);
    run_at(&mut h, 8.5);
    assert_eq!(h.state().autosave.notice(), None);
    assert!(h.query_by_label_contains(SHOWN).is_none());
    eframe::App::on_exit(h.state_mut());
}

/// Screenshot state: Preferences open on the Saving page with both intervals at their Off default.
#[test]
fn saving_page_open() {
    let mut h = harness();
    open_saving_page(&mut h);
    assert!(h.query_by_label("Save to project").is_some());
    assert!(!apply_enabled(&h));
    assert_eq!(intervals(&h), (0, 0));
}

/// Screenshot state: unapplied choices on the Saving page, with Apply enabled.
#[test]
fn saving_page_changed() {
    let mut h = harness();
    open_saving_page(&mut h);
    interval_segment(&h, 0, "5 min").click();
    h.run();
    interval_segment(&h, 1, "1 min").click();
    h.run();
    assert!(apply_enabled(&h));
    assert_eq!(intervals(&h), (0, 0));
}

/// Screenshot state: keyboard focus on a Recovery snapshots interval.
#[test]
fn saving_page_keyboard_focus() {
    let mut h = harness();
    open_saving_page(&mut h);
    interval_segment(&h, 0, "1 min").focus();
    h.run();
    assert!(interval_segment(&h, 0, "1 min").is_focused());
    assert!(!interval_segment(&h, 1, "1 min").is_focused());
}

/// Screenshot state: the status bar saying why unsaved work isn't being kept.
#[test]
fn status_bar_recovery_notice() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("blocked");
    std::fs::write(&dir, b"not a directory").unwrap();
    let mut h = harness();
    h.state_mut().autosave.dir = Some(dir);
    h.state_mut().preferences.autosave_minutes = 5;
    h.state_mut().start_new_project();
    run_at(&mut h, 1.0);
    assert!(h.state().sync_recovery().is_err());
    run_at(&mut h, 2.0);
    run_at(&mut h, 2.5);
    assert!(h.query_by_label_contains("Unsaved work isn’t being kept for recovery").is_some());
}

/// Turning off Recovery snapshots retires the recorder without resetting an already-pending timed
/// Save schedule on the same project.
#[test]
fn turning_off_recovery_keeps_the_timed_save_deadline() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let recovery = work.path().join("recovery");
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Keep save deadline"})).unwrap();
    pcbcraft_storage::save(&root, None, &encode_documents(source.snapshot().unwrap()).unwrap(), &mut |_| Ok(())).unwrap();

    let mut h = harness();
    h.state_mut().autosave.dir = Some(recovery);
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&root.join("project.json"), revision);
    run_at(&mut h, 1.0);
    let saved = tree(&root);

    h.state_mut().preferences.autosave_minutes = 1;
    h.state_mut().preferences.save_to_project_minutes = 1;
    place(&mut h, 0);
    run_at(&mut h, 2.0);
    assert!(h.state().recovery_entry().is_some());
    assert!(h.state().files.dirty(&h.state().session));

    // Turn off only Recovery snapshots at t=52; the timed Save must keep its t=62 deadline.
    h.state_mut().preferences.autosave_minutes = 0;
    run_at(&mut h, 52.0);
    assert!(h.state().recovery_entry().is_none());
    if let Some(finished) = h.state_mut().wait_for_recovery_retirement() {
        finished.unwrap();
    }
    assert_eq!(tree(&root), saved, "timed Save must not run early");
    assert!(h.state().files.dirty(&h.state().session));

    // At t=63, the original one-minute Save deadline fires instead of waiting until t=112.
    run_at(&mut h, 63.0);
    finish_file_job(&mut h);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(pcbcraft_storage::load(&root).unwrap(), encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    eframe::App::on_exit(h.state_mut());
}
