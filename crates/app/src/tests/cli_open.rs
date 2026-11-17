//! The desktop command line opens a project at startup through the ordinary native file
//! worker. No picker and no click are involved: the first frames start the job, the ordinary
//! completion path enters the editor on success, and a failed open leaves Home usable.

use super::recent_projects::finish_file;
use super::*;

/// A project path named at startup opens without a native picker, in both the project folder
/// and the `project.json` entry point spellings. The completed open enters the editor, leaves
/// the project clean, and is remembered in desktop history like a recent-project reopen.
#[test]
fn a_startup_path_opens_the_project_without_a_picker() {
    for entry_point in [false, true] {
        let (_copy, project_json) = dual_led_copy();
        let path = if entry_point { project_json } else { project_json.parent().unwrap().to_owned() };
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc).with_startup_project(path.clone()));
        h.run();
        finish_file(&mut h);
        assert_eq!(h.state().session.project().unwrap().name, "Dual LED · transistor flasher");
        assert!(!h.state().home, "the opened project replaces Home");
        assert!(h.state().message.is_none());
        assert!(!h.state().files.dirty(&h.state().session));
        assert_eq!(h.state().files.recent.entries().len(), 1, "the startup open is remembered like a recents open");
    }
}

/// A startup open of a path the loader refuses reports the ordinary open error in the status
/// bar, keeps the app on Home with no project loaded, and leaves Home's actions working.
#[test]
fn a_failed_startup_open_keeps_home_usable() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone");
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc).with_startup_project(missing.clone()));
    h.run();
    finish_file(&mut h);
    assert!(h.state().home, "a failed startup open keeps Home");
    assert!(h.state().session.project().is_none());
    let message = h.state().message.clone().unwrap();
    assert!(message.contains("Could not open") && message.contains(&missing.display().to_string()), "status error: {message}");
    // Home remains usable after the failure: its start card creates a project as ordinary.
    click(&mut h, "New Schematic");
    assert!(h.state().session.project().is_some());
    assert!(!h.state().home);
}
