//! Arrangement acceptance through the real UI, file adoption and presentation handlers.
use super::*;
use crate::{editor_view::Workspace, tests::click};
use egui_kittest::{Harness, kittest::Queryable};
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};
use serde_json::json;

/// Open an original project with one editable symbol through the shared commands.
fn harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Arrangement test"}));
    app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(30).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Run an arrangement intent through the deferred path used by the tab strip and future MCP.
fn action(h: &mut Harness<'_, PcbApp>, action: Action) {
    h.state_mut().queue_arrangement(&egui::Context::default(), action);
    h.run();
}

/// Right-click the selected arrangement through its real tab, without changing the active layout.
#[allow(clippy::unwrap_used)]
fn active_tab_menu(h: &mut Harness<'_, PcbApp>) {
    let active = h.state().arrangements.active();
    let name = h.state().arrangements.tabs().into_iter().find(|(id, _)| *id == active).unwrap().1;
    h.get_by_label(&format!("Arrangement {name}")).scroll_to_me();
    h.run();
    h.get_by_label(&format!("Arrangement {name}")).click_secondary();
    h.run();
}

/// Save to a temporary project using the production writer with only the file picker bypassed.
fn save(h: &mut Harness<'_, PcbApp>, path: &std::path::Path, save_as: bool) {
    h.state_mut().files.use_stand_in();
    h.state_mut().save_project_file(save_as.then_some(crate::project_files::SaveFormat::Folder), &egui::Context::default());
    h.state_mut().files.finish_stand_in(path.to_owned());
    h.run();
}

/// Top tabs own arrangements; each activation restores mixed panes with fresh input identities.
#[test]
fn arrangement_tabs_preserve_views_cameras_and_shared_edits() {
    let mut h = harness();
    h.get_by_label("Arrangement Design");
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.state_mut().views.active.camera.center_on(Point { x_nm: 15_000_000, y_nm: 4_000_000 });
    h.run();
    let original = h.state().arrangement_snapshot().unwrap();
    let stale = h.state().views.active.input(&h.state().session).unwrap();
    let revision = h.state().session.revision();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    active_tab_menu(&mut h);
    click(&mut h, "Duplicate…");
    click(&mut h, "Create");
    assert_eq!(h.state().arrangements.tabs().len(), 2);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), original);
    assert!(h.state().views.view(stale.view()).is_some());
    action(&mut h, Action::Rename { id: 2, name: "Routing".into() });
    h.get_by_label("Arrangement Routing");
    h.state_mut().views.active.camera.center_on(Point { x_nm: -30_000_000, y_nm: 0 });
    h.run();
    let routing = h.state().arrangement_snapshot().unwrap();
    click(&mut h, "Arrangement Design");
    assert_eq!(h.state().arrangement_snapshot().unwrap(), original);
    click(&mut h, "Arrangement Routing");
    assert_eq!(h.state().arrangement_snapshot().unwrap(), routing);
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// A real save/open cycle restores the selected arrangement, native host membership and panels.
#[test]
fn saved_project_restores_arrangement_windows_and_panels_without_electrical_changes() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project");
    save(&mut h, &path, false);
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let source = h.state().views.active.id();
    h.state_mut().views.open_window(source).unwrap();
    h.run();
    action(&mut h, Action::Create { name: "Review setup".into(), start: Start::Current });
    let id = h.state().arrangements.active();
    action(&mut h, Action::Rename { id, name: "Review".into() });
    h.state_mut().browser.open = false;
    h.state_mut().views.active.camera.center_on(Point { x_nm: 21_000_000, y_nm: -5_000_000 });
    h.run();
    let snapshot = h.state().arrangement_snapshot().unwrap();
    let storage = h.state().arrangements.storage_text().unwrap();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let mut fresh = PcbApp { arrangements: Arrangements::from_storage(Some(storage)), ..PcbApp::default() };
    fresh.files.open_path_for_test(&path.join("project.pc"), fresh.session.revision());
    let mut reopened =
        Harness::builder().with_size(egui::vec2(1400., 900.)).with_max_steps(30).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), fresh);
    reopened.run();
    assert_eq!(reopened.state().arrangements.tabs().len(), 2);
    assert_eq!(reopened.state().arrangement_snapshot().unwrap(), snapshot);
    assert!(!reopened.state().files.dirty(&reopened.state().session));
    assert_eq!(encode_documents(reopened.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Failed and cancelled saves retain the old association; Save As copies, rather than moves, UI state.
#[test]
fn save_as_and_failed_open_preserve_arrangement_records() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    save(&mut h, &dir.path().join("original"), false);
    let old_key = h.state().arrangements.key.clone();
    let id = h.state().arrangements.active();
    action(&mut h, Action::Rename { id, name: "Review".into() });
    for error in [None, Some("disk unavailable")] {
        h.state_mut().save_project_file(Some(crate::project_files::SaveFormat::Folder), &egui::Context::default());
        h.state_mut().files.end_stand_in(error);
        h.run();
        assert_eq!(h.state().arrangements.key, old_key);
    }
    save(&mut h, &dir.path().join("copy"), true);
    assert_ne!(h.state().arrangements.key, old_key);
    assert_eq!(h.state().arrangements.store.projects.len(), 2);
    let before = h.state().arrangements.current.clone();
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&dir.path().join("missing"), revision);
    h.run();
    assert_eq!(h.state().arrangements.current, before);
}

/// Unsupported records remain byte-equivalent as JSON values while the project still opens.
#[test]
fn invalid_newer_and_duplicate_placements_do_not_replace_saved_state() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    save(&mut h, &dir.path().join("project"), false);
    let key = h.state().arrangements.key.clone().unwrap();
    let original = h.state().arrangements.store.projects[&key].clone();
    for mutation in [0, 1, 2, 3] {
        let mut bad = original.clone();
        match mutation {
            0 => bad["version"] = json!(999),
            1 => bad["arrangements"][0]["snapshot"]["hosts"][0]["focused"] = json!(999),
            2 => bad["arrangements"][0]["snapshot"]["hosts"][0]["parked"] = json!([0]),
            _ => bad["arrangements"][0]["snapshot"]["views"][0]["target"]["Sheet"]["future_target_field"] = json!("preserve"),
        }
        let text = json!({"version":1,"projects":{key.clone():bad.clone()}}).to_string();
        let mut app = PcbApp { arrangements: Arrangements::from_storage(Some(text)), ..PcbApp::default() };
        app.files.open_path_for_test(&dir.path().join("project/project.pc"), 0);
        let mut fresh = Harness::builder().with_size(egui::vec2(1000., 700.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        fresh.run();
        assert!(fresh.state().session.project().is_some());
        assert!(fresh.state().arrangements.protected_record);
        assert_eq!(fresh.state().arrangements.store.projects[&key], bad);
    }
    assert!(Arrangements::from_storage(Some("{bad json".into())).storage_text().is_none());
    assert!(Arrangements::from_storage(Some(json!({"version":999,"projects":{}}).to_string())).storage_text().is_none());
}

/// A missing board is retained as an unavailable view, never substituted with the current board.
#[test]
fn missing_target_is_not_rebound_and_can_be_explicitly_replaced() {
    let mut h = harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let state = h.state().arrangement_snapshot().unwrap();
    let mut value = serde_json::to_value(&state).unwrap();
    value["views"][state.hosts[0].focused]["target"]["Board"]["board"] = json!("00000000-0000-4000-8000-000000000099");
    let state: Snapshot = serde_json::from_value(value).unwrap();
    assert!(h.state_mut().restore_arrangement(&state, &egui::Context::default()));
    h.run();
    assert!(h.state().views.active.unavailable());
    assert!(h.state().views.active.input(&h.state().session).is_none());
    h.get_by_label("This document is unavailable. Choose a workspace to replace this view.");
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    assert!(!h.state().views.active.unavailable());
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
}

/// Browser navigation belongs to the focused pane, even when another pane shows that sheet.
#[test]
fn browser_sheet_reuses_local_tab_without_switching_arrangements_or_other_panes() {
    let mut h = harness();
    let original = h.state().views.active.id();
    let arrangement = h.state().arrangements.active();
    let revision = h.state().session.revision();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let companion = h.state().views.hosts[&h.state().views.main].layout.views()[1];
    let companion_camera = h.state().views.view(companion).unwrap().camera.saved_state();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let board = h.state().views.active.id();
    assert_ne!(board, original);
    assert_ne!(board, companion);
    click(&mut h, "Main");
    assert_eq!(h.state().views.active.id(), original);
    assert_eq!(h.state().arrangements.active(), arrangement);
    let group = h.state().views.hosts[&h.state().views.main].layout.group(original).unwrap().views();
    assert_eq!(group, vec![original, board]);
    assert_eq!(h.state().views.view(companion).unwrap().camera.saved_state(), companion_camera);
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    // A tab click returns to the existing board camera instead of creating another view.
    let tabs: Vec<_> = h.query_all_by_label("Arrangement test › Board").collect();
    tabs[0].click();
    h.run();
    assert_eq!(h.state().views.active.id(), board);
    click(&mut h, "Main");
    assert_eq!(h.state().views.hosts[&h.state().views.main].layout.group(original).unwrap().views(), group);
}

/// Closing and detaching tabs retain their siblings; collapse/restore keeps complete tab groups.
#[test]
fn document_tabs_survive_split_toggle_detach_and_local_close() {
    let mut h = harness();
    let sheet = h.state().views.active.id();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let board = h.state().views.active.id();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open Schematic beside current view");
    let group = h.state().views.hosts[&h.state().views.main].layout.group(board).unwrap().views();
    assert_eq!(group, vec![sheet, board]);
    h.state_mut().split_editors(&egui::Context::default(), false);
    h.run();
    assert_eq!(h.state().views.hosts[&h.state().views.main].layout.views(), group);
    h.state_mut().split_editors(&egui::Context::default(), true);
    h.run();
    assert_eq!(h.state().views.hosts[&h.state().views.main].layout.group(board).unwrap().views(), group);
    h.state_mut().split_editors(&egui::Context::default(), false);
    h.run();
    let detached = h.state_mut().views.detach_view(board).unwrap();
    h.run();
    assert_eq!(h.state().views.hosts[&h.state().views.main].layout.views(), vec![sheet]);
    assert_eq!(h.state().views.hosts[&detached].layout.views(), vec![board]);
    h.state_mut().views.close_view(board);
    h.run();
    assert!(h.state().views.view(board).is_none());
    assert!(h.state().arrangement_snapshot().unwrap().valid());
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Close view Arrangement test › Board");
    assert_eq!(h.state().views.active.id(), sheet);
    assert!(h.state().arrangement_snapshot().unwrap().valid());
}

/// A queued arrangement intent cannot cross a project replacement or an owned modal.
#[test]
fn stale_or_modal_arrangement_intents_do_not_run() {
    let mut h = harness();
    h.state_mut().queue_arrangement(&egui::Context::default(), Action::Create { name: "Review setup".into(), start: Start::Current });
    h.state_mut().run("project.new", &json!({"name":"Replacement"}));
    h.run();
    assert_eq!(h.state().arrangements.tabs().len(), 1);
    h.state_mut().dialog = Dialog::About;
    h.state_mut().queue_arrangement(&egui::Context::default(), Action::Create { name: "Review setup".into(), start: Start::Current });
    h.run();
    assert_eq!(h.state().arrangements.tabs().len(), 1);
}

/// Oversized layouts keep the last valid saved record, report the limit once, and resume
/// checkpointing after the user closes the excess window without losing project edits.
#[test]
fn exceeding_host_limit_preserves_saved_arrangements_and_reports_once() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    save(&mut h, &dir.path().join("project"), false);
    let before = h.state().arrangements.storage_text().unwrap();
    let project = h.state().session.inspect();
    let source = h.state().views.active.id();
    let mut last = None;
    for _ in 0..16 {
        last = h.state_mut().views.open_window(source);
    }
    h.state_mut().checkpoint_arrangements();
    assert!(h.state_mut().arrangements.notice.take().unwrap().contains("limits"));
    assert_eq!(h.state().arrangements.storage_text().unwrap(), before);
    h.state_mut().checkpoint_arrangements();
    assert!(h.state().arrangements.notice.is_none());
    h.state_mut().views.close_host(last.unwrap());
    h.state_mut().checkpoint_arrangements();
    assert!(!h.state().arrangements.snapshot_limited);
    assert_ne!(h.state().arrangements.storage_text().unwrap(), before);
    assert_eq!(h.state().session.inspect(), project);
}

/// Creating and renaming use stable forms, and naming an existing setup preserves all live IDs.
#[test]
fn arrangement_forms_keep_windows_and_views_alive() {
    use egui::{Key, Modifiers};
    let mut h = harness();
    h.state_mut().integrated_titlebar = true;
    h.run();
    let view = h.state().views.active.id();
    let revision = h.state().session.revision();
    let before = h.state().arrangement_snapshot().unwrap();
    click(&mut h, "New arrangement");
    h.get_by_label("Arrangement name");
    h.get_by_label("Cancel").hover();
    h.run_steps(30);
    assert!(matches!(h.state().dialog, Dialog::Arrangement(_)));
    click(&mut h, "Cancel");
    assert_eq!(h.state().arrangements.tabs().len(), 1);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), before);

    click(&mut h, "New arrangement");
    h.get_by_label("Arrangement name").type_text("Routing");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().arrangements.tabs().len(), 2);
    h.get_by_label("Arrangement Routing");
    assert_eq!(h.state().views.active.id(), view);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), before);

    active_tab_menu(&mut h);
    click(&mut h, "Rename…");
    h.get_by_label("Arrangement name").type_text("Review");
    h.run();
    // Enter commits the name; neither a native close nor a new-project shortcut may escape.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Q);
    h.run();
    assert!(!h.state().windows.quit_requested);
    assert!(!h.state().close_allowed);
    h.get_by_label("Rename").click();
    h.step();
    assert!(!h.output().viewport_output.values().any(|output| output.commands.contains(&egui::ViewportCommand::Close)));
    h.run();
    h.get_by_label("Arrangement Review");
    assert_eq!(h.state().views.active.id(), view);
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), before);

    active_tab_menu(&mut h);
    click(&mut h, "Rename…");
    h.get_by_label("Arrangement name").type_text("Uncommitted");
    h.run();
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Arrangement Review");
    assert!(matches!(h.state().dialog, Dialog::None));
}

/// Invalid and duplicate names stay editable; removal needs an explicit confirmation.
#[test]
fn arrangement_form_validation_and_removal_preserve_cancelled_work() {
    use egui::{Key, Modifiers};
    let mut h = harness();
    click(&mut h, "New arrangement");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run();
    click(&mut h, "Create");
    h.get_by_label("Use a name of 1–80 characters on one line.");
    h.get_by_label("Arrangement name").type_text("Design");
    h.run();
    click(&mut h, "Create");
    h.get_by_label("An arrangement already uses that name.");
    click(&mut h, "Cancel");
    assert_eq!(h.state().arrangements.tabs().len(), 1);

    active_tab_menu(&mut h);
    click(&mut h, "Duplicate…");
    click(&mut h, "Create");
    h.get_by_label("Arrangement Design copy");
    let before = h.state().arrangement_snapshot().unwrap();
    active_tab_menu(&mut h);
    click(&mut h, "Delete…");
    click(&mut h, "Cancel");
    assert_eq!(h.state().arrangements.tabs().len(), 2);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), before);
    active_tab_menu(&mut h);
    click(&mut h, "Delete…");
    click(&mut h, "Delete");
    h.get_by_label("Arrangement Design");
    assert_eq!(h.state().arrangements.tabs().len(), 1);
}

/// A deliberately single-view setup can be created without losing the original split layout.
#[test]
fn new_single_view_arrangement_keeps_the_previous_setup() {
    let mut h = harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let before = h.state().arrangement_snapshot().unwrap();
    click(&mut h, "New arrangement");
    click(&mut h, "Single view in one window");
    click(&mut h, "Create");
    assert!(!h.state().views.is_split());
    click(&mut h, "Arrangement Design");
    assert_eq!(h.state().arrangement_snapshot().unwrap(), before);
}

/// Compact title bars keep the plus reachable and tab context menus clear of native controls.
#[test]
fn compact_arrangement_controls_stay_reachable() {
    let mut h = harness();
    h.state_mut().integrated_titlebar = true;
    action(&mut h, Action::Create { name: "A deliberately long arrangement name for board review".into(), start: Start::Current });
    for width in [640.0, 760.0, 1280.0] {
        h.set_size(egui::vec2(width, 600.0));
        h.run();
        assert!(h.query_by_label("Arrangement options").is_none());
        let new = h.get_by_label("New arrangement").rect();
        let utilities = h.get_by_label("Preferences").rect();
        assert!(new.right() < utilities.left());
        assert!(h.get_by_label("Close window").rect().right() <= width);
        active_tab_menu(&mut h);
        click(&mut h, "Rename…");
        h.get_by_label("Arrangement name");
        click(&mut h, "Cancel");
        click(&mut h, "New arrangement");
        click(&mut h, "Cancel");
    }
}

/// Context actions address the clicked tab without activating it; duplication uses its own setup.
#[test]
fn inactive_tab_context_actions_preserve_the_working_arrangement() {
    let mut h = harness();
    let design = h.state().arrangement_snapshot().unwrap();
    action(&mut h, Action::Create { name: "Review".into(), start: Start::Current });
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let review = h.state().arrangement_snapshot().unwrap();
    let active = h.state().arrangements.active();
    let view = h.state().views.active.id();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();

    h.get_by_label("Arrangement Design").click_secondary();
    h.run();
    assert_eq!(h.state().arrangements.active(), active);
    click(&mut h, "Rename…");
    h.get_by_label("Arrangement name").type_text("Capture");
    h.run();
    click(&mut h, "Rename");
    h.get_by_label("Arrangement Capture");
    assert_eq!(h.state().arrangements.active(), active);
    assert_eq!(h.state().views.active.id(), view);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), review);

    h.get_by_label("Arrangement Capture").click_secondary();
    h.run();
    click(&mut h, "Duplicate…");
    click(&mut h, "Create");
    h.get_by_label("Arrangement Capture copy");
    assert_eq!(h.state().arrangement_snapshot().unwrap(), design);
    click(&mut h, "Arrangement Review");
    assert_eq!(h.state().arrangement_snapshot().unwrap(), review);
    let view = h.state().views.active.id();
    h.get_by_label("Arrangement Capture").click_secondary();
    h.run();
    click(&mut h, "Delete…");
    click(&mut h, "Delete");
    assert!(h.query_by_label("Arrangement Capture").is_none());
    assert_eq!(h.state().arrangements.active(), active);
    assert_eq!(h.state().views.active.id(), view);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), review);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}
