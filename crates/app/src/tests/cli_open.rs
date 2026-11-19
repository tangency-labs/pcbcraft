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
        assert!(!h.state().startup_open_pending, "the completed startup open consumed its terminal-report flag");
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
    assert!(!h.state().startup_open_pending, "the failed startup open consumed its terminal-report flag once");
    let message = h.state().message.clone().unwrap();
    assert!(message.contains("Could not open") && message.contains(&missing.display().to_string()), "status error: {message}");
    // Home remains usable after the failure: its start card creates a project as ordinary.
    click(&mut h, "New Schematic");
    assert!(h.state().session.project().is_some());
    assert!(!h.state().home);
}

/// A startup open whose worker cannot start has no later poll result to report: the setup
/// failure prints the same line once to the launching terminal, reports in the status bar,
/// and must not arm a report the poll can never deliver.
#[test]
fn a_startup_setup_failure_prints_once_to_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let fault = "Could not start opening the project: injected";
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| {
        let mut app = PcbApp::new(cc).with_startup_project(dir.path().join("gone"));
        app.files.open_setup_fault = Some(fault.to_owned());
        app
    });
    h.run();
    let state = h.state();
    assert!(state.home, "a setup failure keeps Home");
    assert_eq!(state.message.as_deref(), Some(fault), "the setup failure reports in the status bar");
    assert!(!state.startup_open_pending, "a setup failure must not wait for a poll result");
    assert_eq!(state.startup_reports.len(), 1, "the setup failure prints once to the terminal");
    assert_eq!(state.startup_reports[0], fault, "the printed line matches the status error");
}

/// The command line names paths relative to the launching directory: `.`, a bare `project.json`
/// and a bare `.pc` name must resolve to an absolute target with a parent and a name before the
/// ordinary open runs. An absolute path keeps its spelling.
#[test]
fn a_startup_path_resolves_before_the_open() {
    for spelling in [".", "./", "project.json", "board.pc"] {
        let resolved = PcbApp::resolve_startup_path(spelling.into());
        assert!(resolved.is_absolute(), "{spelling} resolves to an absolute path");
        assert!(resolved.file_name().is_some(), "{spelling} keeps a file name");
        assert!(resolved.parent().is_some_and(|parent| parent.exists()), "{spelling} keeps an existing parent");
    }
    let dir = tempfile::tempdir().unwrap();
    let absolute = dir.path().join("project.json");
    assert_eq!(PcbApp::resolve_startup_path(absolute.clone()), absolute, "an absolute path keeps its spelling");
}

/// The resolved startup path reaches the ordinary open: `.` no longer fails as a nameless
/// project file. The launching directory holds no project, so the open reports its own error.
#[test]
fn a_relative_startup_path_reaches_the_open_resolved() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc).with_startup_project(".".into()));
    h.run();
    finish_file(&mut h);
    let message = h.state().message.clone().unwrap_or_default();
    assert!(!message.contains("Project file has no name"), "a relative startup path resolves before the open: {message}");
    assert!(!h.state().startup_open_pending);
}

/// Abandoning a stalled startup open clears its terminal-report arming: the abandon returns no
/// poll result, so a later ordinary open failure must not print as a startup failure. A refused
/// abandon leaves the arming alone.
#[test]
fn an_abandoned_startup_open_clears_the_terminal_report() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc));
    let revision = h.state().session.revision();
    let _late = h.state_mut().files.stall("open", Some("/stalled/project"), revision);
    h.state_mut().startup_open_pending = true;
    h.state_mut().stop_waiting_for_file(None).unwrap();
    assert!(!h.state().startup_open_pending, "an abandoned startup open clears the arming");
    assert!(h.state().startup_reports.is_empty(), "abandoning prints nothing to the terminal");
    h.state_mut().report_startup_open(Some(&project_files::Update::Error("Could not open later".to_owned())));
    assert!(h.state().startup_reports.is_empty(), "a later open failure is not reported as a startup failure");
    h.state_mut().startup_open_pending = true;
    assert!(h.state_mut().stop_waiting_for_file(None).unwrap_err().contains("No file operation"));
    assert!(h.state().startup_open_pending, "a refused abandon leaves the arming alone");
}

/// The terminal-report arming must survive every frame whose file poll has no result yet:
/// the worker cannot finish in the frame that starts it, and its result usually lands
/// between frames, so no harness step can catch that window reliably. It is driven at the
/// frame seam instead: result-free frames leave the arming intact, and the first result of
/// any kind consumes it exactly once. The earlier shape consumed the arming on the starting
/// frame itself, so the error never reached the launching terminal; that regression lands
/// here, not on worker timing.
#[test]
fn the_terminal_report_survives_frames_without_a_file_result() {
    let dir = tempfile::tempdir().unwrap();
    let mut h =
        Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc).with_startup_project(dir.path().join("gone")));
    let app = h.state_mut();
    app.startup_open_pending = true;
    for _ in 0..4 {
        app.report_startup_open(None);
        assert!(app.startup_open_pending, "a frame whose poll has no result yet must leave the arming intact");
    }
    app.report_startup_open(Some(&project_files::Update::Message("Saved".to_owned())));
    assert!(!app.startup_open_pending, "a message result consumes the arming");
    app.report_startup_open(Some(&project_files::Update::Error("Could not open".to_owned())));
    assert!(!app.startup_open_pending, "the report happens once; a later error finds the arming already spent");
    assert_eq!(app.startup_reports.len(), 1, "the failed result prints once to the terminal");
    app.startup_open_pending = true;
    app.report_startup_open(Some(&project_files::Update::Opened));
    assert!(!app.startup_open_pending, "an opened result consumes the arming too");
}
