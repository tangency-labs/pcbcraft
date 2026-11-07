//! Home history exercises real native file workers, the deferred unsaved decision and app storage.

use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

use super::*;

/// The file worker keeps requesting repaints, so step until its result is adopted rather than
/// asking the harness to wait for idle while I/O may still be running.
fn finish_file(h: &mut Harness<'_, PcbApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().files.busy() && Instant::now() < deadline {
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!h.state().files.busy(), "file worker did not complete");
    h.run();
}

/// Let bounded background previews finish without requiring every worker to fit in one idle run.
fn finish_previews(h: &mut Harness<'_, PcbApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        h.run_steps(1);
        if h.state().files.recent.entries().iter().all(|entry| h.state().files.recent.preview(&entry.path).is_some()) {
            break;
        }
        assert!(Instant::now() < deadline, "preview worker did not complete");
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

/// Click a path-qualified row through accessibility, then let the normal worker finish.
fn open_recent(h: &mut Harness<'_, PcbApp>, name: &str, path: &Path) {
    h.get_by_label(&format!("Open {name} ({})", path.display())).click();
    h.run_steps(2);
}

/// Save through the test picker stand-in, retaining production publication and polling.
fn saved_project(h: &mut Harness<'_, PcbApp>, path: &Path) {
    new_project(h);
    h.state_mut().files.use_stand_in();
    let ctx = h.ctx.clone();
    h.state_mut().save_project_file(None, &ctx);
    h.state_mut().files.finish_stand_in(path.to_owned());
    finish_file(h);
}

#[derive(Default)]
struct Storage(HashMap<String, String>);
impl eframe::Storage for Storage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

/// Restore through the actual launcher constructor with saved application settings.
fn restore(storage: &dyn eframe::Storage) -> PcbApp {
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(storage);
    PcbApp::new(&cc)
}

/// Successful save and reopen are remembered across app launches, and folder/project.pc opens
/// converge on one row. Removing history leaves both the open session and disk data intact.
#[test]
fn saved_projects_reopen_from_home_and_history_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Power controller");
    let mut h = harness();
    saved_project(&mut h, &path);
    assert_eq!(h.state().files.recent.entries()[0].name, "Power controller");
    open_home(&mut h);
    h.get_by_label("Recent projects");
    let mut storage = Storage::default();
    eframe::App::save(h.state_mut(), &mut storage);
    let app = restore(&storage);
    assert!(app.home);
    assert!(app.session.project().is_none());
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| {
        cc.storage = Some(&storage);
        PcbApp::new(cc)
    });
    h.run();
    open_recent(&mut h, "Power controller", &path);
    finish_file(&mut h);
    assert!(!h.state().home);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(h.state().session.project().unwrap().name, "Power controller");
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    h.state_mut().files.open_path(path.join("project.pc"), revision, &ctx).unwrap();
    finish_file(&mut h);
    assert_eq!(h.state().files.recent.entries().len(), 1);
    open_home(&mut h);
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, &format!("Remove Power controller ({}) from recent projects", path.display()));
    h.get_by_label("No recent projects yet");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert!(path.join("project.pc").is_file());
    let mut cleared_storage = Storage::default();
    eframe::App::save(h.state_mut(), &mut cleared_storage);
    assert!(restore(&cleared_storage).files.recent.entries().is_empty());
}

/// Archive and legacy paths use the same loader. A missing recent stays removable, reports its
/// error, and leaves the existing project and Home intact instead of replacing either.
#[test]
fn archives_and_legacy_files_reopen_and_missing_files_preserve_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("original.pc");
    std::fs::write(&legacy, include_bytes!("../../tests/fixtures/two-resistors.pc")).unwrap();
    let archive = dir.path().join("archive.pc.zip");
    std::fs::copy("tests/fixtures/split/two-resistors.pc.zip", &archive).unwrap();
    let mut h = harness();
    h.state_mut().show_home();
    h.state_mut().files.recent.remember("Legacy", legacy.clone());
    h.state_mut().files.recent.remember("Archive", archive.clone());
    h.run();
    open_recent(&mut h, "Legacy", &legacy);
    finish_file(&mut h);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    open_home(&mut h);
    open_recent(&mut h, "Archive", &archive);
    finish_file(&mut h);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    let name = h.state().files.recent.entries()[0].name.clone();
    std::fs::remove_file(&archive).unwrap();
    open_home(&mut h);
    let before = h.state().session.snapshot().unwrap();
    open_recent(&mut h, &name, &archive);
    finish_file(&mut h);
    assert!(h.state().home);
    assert!(h.state().message.as_ref().unwrap().contains("Could not open"));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().files.recent.entries()[0].path, archive);
    click(&mut h, "Clear recent projects");
    h.get_by_label("No recent projects yet");
    assert!(legacy.is_file());
}

/// The exact chosen path survives the unsaved prompt and its save job, even when saving promotes
/// another recent entry. Cancel keeps the draft; Save opens the originally chosen project.
#[test]
fn recent_open_asks_before_replacing_unsaved_work_and_keeps_its_target_through_save() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Target");
    let draft = dir.path().join("Draft");
    let mut h = harness();
    saved_project(&mut h, &target);
    new_project(&mut h);
    open_home(&mut h);
    let before = h.state().session.snapshot().unwrap();
    open_recent(&mut h, "Target", &target);
    h.get_by_label("Save changes to Untitled?");
    click(&mut h, "Cancel");
    assert!(h.state().home);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    open_recent(&mut h, "Target", &target);
    h.query_all_by_label("Save").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
    h.run_steps(2);
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().files.finish_stand_in(draft.clone());
    finish_file(&mut h);
    assert_eq!(h.state().session.project().unwrap().name, "Target");
    assert_eq!(h.state().files.recent.entries()[0].path, target);
    assert_eq!(h.state().files.recent.entries()[1].path, draft);
    assert!(draft.join("project.pc").is_file());
}

/// Long, equal names remain distinguishable by full paths. The narrow layout can scroll to the
/// last row and remove it without opening it or affecting its equally named neighbor.
#[test]
fn narrow_history_keeps_long_paths_reachable_and_duplicate_names_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.set_size(egui::vec2(420.0, 700.0));
    let name = "A very long Unicode project name — Power controller";
    for index in 0..12 {
        h.state_mut().files.recent.remember(name, dir.path().join(format!("a-long-parent-directory/project-{index}.pc.zip")));
    }
    h.state_mut().show_home();
    finish_previews(&mut h);
    let path = dir.path().join("a-long-parent-directory/project-0.pc.zip");
    let label = format!("Remove {name} ({}) from recent projects", path.display());
    h.get_by_label(&label).scroll_to_me();
    h.run();
    let rect = h.get_by_label(&label).rect();
    assert!(rect.left() >= 0.0 && rect.right() <= 420.0, "{rect:?}");
    click(&mut h, &label);
    assert!(h.state().home);
    assert!(!h.state().files.busy());
    assert_eq!(h.state().files.recent.entries().len(), 11);
}

/// Cancelling or failing the prompted save keeps the draft; explicitly discarding it then opens
/// the recent target, without adding a history entry for the unsuccessful save.
#[test]
fn cancelled_and_failed_saves_do_not_open_the_recent_target() {
    for error in [None, Some("Destination is read-only")] {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Target");
        let mut h = harness();
        saved_project(&mut h, &target);
        new_project(&mut h);
        open_home(&mut h);
        let before = h.state().session.snapshot().unwrap();
        open_recent(&mut h, "Target", &target);
        h.query_all_by_label("Save").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
        h.run_steps(2);
        h.state_mut().files.end_stand_in(error);
        finish_file(&mut h);
        assert!(h.state().home);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().files.recent.entries().len(), 1);
        open_recent(&mut h, "Target", &target);
        h.get_by_label("Don't Save").click();
        h.run_steps(2);
        finish_file(&mut h);
        assert!(!h.state().home);
        assert_eq!(h.state().session.project().unwrap().name, "Target");
    }
}

/// Restored history renders real saved geometry without opening the document or changing the
/// current session, dirty state or file bytes. Empty and unavailable previews remain actionable.
#[test]
fn restored_home_cards_preview_saved_schematics_without_opening_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-resistors.pc.zip");
    std::fs::copy("tests/fixtures/split/two-resistors.pc.zip", &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut h = harness();
    let empty = dir.path().join("empty");
    saved_project(&mut h, &empty);
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let missing = dir.path().join("missing.pc");
    h.state_mut().files.recent.remember("Two resistors", path.clone());
    h.state_mut().files.recent.remember("Missing", missing.clone());
    open_home(&mut h);
    finish_previews(&mut h);
    h.get_by_label(&format!("Schematic preview: Two resistors ({})", path.display()));
    assert!(!h.state().files.recent.preview(&path).unwrap().unwrap().is_empty());
    assert!(h.state().files.recent.preview(&empty).unwrap().unwrap().is_empty());
    assert!(matches!(h.state().files.recent.preview(&missing), Some(None)));
    assert!(h.state().home);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.revision(), revision);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
