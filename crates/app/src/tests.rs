//! Drives the real window through its accessibility tree, with clicks and keys, no GPU needed.

use egui::{Key, Modifiers};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use super::*;

#[cfg(not(target_arch = "wasm32"))]
mod detached_editors;
mod display_units;
mod editor_views;
mod grid_spacing;
mod save_changes;
mod split_editors;

/// Start directly in the workspace for editor-control tests; startup tests use the real constructor.
fn harness<'a>() -> Harness<'a, PcbApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), PcbApp::default());
    harness.run();
    harness
}

/// Exercise the same constructor used by both launchers, with no saved application settings.
fn startup_harness<'a>() -> Harness<'a, PcbApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| PcbApp::new(cc));
    harness.run();
    harness
}

/// Click the named control and run the frame that handles the action.
pub(super) fn click(harness: &mut Harness<'_, PcbApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// Open the menu before activating its item in a separate frame.
fn choose(harness: &mut Harness<'_, PcbApp>, menu: &str, item: &str) {
    click(harness, menu);
    click(harness, item);
}

/// Choose a tile, then close the floating library to expose the coordinates used by canvas tests.
fn select_symbol(harness: &mut Harness<'_, PcbApp>, name: &str) {
    harness.get_by_label(name).scroll_to_me();
    harness.run();
    harness.get_by_label(name).click();
    harness.run();
    click(harness, "Close Parts library");
}

/// Create an Untitled project through the File menu; it opens without a naming dialog.
fn new_project(harness: &mut Harness<'_, PcbApp>) {
    choose(harness, "File", "New Project");
}

/// Home's New Project tile. The navigation row shares its label, so take the larger control.
fn new_project_tile<'h>(harness: &'h Harness<'_, PcbApp>) -> egui_kittest::Node<'h> {
    let area = |node: &egui_kittest::Node<'_>| node.rect().area();
    harness.query_all_by_label("New Project").max_by(|a, b| area(a).total_cmp(&area(b))).unwrap()
}

/// Empty the focused name field: select all of it, then delete the selection.
fn clear_name(harness: &mut Harness<'_, PcbApp>) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.key_press(Key::Backspace);
    harness.run();
}

/// An empty workspace reached outside Home identifies the missing project.
#[test]
fn the_empty_workspace_reports_no_project() {
    let harness = harness();
    harness.get_by_label("No project open. Choose File, then New Project.");
    assert!(harness.state().session().project().is_none());
}

/// The first frame shows the Home actions, and cancelling Open does not enter the empty editor.
#[test]
fn startup_opens_home_and_cancelled_dialogs_leave_it_visible() {
    let mut harness = startup_harness();
    new_project_tile(&harness);
    harness.get_by_label("Open Project");
    assert!(harness.state().session().project().is_none());
    assert!(harness.query_by_label("Toggle Net Inspector").is_none());
    assert!(harness.query_by_label("Move Parts library").is_none());
    assert!(harness.query_by_label("No project open. Choose File, then New Project.").is_none());
    assert!(harness.get_by_label("Save").accesskit_node().is_disabled());

    click(&mut harness, "Open Project");
    harness.get_by_label("Project file or archive…");
    click(&mut harness, "Cancel");
    new_project_tile(&harness);
    assert!(harness.state().home);
    assert!(harness.state().session().project().is_none());
}

/// Creating a project from the initial Home page enters an Untitled schematic through the shared
/// command at once, with no naming dialog. Parts can be placed and undone before any save.
#[test]
fn startup_new_project_enters_the_workspace() {
    let mut harness = startup_harness();
    new_project_tile(&harness).click();
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::None));
    harness.get_by_label("Toggle Net Inspector");
    assert!(harness.query_by_label("Move Parts library").is_none());
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("Move Parts library");
    harness.get_by_label("Resistor");
    assert!(harness.query_by_label("Open Project").is_none());
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
    assert!(!harness.state().home);
    harness.get_by_label("Unsaved changes");
}

/// The inspector never reserves space for the empty workspace, even after undoing project creation.
#[test]
fn the_empty_workspace_stays_centered_after_undo_with_the_inspector_open() {
    let mut harness = harness();
    let empty = "No project open. Choose File, then New Project.";
    let original = harness.get_by_label(empty).rect().center();
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    assert!(harness.query_by_label("Close Net Inspector").is_none());

    new_project(&mut harness);
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    harness.get_by_label("Close Net Inspector");
    click(&mut harness, "Undo");

    assert!(harness.query_by_label("Close Net Inspector").is_none());
    assert_eq!(harness.get_by_label(empty).rect().center(), original);
}

/// Creating from the menu opens Untitled without a dialog and leaves Parts available through A.
#[test]
fn a_person_makes_a_project_through_the_menu() {
    let mut harness = harness();
    new_project(&mut harness);
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
    assert!(matches!(harness.state().dialog, Dialog::None));
    assert!(harness.query_by_label("Move Parts library").is_none());
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("Move Parts library");
}

/// Save As stays disabled without a project and opens the format chooser once a project exists.
#[test]
fn compact_file_menu_keeps_save_as_accessible() {
    let mut harness = harness();
    harness.set_size(egui::vec2(420.0, 520.0));
    harness.run();
    harness.get_by_label("File").click();
    harness.run();
    let save_as = harness.get_by_label("Save As…");
    assert!(save_as.accesskit_node().is_disabled());
    assert!(save_as.rect().right() <= 420.0);
    save_as.click();
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::None));
    harness.key_press(Key::Escape);
    harness.run();

    new_project(&mut harness);
    choose(&mut harness, "File", "Save As…");
    harness.get_by_label("Project folder…");
    harness.get_by_label("Packed project (.pc.zip)…");
}

/// The brand and File trigger share the same centerline as the quick actions.
#[test]
fn app_bar_brand_aligns_with_the_controls() {
    let harness = harness();
    let center = |label| harness.get_by_label(label).rect().center().y;
    let file = center("File");
    assert!((center("PCBCraft") - file).abs() <= 1.0);
    assert!((center("Save Project") - file).abs() <= 1.0);
    assert!((center("Preferences") - file).abs() <= 1.0);
}

/// Open Home from the app bar's house button and wait for the page to draw.
fn open_home(harness: &mut Harness<'_, PcbApp>) {
    click(harness, "Home");
    assert!(harness.state().home);
}

/// Home replaces the ribbon and canvas without touching the project, and Escape or the back row
/// return to the same unsaved schematic.
#[test]
fn home_hides_the_workspace_and_returns_to_the_same_project() {
    let mut harness = harness();
    new_project(&mut harness);
    let revision = harness.state().session().revision();
    open_home(&mut harness);
    assert!(harness.query_by_label("Toggle Net Inspector").is_none());
    assert!(harness.query_all_by_label("Start").any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)));
    harness.get_by_label("Unsaved changes");

    harness.key_press(Key::Escape);
    harness.run();
    assert!(!harness.state().home);
    harness.get_by_label("Toggle Net Inspector");

    open_home(&mut harness);
    click(&mut harness, "Back to Untitled");
    assert!(!harness.state().home);
    assert_eq!(harness.state().session().revision(), revision);
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
}

/// The canvas keys stay with the hidden schematic, and undo still reaches the shared history.
#[test]
fn canvas_keys_do_nothing_on_home() {
    let mut harness = harness();
    new_project(&mut harness);
    open_home(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::None));
    harness.key_press(Key::W);
    harness.run();
    assert!(harness.state().views.active.tool.is_none());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(harness.state().session().project().is_none());
    harness.get_by_label("Back to the workspace");
}

/// The New Project tile and the navigation row both run `project.new` and show the new Untitled
/// schematic. Over unsaved work they first ask, as Open does: Cancel keeps the project and Home,
/// and discarding starts a new document whose replacement undo brings back. Ctrl+N asks too.
#[test]
fn home_new_project_tile_creates_the_project_and_returns_to_it() {
    let mut harness = harness();
    open_home(&mut harness);
    assert!(harness.get_by_label("Save").accesskit_node().is_disabled());
    new_project_tile(&harness).click();
    harness.run();
    assert!(!harness.state().home);
    assert!(matches!(harness.state().dialog, Dialog::None), "nothing to lose, so no question");
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
    harness.get_by_label("Toggle Net Inspector");
    let first = harness.state().session().snapshot().unwrap();

    open_home(&mut harness);
    let area = |node: &egui_kittest::Node<'_>| node.rect().area();
    let rail = |harness: &mut Harness<'_, PcbApp>| {
        harness.query_all_by_label("New Project").min_by(|a, b| area(a).total_cmp(&area(b))).unwrap().click();
        harness.run();
    };
    rail(&mut harness);
    harness.get_by_label("Untitled has unsaved changes. If you don't save, they are lost.");
    click(&mut harness, "Cancel");
    assert!(harness.state().home);
    assert!(Arc::ptr_eq(&harness.state().session().snapshot().unwrap(), &first));

    rail(&mut harness);
    click(&mut harness, "Don't Save");
    assert!(!harness.state().home);
    let second = harness.state().session().snapshot().unwrap();
    assert_eq!(second.name, "Untitled");
    assert!(!second.same_document(&first));
    click(&mut harness, "Undo");
    assert!(Arc::ptr_eq(&harness.state().session().snapshot().unwrap(), &first));

    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::SaveChanges(unsaved::Then::New)));
    click(&mut harness, "Cancel");
    assert!(Arc::ptr_eq(&harness.state().session().snapshot().unwrap(), &first));
}

/// Save As on Home returns to the workspace with the format chooser, and Preferences opens over Home.
#[test]
fn home_rows_reach_the_save_and_preferences_dialogs() {
    let mut harness = harness();
    new_project(&mut harness);
    open_home(&mut harness);
    harness.get_by_label("Open Project");
    let preferences = harness.query_all_by_label("Preferences").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap();
    preferences.click();
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::Preferences(_)));
    assert!(harness.state().home);
    click(&mut harness, "Cancel");

    click(&mut harness, "Save As…");
    assert!(!harness.state().home);
    harness.get_by_label("Project folder…");
}

/// At narrow widths the navigation sits above the tiles, and both stay inside the window.
#[test]
fn narrow_home_stacks_the_navigation_above_the_tiles() {
    let mut harness = harness();
    harness.set_size(egui::vec2(420.0, 700.0));
    harness.run();
    open_home(&mut harness);
    let back = harness.get_by_label("Back to the workspace").rect();
    let tile = new_project_tile(&harness).rect();
    assert!(back.right() <= 420.0 && tile.right() <= 420.0, "{back:?} {tile:?}");
    assert!(back.bottom() < tile.top(), "{back:?} should sit above {tile:?}");
}

/// Help under the ? button holds About, Keyboard Shortcuts, Report an Issue and Connect AI
/// assistants, as SolidCraft's does. File has no connection row, and ends with Preferences.
#[test]
fn the_help_menu_holds_about_shortcuts_issues_and_ai_connection() {
    let mut harness = harness();
    click(&mut harness, "File");
    assert!(harness.query_by_label("Local MCP…").is_none());
    assert!(harness.query_by_label("Connect AI assistants…").is_none());
    harness.key_press(Key::Escape);
    harness.run();

    // File ends with Preferences and Keyboard Shortcuts above Quit, as SolidCraft's does.
    choose(&mut harness, "File", "Preferences…");
    harness.get_by_label("Interface scale");
    click(&mut harness, "OK");
    choose(&mut harness, "File", "Keyboard Shortcuts…");
    harness.get_by_label("Draw wire");
    click(&mut harness, "OK");

    choose(&mut harness, "Help", "Keyboard Shortcuts");
    assert!(matches!(harness.state().dialog, Dialog::Preferences(_)));
    harness.get_by_label("Draw wire");
    click(&mut harness, "OK");

    click(&mut harness, "Help");
    harness.get_by_label("Report an Issue…").click();
    harness.step();
    assert!(
        harness
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == BUG_REPORT_URL))
    );

    choose(&mut harness, "Help", "Connect AI assistants…");
    assert!(matches!(harness.state().dialog, Dialog::LocalMcp { .. }));
    harness.get_by_label("Enable local connection");
}

/// The workspace tile opens a picker that lists Schematic as current and the planned
/// workspaces as disabled rows. Clicking a planned row does nothing; Escape, a click outside and
/// choosing Schematic each close the picker. Keys pressed in the picker don't reach the canvas.
#[test]
fn the_workspace_tile_opens_a_picker_with_planned_workspaces() {
    let mut harness = harness();
    let picker_open = |harness: &Harness<'_, PcbApp>| harness.query_by_label("PCB layout").is_some();
    click(&mut harness, "Choose workspace");
    assert!(picker_open(&harness));
    harness.get_by_label("One electronics project across workspaces. PCB layout, 3D rendering and simulation are planned, not implemented.");
    for planned in ["PCB layout", "3D render", "Simulate"] {
        assert!(harness.get_by_label(planned).accesskit_node().is_disabled(), "{planned} is planned");
    }
    assert!(!harness.get_by_label("Schematic").accesskit_node().is_disabled());

    harness.get_by_label("PCB layout").click();
    harness.run();
    assert!(picker_open(&harness), "a planned row doesn't close the picker");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(!picker_open(&harness));

    click(&mut harness, "Choose workspace");
    harness.get_by_label("Schematic").click();
    harness.run();
    assert!(!picker_open(&harness));

    new_project(&mut harness);
    click(&mut harness, "Choose workspace");
    harness.key_press(Key::Enter);
    harness.run();
    assert!(!picker_open(&harness), "Enter picks the current workspace");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.get_by_label("Draw wire (W)");
}

/// Enter on the focused workspace tile opens the picker and leaves it open: the key that opened
/// it doesn't also pick the current workspace. A second Enter picks Schematic and closes it.
#[test]
fn enter_on_the_workspace_tile_opens_the_picker() {
    let mut harness = harness();
    harness.get_by_label("Choose workspace").focus();
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.query_by_label("PCB layout").is_some(), "the opening Enter keeps the picker open");
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.query_by_label("PCB layout").is_none());
}

/// The Wire tool reaches a multi-pin symbol's pins where the library draws them, so the
/// header's pin 4 connects to an LED's anode through the shared command.
#[test]
fn the_wire_tool_connects_multi_pin_library_symbols() {
    let mut harness = harness();
    new_project(&mut harness);
    let grid = pcbcraft_engine::schematic::GRID_NM;
    for (definition, x_nm) in [("header-4", 0), ("led", 20 * grid)] {
        let revision = harness.state().session().revision();
        harness
            .state_mut()
            .session
            .apply(PLACE_SYMBOL, &json!({"definition": definition, "x_nm": x_nm, "y_nm": 0, "expected_revision": revision}))
            .unwrap();
    }
    harness.run();
    click(&mut harness, "Draw wire (W)");
    click_pin(&mut harness, 0, 3);
    click_pin(&mut harness, 1, 0);
    let wires = harness.state().session().inspect()["project"]["schematic"]["wires"].clone();
    assert_eq!(wires[0]["from"], json!({"symbol": "symbol-2", "terminal": "4"}));
    assert_eq!(wires[0]["to"], json!({"symbol": "symbol-3", "terminal": "1"}));
}

/// Below 380 points the ? button gives way, and Home's rail still reaches the same rows.
#[test]
fn narrow_bars_reach_help_rows_from_home() {
    let mut harness = harness();
    harness.set_size(egui::vec2(320.0, 640.0));
    harness.run();
    assert!(harness.query_by_label("Help").is_none());
    open_home(&mut harness);
    harness.get_by_label("About PCBCraft");
    click(&mut harness, "Connect AI assistants…");
    assert!(matches!(harness.state().dialog, Dialog::LocalMcp { .. }));
}

/// The full app bar opens the issue form, and the compact bar keeps the window controls accessible.
#[test]
fn integrated_titlebar_keeps_window_controls_accessible() {
    let mut harness = harness();
    assert!(harness.query_by_label("Maximize window").is_none());
    harness.state_mut().integrated_titlebar = true;
    new_project(&mut harness);
    harness.state_mut().run("project.rename", &json!({"name": "Two resistors"}));
    harness.run();
    let bar_bottom = harness.get_by_label("File").rect().bottom();
    let title = harness.query_all_by_label("Two resistors").find(|node| node.rect().bottom() <= bar_bottom).unwrap();
    assert!(title.rect().width() < 120.0);
    let report = harness.get_by_label("Report a bug");
    assert!(report.rect().right() < harness.get_by_label(&format!("v{VERSION}")).rect().left());
    report.click();
    harness.step();
    assert!(
        harness
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| { matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == BUG_REPORT_URL) })
    );

    harness.set_size(egui::vec2(320.0, 520.0));
    harness.run();

    let preferences = harness.get_by_label("Preferences").rect();
    let minimize = harness.get_by_label("Minimize window").rect();
    let close = harness.get_by_label("Close window").rect();
    assert!(preferences.right() < minimize.left());
    assert!(close.right() <= 320.0);
    assert!(harness.get_by_label("Unsaved changes").rect().right() <= preferences.left());

    harness.get_by_label("Maximize window").click();
    harness.step();
    let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands.contains(&egui::ViewportCommand::Maximized(true)));
}

/// A dirty project keeps its marker when the title no longer fits beside the utilities.
#[test]
fn narrow_app_bar_keeps_the_unsaved_marker() {
    let mut harness = harness();
    let name = "Narrow schematic";
    new_project(&mut harness);
    harness.get_by_label("Unsaved changes");

    // The desktop minimum of 640 pixels leaves 320 logical points at 200% UI scale.
    let width = 320.0;
    harness.set_size(egui::vec2(width, 520.0));
    harness.run();
    let app_bar_bottom = harness.get_by_label("File").rect().bottom();
    assert!(harness.query_all_by_label(name).all(|node| node.rect().top() > app_bar_bottom));
    let marker = harness.get_by_label("Unsaved changes").rect();
    let preferences = harness.get_by_label("Preferences").rect();
    assert!(marker.right() <= preferences.left(), "{marker:?} overlaps {preferences:?}");
    assert!(preferences.right() <= width, "{preferences:?} extends past the window");
    // About moves from the narrow bar to the Home rail, which keeps it inside the window.
    assert!(harness.query_by_label("About PCBCraft").is_none());
    open_home(&mut harness);
    assert!(harness.get_by_label("About PCBCraft").rect().right() <= width);
    harness.key_press(Key::Escape);
    harness.run();

    click(&mut harness, "Undo");
    assert!(harness.query_by_label("Unsaved changes").is_none());
}

/// Undo and Redo from the app bar produce the same project state as the engine commands.
#[test]
fn app_bar_actions_reach_the_same_state_as_commands_an_agent_runs() {
    let mut harness = harness();
    new_project(&mut harness);
    click(&mut harness, "Undo");
    click(&mut harness, "Redo");
    let mut session = Session::default();
    session.execute("project.new", &Value::Null).unwrap();
    session.execute("edit.undo", &Value::Null).unwrap();
    session.execute("edit.redo", &Value::Null).unwrap();
    assert_equivalent_sessions(harness.state().session(), &session);
}

#[test]
fn a_bad_name_stays_in_the_dialog_with_the_reason() {
    let mut harness = harness();
    new_project(&mut harness);
    let revision = harness.state().session().revision();
    choose(&mut harness, "File", "Rename Project…");
    clear_name(&mut harness);
    harness.get_by_label("OK").click();
    harness.run();
    harness.get_by_label("the name can't be empty");
    assert_eq!(harness.state().session().revision(), revision);
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
}

#[test]
fn cancelling_changes_nothing() {
    let mut harness = harness();
    new_project(&mut harness);
    let revision = harness.state().session().revision();
    choose(&mut harness, "File", "Rename Project…");
    harness.get_by_label("Name").type_text(" board");
    harness.run();
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().session().revision(), revision);
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
    assert!(harness.query_by_label("Cancel").is_none());
}

/// Renaming edits the project name independently of parts search and shares undo history.
#[test]
fn rename_starts_from_the_current_name_and_can_be_undone() {
    let mut harness = harness();
    new_project(&mut harness);
    choose(&mut harness, "File", "Rename Project…");
    assert_eq!(harness.get_by_label("Name").value().as_deref(), Some("Untitled"));
    harness.get_by_label("Name").type_text(" v2");
    harness.run();
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled v2");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
}

#[test]
fn undo_and_redo_keys_match_kicad_and_do_nothing_when_there_is_nothing_to_do() {
    let mut harness = harness();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().revision(), 0);
    new_project(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(harness.state().session().project().is_none());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    harness.run();
    assert!(harness.state().session().project().is_some());
}

#[test]
fn typing_ctrl_z_in_a_name_field_does_not_undo_the_project() {
    let mut harness = harness();
    new_project(&mut harness);
    choose(&mut harness, "File", "Rename Project…");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "Untitled");
}

/// The right-side app bar controls reach both dialogs and apply theme and scale changes.
#[test]
fn preferences_change_the_palette_and_the_scale_and_the_about_box_opens() {
    let mut harness = harness();
    click(&mut harness, "Preferences");
    harness.get_by_label("Light").click();
    harness.run();
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Light);
    assert_eq!(harness.ctx.global_style().visuals.panel_fill, Tokens::light().panel);
    harness.get_by_label("125%").click();
    harness.run();
    assert_eq!(harness.state().preferences().scale_percent, 125);
    assert_eq!(harness.ctx.zoom_factor(), 1.25);
    click(&mut harness, "OK");
    assert!(matches!(harness.state().dialog, Dialog::None));
    assert_eq!(harness.state().preferences().scale_percent, 125);
    choose(&mut harness, "Help", "About PCBCraft");
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_none());
}

/// Cancel, Esc and the close button put back what was there when Preferences opened, and
/// Apply moves that point forward.
#[test]
fn preferences_cancel_restores_and_apply_keeps() {
    let mut harness = harness();
    click(&mut harness, "Preferences");
    click(&mut harness, "Light");
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Light);
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Dark);
    assert_eq!(harness.ctx.global_style().visuals.panel_fill, Tokens::dark().panel);

    click(&mut harness, "Preferences");
    assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
    click(&mut harness, "110%");
    click(&mut harness, "Apply");
    click(&mut harness, "Light");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::None));
    assert_eq!((harness.state().preferences().theme, harness.state().preferences().scale_percent), (ThemeChoice::Dark, 110));

    click(&mut harness, "Preferences");
    click(&mut harness, "Light");
    click(&mut harness, "Close Preferences");
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Dark);
}

/// Adding an agent from the AI page returns to the same Preferences window, so Cancel there
/// still puts back changes made before the agent dialog opened.
#[test]
fn adding_an_agent_from_preferences_returns_to_the_same_window() {
    let mut harness = harness();
    click(&mut harness, "Preferences");
    click(&mut harness, "Light");
    click(&mut harness, "AI");
    harness.get_by_label("Show AI features");
    click(&mut harness, "Add agent…");
    assert!(matches!(harness.state().dialog, Dialog::Agent { return_to: Some(_), .. }), "{:?}", harness.state().dialog);
    click(&mut harness, "Cancel");
    assert!(matches!(harness.state().dialog, Dialog::Preferences(_)));
    harness.get_by_label("Show AI features");
    click(&mut harness, "Cancel");
    assert!(matches!(harness.state().dialog, Dialog::None));
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Dark);
}

/// Restore page defaults resets the open page only.
#[test]
fn preferences_restore_defaults_for_the_open_page() {
    let mut harness = harness();
    click(&mut harness, "Preferences");
    click(&mut harness, "Light");
    click(&mut harness, "Navigation");
    click(&mut harness, "Reversed");
    click(&mut harness, "Center of view");
    assert!(harness.state().preferences().navigation.reverse_zoom);
    click(&mut harness, "Restore page defaults");
    assert_eq!(harness.state().preferences().navigation, Navigation::default());
    assert_eq!(harness.state().preferences().theme, ThemeChoice::Light);
    click(&mut harness, "Shortcuts");
    assert!(harness.get_by_label("Restore page defaults").accesskit_node().is_disabled());
    harness.get_by_label("Draw wire");
}

/// Search shows matching rows from every page, finds Theme by "dark", and Enter opens the
/// result's page with the search cleared.
#[test]
fn preferences_search_filters_rows_across_pages() {
    let mut harness = harness();
    click(&mut harness, "Preferences");
    let search = |harness: &mut Harness<'_, PcbApp>, text: &str| {
        harness.get_by_role(egui::accesskit::Role::TextInput).click();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text(text);
        harness.run();
        harness.run();
    };
    search(&mut harness, "zoom");
    assert!(harness.get_by_label("Restore page defaults").accesskit_node().is_disabled(), "results span pages");
    harness.get_by_label("Zoom speed");
    harness.get_by_label("Zoom direction");
    assert!(harness.query_by_label("Theme").is_none());
    assert!(harness.query_by_label("Add symbol").is_none());

    search(&mut harness, "dark");
    harness.get_by_label("Theme");
    assert!(harness.query_by_label("Zoom speed").is_none());

    search(&mut harness, "wire");
    harness.get_by_label("Draw wire");
    harness.key_press(Key::Enter);
    harness.run();
    harness.run();
    assert!(harness.query_by_label("Zoom speed").is_none());
    assert!(harness.query_all_by_label("Shortcuts").any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)));
    harness.get_by_label("Add symbol");

    search(&mut harness, "qqq");
    harness.get_by_label("No settings match “qqq”");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::Preferences(_)), "Esc clears the search first");
    harness.get_by_label("Add symbol");
}

/// Grid opacity is on Appearance and found by searching "grid". Its slider moves in 5% steps,
/// Reset returns it to the default, and Cancel puts back what was there when the window opened.
#[test]
fn grid_opacity_preference_steps_resets_and_cancels() {
    let mut harness = harness();
    harness.state_mut().preferences.grid_opacity_percent = 40;
    click(&mut harness, "Preferences");
    harness.get_by_label("Grid opacity");
    // The percentage box beside it reads "40%"; the slider itself carries the number.
    harness.get_by_value("40%");
    let slider = harness.query_all_by_role(egui::accesskit::Role::Slider).find(|node| node.accesskit_node().numeric_value() == Some(40.0));
    let (target_node, target_tree) = slider.unwrap().accesskit_node().locate();
    let action = egui::accesskit::Action::Decrement;
    harness.event(egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest { action, target_node, target_tree, data: None }));
    harness.run();
    assert_eq!(harness.state().preferences().grid_opacity_percent, 35);
    click(&mut harness, "Reset");
    assert_eq!(harness.state().preferences().grid_opacity_percent, pcbcraft_chrome::grid::OPACITY_DEFAULT);
    assert!(harness.get_by_label("Reset").accesskit_node().is_disabled());
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().preferences().grid_opacity_percent, 40);

    click(&mut harness, "Preferences");
    harness.get_by_role(egui::accesskit::Role::TextInput).click();
    harness.run();
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("grid");
    harness.run();
    harness.run();
    harness.get_by_label("Grid opacity");
    assert!(harness.query_by_label("Theme").is_none());
}

/// Hiding the grid changes neither where a click places a part nor the project: no revision,
/// no dirty marker, no undo step and the same saved bytes.
#[test]
fn hiding_the_grid_keeps_snapping_and_leaves_the_project_untouched() {
    let mut harness = harness();
    new_project(&mut harness);
    let before = harness.state().session().snapshot().unwrap();
    let revision = harness.state().session().revision();
    let dirty = harness.state().files.dirty(&harness.state().session);
    for percent in [0, 35, 100] {
        harness.state_mut().preferences.grid_opacity_percent = percent;
        harness.run();
        assert_eq!(harness.state().session().revision(), revision);
        assert_eq!(harness.state().files.dirty(&harness.state().session), dirty);
    }
    let after = harness.state().session().snapshot().unwrap();
    assert_eq!(pcbcraft_engine::persistence::encode(after).unwrap(), pcbcraft_engine::persistence::encode(before).unwrap());

    harness.state_mut().preferences.grid_opacity_percent = 0;
    click(&mut harness, "Add symbol (A)");
    select_symbol(&mut harness, "Resistor");
    click_canvas_offset(&mut harness, egui::vec2(40.64 + 3.0, -2.0));
    let schematic = harness.state().session().project().unwrap();
    assert_eq!(schematic.symbols()[0].position, pcbcraft_engine::schematic::Point { x_nm: 4 * pcbcraft_engine::schematic::GRID_NM, y_nm: 0 });
    click(&mut harness, "Undo");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
}

/// With scrolling set to pan, the wheel moves the schematic instead of zooming it.
#[test]
fn navigation_preferences_change_what_the_wheel_does() {
    let mut harness = harness();
    new_project(&mut harness);
    let wheel = |harness: &mut Harness<'_, PcbApp>, modifiers| {
        harness.hover_at(canvas_center(harness));
        harness.run();
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 2.0),
            phase: egui::TouchPhase::Move,
            modifiers,
        });
        harness.run();
    };
    wheel(&mut harness, Modifiers::NONE);
    let zoomed = harness.state().views.active.camera.zoom();
    assert!(zoomed > 1.0);

    harness.state_mut().preferences.navigation.scroll = ScrollAction::Pan;
    wheel(&mut harness, Modifiers::NONE);
    assert_eq!(harness.state().views.active.camera.zoom(), zoomed);
    wheel(&mut harness, Modifiers::COMMAND);
    assert!(harness.state().views.active.camera.zoom() > zoomed);
}

/// Capture placement snaps through the shared engine command and can be undone from the app bar.
#[test]
fn canvas_placement_uses_the_shared_command_and_snaps_to_the_grid() {
    let mut harness = harness();
    new_project(&mut harness);
    click(&mut harness, "Add symbol (A)");
    select_symbol(&mut harness, "Resistor");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.get_by_label("Place Resistor · R rotate · Esc to cancel");

    // A click near the center snaps to the schematic origin.
    click_canvas(&mut harness);
    let mut session = Session::default();
    session.execute("project.new", &Value::Null).unwrap();
    session.execute(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": 1})).unwrap();
    assert_equivalent_sessions(harness.state().session(), &session);
    click_canvas(&mut harness);
    assert_equivalent_sessions(harness.state().session(), &session);
    click(&mut harness, "Undo");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    click(&mut harness, "Redo");
    assert_eq!(harness.state().session().project().unwrap().symbols(), session.project().unwrap().symbols());
}

/// Escape leaves library search, then cancels placement; opening a dialog also cancels placement.
#[test]
fn escape_and_opening_a_dialog_cancel_the_placement_preview() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("LED");
    harness.key_press(Key::Escape);
    harness.run();
    harness.get_by_label("LED");
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());

    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    harness.get_by_label("Place Resistor · R rotate · Esc to cancel");
    harness.key_press(Key::Escape);
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());

    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    choose(&mut harness, "File", "Rename Project…");
    harness.get_by_label("Cancel").click();
    harness.run();
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
}

/// Locate the actual canvas center after the surrounding workspace panels have laid out.
fn canvas_center(harness: &Harness<'_, PcbApp>) -> egui::Pos2 {
    harness.get_by_label("Schematic canvas").rect().center()
}

/// Click at a known offset from the center of the canvas at the default camera zoom.
fn click_canvas_offset(harness: &mut Harness<'_, PcbApp>, offset: egui::Vec2) {
    let point = canvas_center(harness) + offset;
    click_canvas_at(harness, point);
}

fn click_canvas(harness: &mut Harness<'_, PcbApp>) {
    click_canvas_offset(harness, egui::vec2(2.0, 0.0));
}

/// Send a real pointer click to the canvas at a specified logical position.
fn click_canvas_at(harness: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
    harness.hover_at(point);
    harness.run();
    harness.drag_at(point);
    harness.run();
    harness.drop_at(point);
    harness.run();
}

/// The grid coordinate under the zoom cursor follows the middle-button pan.
#[test]
fn wheel_zoom_and_middle_pan_preserve_the_placement_coordinate() {
    let mut harness = harness();
    new_project(&mut harness);
    let point = canvas_center(&harness) + egui::vec2(40.64, 0.0);
    harness.hover_at(point);
    harness.run();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 4.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    harness.run();
    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    let middle = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    harness.event(middle(point, true));
    harness.run();
    let panned = point + egui::vec2(120.0, 65.0);
    harness.hover_at(panned);
    harness.run();
    harness.event(middle(panned, false));
    harness.run();
    assert_eq!(harness.state().session().revision(), 1);
    click_canvas_at(&mut harness, panned);
    let schematic = harness.state().session().project().unwrap();
    assert_eq!(schematic.symbols()[0].position, pcbcraft_engine::schematic::Point { x_nm: 4 * pcbcraft_engine::schematic::GRID_NM, y_nm: 0 });
}

/// The Fit action restores the default canvas origin without advancing the project revision.
#[test]
fn zoom_to_fit_on_an_empty_sheet_restores_the_default_origin() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.hover_at(egui::pos2(600.0, 400.0));
    harness.run();
    harness.event(egui::Event::Zoom(2.0));
    harness.run();
    click(&mut harness, "Fit schematic");
    assert_eq!(harness.state().session().revision(), 1);
    click(&mut harness, "Add symbol (A)");
    select_symbol(&mut harness, "Resistor");
    click_canvas(&mut harness);
    assert_eq!(harness.state().session().project().unwrap().symbols()[0].position, pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 });
}

/// Resolve the actual library pin through the canvas camera so fixture geometry can evolve.
fn pin_screen(harness: &Harness<'_, PcbApp>, symbol: usize, pin: usize) -> egui::Pos2 {
    let point = harness.state().session().project().unwrap().symbols()[symbol].pins()[pin];
    harness.state().views.active.camera.screen(point, harness.get_by_label("Schematic canvas").rect())
}

/// Click an exact pin without assuming a particular symbol-library revision.
fn click_pin(harness: &mut Harness<'_, PcbApp>, symbol: usize, pin: usize) {
    let point = pin_screen(harness, symbol, pin);
    click_canvas_at(harness, point);
}

/// Place a resistor and LED through the canvas so their exact terminal positions are known.
fn wiring_project(harness: &mut Harness<'_, PcbApp>) {
    new_project(harness);
    click(harness, "Add symbol (A)");
    select_symbol(harness, "Resistor");
    click_canvas(harness);
    click(harness, "Add symbol (A)");
    select_symbol(harness, "LED");
    harness.get_by_label("Place LED · R rotate · Esc to cancel");
    click_canvas_offset(harness, egui::vec2(203.2, 60.96));
    assert_eq!(harness.state().session().project().unwrap().symbols()[1].definition.id, "led");
}

/// Capture wiring produces a shared connection that the app bar can undo and redo.
#[test]
fn the_wire_tool_runs_the_shared_command_and_is_undoable() {
    let mut harness = harness();
    wiring_project(&mut harness);
    click(&mut harness, "Draw wire (W)");
    click_pin(&mut harness, 0, 1);
    harness.get_by_label("Wire · Click corners · Enter finish · Space flip · Esc cancel");
    assert_eq!(harness.state().session().revision(), 3);
    click_pin(&mut harness, 1, 0);

    let inspected = harness.state().session().inspect();
    let schematic = &inspected["project"]["schematic"];
    assert_eq!(
        schematic["wires"],
        json!([{
            "id": "wire-4",
            "from": {"symbol": "symbol-2", "terminal": "2"},
            "to": {"symbol": "symbol-3", "terminal": "1"},
            "bends": [{"x_nm": 21_590_000, "y_nm": 0}],
        }])
    );
    assert_eq!(schematic["netlist"]["nets"].as_array().unwrap().len(), 1);
    click(&mut harness, "Undo");
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    click(&mut harness, "Redo");
    assert_eq!(&harness.state().session().inspect()["project"]["schematic"], schematic);
}

/// At a compact width, ribbon actions remain reachable and the right card leaves canvas room.
#[test]
fn compact_ribbon_places_and_wires_symbols_with_a_docked_inspector() {
    let mut harness = harness();
    harness.set_size(egui::vec2(400.0, 600.0));
    harness.run();
    new_project(&mut harness);
    // The floating Browser would cover the narrow canvas where the parts go.
    click(&mut harness, "View");
    click(&mut harness, "Toggle Browser");
    click(&mut harness, "Capture");

    // In a narrow window the parts list scrolls, so search for each part and press Enter.
    for (name, x) in [("Resistor", -81.28), ("LED", 81.28)] {
        harness.get_by_label("Add symbol (A)").click();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text(name);
        harness.run();
        harness.key_press(Key::Enter);
        harness.run();
        click(&mut harness, "Close Parts library");
        click_canvas_offset(&mut harness, egui::vec2(x, 0.0));
    }
    harness.get_by_label("Draw wire (W)").click();
    harness.run();
    click_pin(&mut harness, 0, 1);
    click_pin(&mut harness, 1, 0);
    let wired = harness.state().session().inspect();
    assert_eq!(wired["project"]["schematic"]["wires"].as_array().unwrap().len(), 1);

    let full_width = harness.get_by_label("Schematic canvas").rect().width();
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    harness.get_by_label("R1.2, D1.1");
    let canvas = harness.get_by_label("Schematic canvas").rect();
    let close = harness.get_by_label("Close Net Inspector");
    assert!(canvas.width() >= 0.6 * full_width - 1.0, "canvas: {canvas:?}");
    assert!(close.rect().left() >= canvas.right());
    close.click();
    harness.run();
    assert!(harness.query_by_label("R1.2, D1.1").is_none());
    assert_eq!(harness.state().session().inspect(), wired);
}

#[test]
fn wiring_accepts_empty_space_and_escape_discards_the_preview() {
    let mut harness = harness();
    wiring_project(&mut harness);
    let before = harness.state().session().inspect();
    harness.key_press(Key::W);
    harness.run();
    click_pin(&mut harness, 0, 1);
    click_canvas_offset(&mut harness, egui::vec2(100.0, 36.0));
    harness.get_by_label("Wire · Click corners · Enter finish · Space flip · Esc cancel");
    harness.key_press(Key::Escape);
    harness.run();
    click_pin(&mut harness, 1, 0);
    assert_eq!(harness.state().session().inspect(), before);
}

/// A opens Parts and Enter places the first match without hiding the card or moving the canvas.
/// Escape leaves search focus and preserves its query.
#[test]
fn the_parts_browser_searches_and_places_parts() {
    let mut harness = harness();
    new_project(&mut harness);
    assert!(harness.query_by_label("Move Parts library").is_none());
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("Move Parts library");
    harness.get_by_label("2-pin connector");
    harness.get_by_label("Close Parts library");
    let canvas = harness.get_by_label("Schematic canvas").rect();
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("cap");
    harness.run();
    assert!(harness.query_by_label("Resistor").is_none());
    harness.get_by_label("Capacitor");
    harness.key_press(Key::Enter);
    harness.run();
    harness.get_by_label("Move Parts library");
    harness.get_by_label("Capacitor");
    assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
    harness.get_by_label("Place Capacitor · R rotate · Esc to cancel");
    click(&mut harness, "Parts library position");
    click(&mut harness, "Dock left");
    click_canvas(&mut harness);
    assert_eq!(first_symbol(&harness, "kind"), "capacitor");

    harness.key_press(Key::A);
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    harness.get_by_label("Move Parts library");
    assert_eq!(harness.get_by_role(egui::accesskit::Role::TextInput).value().as_deref(), Some("cap"));
    assert!(harness.state().views.active.tool.is_none());
    harness.key_press(Key::W);
    harness.run();
    assert!(matches!(harness.state().views.active.tool, Some(Tool::Wire { .. })), "Escape must release search focus for canvas shortcuts");
}

/// Click selection, search Escape and tool cancellation keep the card and canvas bounds stable.
#[test]
fn clicking_parts_and_cancelling_placement_keep_the_browser_visible() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    click(&mut harness, "Parts library position");
    click(&mut harness, "Dock left");
    let canvas = harness.get_by_label("Schematic canvas").rect();
    click(&mut harness, "Capacitor");
    harness.get_by_label("Move Parts library");
    assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
    harness.key_press(Key::R);
    harness.run();
    assert!(matches!(harness.state().views.active.tool, Some(Tool::PlaceSymbol { turns: 1, .. })));
    harness.key_press(Key::A);
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.state().views.active.tool.is_some(), "Escape from search releases focus before cancelling placement");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.state().views.active.tool.is_none());
    harness.get_by_label("Move Parts library");
    assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
    click(&mut harness, "Resistor");
    click_canvas(&mut harness);
    assert_eq!(first_symbol(&harness, "kind"), "resistor");
    harness.get_by_label("Move Parts library");
    assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
}

/// Popups, dialogs and Home handle Escape before search; returning from Home restores the card.
#[test]
fn the_browser_leaves_escape_to_overlays_and_home() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    click(&mut harness, "File");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("New Project…").is_none());
    harness.get_by_label("Move Parts library");

    click(&mut harness, "Preferences");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(matches!(harness.state().dialog, Dialog::None));
    harness.get_by_label("Move Parts library");

    click(&mut harness, "Home");
    assert!(harness.query_by_label("Move Parts library").is_none());
    harness.key_press(Key::Escape);
    harness.run();
    assert!(!harness.state().home);
    harness.get_by_label("Move Parts library");
    harness.key_press(Key::Escape);
    harness.run();
    harness.get_by_label("Move Parts library");
}

/// Dragging a part previews it on the snapped grid before release, places it through the shared
/// command, and leaves the browser open for the next one.
#[test]
fn dragging_a_part_onto_the_sheet_places_it() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    let from = harness.get_by_label("Capacitor").rect().center();
    let to = canvas_center(&harness) + egui::vec2(306.0, 3.0);
    harness.event(egui::Event::PointerMoved(from));
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run();
    for step in 1..=6 {
        harness.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0)));
        harness.run();
    }
    let canvas = harness.get_by_label("Schematic canvas").rect();
    let preview = harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == Tokens::get().accent && points.iter().all(|point| canvas.contains(*point)) =>
            {
                Some(egui::Rect::from_two_pos(points[0], points[1]))
            }
            _ => None,
        })
        .reduce(|a, b| a.union(b))
        .expect("The dragged part must have a visible preview before release");
    let snapped = canvas.center() + egui::vec2(304.8, 0.0);
    assert!(preview.center().distance(snapped) < 0.1, "The preview must snap to the placement grid: {preview:?}");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run();
    assert_eq!(first_symbol(&harness, "kind"), "capacitor");
    assert_eq!(first_symbol(&harness, "x_nm"), 30 * pcbcraft_engine::schematic::GRID_NM);
    assert_eq!(first_symbol(&harness, "y_nm"), 0);
    harness.get_by_label("Move Parts library");
}

/// An edit during a browser drag invalidates that gesture without reverting the intervening edit.
#[test]
fn an_intervening_edit_cancels_a_browser_drag() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    harness.key_press(Key::A);
    harness.run();
    let from = harness.get_by_label("Capacitor").rect().center();
    let to = canvas_center(&harness) + egui::vec2(304.8, 0.0);
    harness.drag_at(from);
    harness.run();
    harness.hover_at(from + egui::vec2(8.0, 0.0));
    harness.run();
    harness.hover_at(to);
    harness.run();

    let revision = harness.state().session().revision();
    let params = json!({"symbol": "symbol-2", "x_nm": 0, "y_nm": 1_270_000, "expected_revision": revision});
    harness.state_mut().session.apply(MOVE_SYMBOL, &params).unwrap();
    let after_edit = harness.state().session().inspect();
    harness.run();
    harness.drop_at(to);
    harness.run();
    assert_eq!(harness.state().session().inspect(), after_edit, "A stale browser drag must not place a symbol");
}

/// Place a resistor at the schematic origin, which sits at the canvas center.
fn resistor_at_origin(harness: &mut Harness<'_, PcbApp>) {
    new_project(harness);
    let revision = harness.state().session().revision();
    harness.state_mut().session.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": revision})).unwrap();
    harness.run();
}

/// The first symbol's field as project inspection reports it.
fn first_symbol(harness: &Harness<'_, PcbApp>, field: &str) -> Value {
    harness.state().session().inspect()["project"]["schematic"]["symbols"][0][field].clone()
}

/// Click a symbol to select it, then R rotates it, E edits its fields and Delete removes it,
/// each through the shared commands. Escape clears the selection.
#[test]
fn a_selected_symbol_rotates_edits_and_deletes_with_kicad_keys() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    click_canvas(&mut harness);
    assert_eq!(harness.state().views.active.selection.single_symbol(), Some("symbol-2"));
    harness.get_by_label("R1 selected · R rotate · E edit fields · Del delete");

    harness.key_press(Key::R);
    harness.run();
    assert_eq!(first_symbol(&harness, "rotation"), 90);

    harness.key_press(Key::E);
    harness.run();
    let value = harness.get_by_label("Value");
    value.click();
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.get_by_label("Value").type_text("4.7k");
    harness.run();
    click(&mut harness, "OK");
    assert_eq!(first_symbol(&harness, "value"), "4.7k");
    assert!(matches!(harness.state().dialog, Dialog::None));

    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.state().views.active.selection.is_empty());
    click_canvas(&mut harness);
    harness.key_press(Key::Delete);
    harness.run();
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    assert!(harness.state().views.active.selection.is_empty());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(first_symbol(&harness, "value"), "4.7k");
}

/// Right-clicking a symbol selects it and offers Rotate, Edit Fields and Delete. A taken
/// reference stays in the Edit Fields dialog with the reason.
#[test]
fn the_symbol_context_menu_rotates_and_edits() {
    let mut harness = harness();
    resistor_at_origin(&mut harness);
    let point = canvas_center(&harness);
    harness.hover_at(point);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.run();
    }
    assert_eq!(harness.state().views.active.selection.single_symbol(), Some("symbol-2"));
    harness.get_by_label("Delete");
    click(&mut harness, "Rotate");
    assert_eq!(first_symbol(&harness, "rotation"), 90);

    let revision = harness.state().session().revision();
    harness
        .state_mut()
        .session
        .apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 25_400_000, "y_nm": 0, "expected_revision": revision}))
        .unwrap();
    harness.state_mut().views.active.selection = Selection::one_symbol("symbol-2".to_owned());
    harness.state_mut().edit_selected(SymbolAction::EditFields);
    harness.run();
    let reference = harness.get_by_label("Reference");
    reference.click();
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.get_by_label("Reference").type_text("R2");
    harness.run();
    click(&mut harness, "OK");
    harness.get_by_label("another component is already R2");
    assert!(matches!(harness.state().dialog, Dialog::EditFields { .. }));
}

fn open_wire_context(harness: &mut Harness<'_, PcbApp>) {
    let point = canvas_center(harness) + egui::vec2(80.0, 26.0);
    harness.hover_at(point);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.run();
    }
    harness.get_by_label("Cancel");
}

#[test]
fn dismissing_the_wire_menu_over_a_pin_does_not_commit() {
    let mut harness = harness();
    wiring_project(&mut harness);
    harness.key_press(Key::W);
    harness.run();
    click_pin(&mut harness, 0, 1);
    open_wire_context(&mut harness);
    let destination = pin_screen(&harness, 1, 0);
    click_canvas_at(&mut harness, destination);
    assert!(harness.query_by_label("Cancel").is_none());
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    harness.get_by_label("Wire · Click corners · Enter finish · Space flip · Esc cancel");
    click_canvas_at(&mut harness, destination);
    assert_eq!(harness.state().session().project().unwrap().wires().len(), 1);
}

/// Both cancellation paths discard the draft, with no revision or undo change.
#[test]
fn the_wire_menu_cancel_button_and_escape_both_discard_the_preview() {
    let mut harness = harness();
    wiring_project(&mut harness);
    let before = harness.state().session().inspect();
    for escape in [false, true] {
        harness.key_press(Key::W);
        harness.run();
        click_pin(&mut harness, 0, 1);
        open_wire_context(&mut harness);
        if escape {
            harness.key_press(Key::Escape);
        } else {
            harness.get_by_label("Cancel").click();
        }
        harness.run();
        assert!(harness.query_by_label("Cancel").is_none());
        assert!(harness.query_by_label("Wire · Click corners · Enter finish · Space flip · Esc cancel").is_none());
        click_pin(&mut harness, 1, 0);
        assert_eq!(harness.state().session().inspect(), before);
    }
}

/// Drag a wired symbol, keeping the preview outside history and committing one move on release.
/// A drag preserves the wire and can be reverted with one click on Undo.
#[test]
fn dragging_a_symbol_preserves_connectivity_and_is_one_undo_step() {
    let mut harness = harness();
    wiring_project(&mut harness);
    click(&mut harness, "Draw wire (W)");
    click_pin(&mut harness, 0, 1);
    click_pin(&mut harness, 1, 0);
    let before = harness.state().session().snapshot().unwrap();
    let revision = harness.state().session().revision();
    let start = canvas_center(&harness);
    let end = start + egui::vec2(0.0, 40.64);
    harness.drag_at(start);
    harness.run();
    harness.hover_at(end);
    harness.run();
    assert_eq!(harness.state().session().revision(), revision);
    harness.drop_at(end);
    harness.run();
    let moved = harness.state().session().project().unwrap();
    assert_eq!(harness.state().session().revision(), revision + 1);
    assert_eq!(moved.symbols()[0].position.y_nm, 4 * pcbcraft_engine::schematic::GRID_NM);
    assert!(before.same_connectivity(moved));
    assert_eq!(before.wires(), moved.wires());
    click(&mut harness, "Undo");
    assert_eq!(harness.state().session().project().unwrap().symbols(), before.symbols());
}

#[cfg(not(target_arch = "wasm32"))]
mod live_mcp {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use pcbcraft_mcp::{
        PROTOCOL_VERSION, Server,
        live::{Remote, TOKEN_ENV},
    };

    use super::*;

    /// Run the MCP request on a separate thread while the real editor handles its frames.
    fn tool(harness: &mut Harness<'_, PcbApp>, connection: (u16, &str), name: &str, arguments: Value) -> Value {
        thread::scope(|scope| {
            let request = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
            let worker = scope.spawn(move || {
                let mut server = Server::new(Box::new(Remote::new(connection.0, connection.1).unwrap()));
                let initialize = server
                    .handle(json!({
                        "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": PROTOCOL_VERSION}
                    }))
                    .unwrap();
                assert_eq!(initialize["result"]["protocolVersion"], PROTOCOL_VERSION);
                server.handle(request)
            });
            let deadline = Instant::now() + Duration::from_secs(6);
            while !worker.is_finished() && Instant::now() < deadline {
                harness.run();
                thread::sleep(Duration::from_millis(2));
            }
            assert!(worker.is_finished(), "MCP call timed out: {name}");
            worker.join().unwrap().unwrap()["result"].clone()
        })
    }

    /// Read the structured tool payload and report an MCP failure if the call was refused.
    fn payload(result: Value) -> Value {
        assert_ne!(result["isError"], true, "{result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    /// Locate the painted wire in the frame, including the orthogonal path after a pin moves.
    fn painted_wire(harness: &Harness<'_, PcbApp>) -> Vec<egui::Pos2> {
        let color = Tokens::get().schematic_wire;
        harness
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == color => Some(points.to_vec()),
                egui::Shape::Path(path) if matches!(path.stroke.color, egui::epaint::ColorMode::Solid(ink) if ink == color) => {
                    Some(path.points.clone())
                }
                _ => None,
            })
            .unwrap()
    }

    /// Move a wired resistor through MCP, verify the canvas and undo history, then revoke access.
    #[test]
    fn local_mcp_moves_the_open_schematic_and_shares_undo() {
        let mut harness = harness();
        assert!(harness.state().live_control.is_none());
        wiring_project(&mut harness);
        click(&mut harness, "Draw wire (W)");
        click_pin(&mut harness, 0, 1);
        click_pin(&mut harness, 1, 0);
        let before_wire = painted_wire(&harness);

        choose(&mut harness, "Help", "Connect AI assistants…");
        click(&mut harness, "Enable local connection");
        click(&mut harness, "Copy configuration");
        let config = harness.state().live_control.as_ref().unwrap().client_configuration();
        let config: Value = serde_json::from_str(&config).unwrap();
        let settings = &config["mcpServers"]["pcbcraft-live"];
        let port: u16 = settings["args"][2].as_str().unwrap().parse().unwrap();
        let token = settings["env"][TOKEN_ENV].as_str().unwrap();
        assert_eq!(settings["args"], json!(["mcp", "--connect", port.to_string()]));
        click(&mut harness, "Close");

        let connection = (port, token);
        let commands = payload(tool(&mut harness, connection, "list_commands", json!({})));
        assert_eq!(commands.as_array().unwrap().len(), 1);
        assert_eq!(commands[0]["id"], MOVE_SYMBOL);
        let before = payload(tool(&mut harness, connection, "inspect_project", json!({})));
        let from = &before["project"]["schematic"];
        let request = json!({"command": MOVE_SYMBOL, "params": {
            "symbol": from["symbols"][0]["id"], "x_nm": 0, "y_nm": 5_080_000, "expected_revision": before["revision"]
        }});
        let moved = payload(tool(&mut harness, connection, "execute", request.clone()));
        let to = &moved["project"]["schematic"];
        assert_eq!(to["symbols"][0]["y_nm"], 5_080_000);
        assert_eq!(to["wires"], from["wires"]);
        let after_wire = painted_wire(&harness);
        assert_ne!(after_wire, before_wire);
        assert_eq!(after_wire.last(), before_wire.last());
        assert!(harness.state().files.dirty(&harness.state().session));

        let stale = tool(&mut harness, connection, "execute", request);
        assert_eq!(stale["isError"], true);
        assert!(stale["content"][0]["text"].as_str().unwrap().contains("project changed"));
        let denied = tool(&mut harness, connection, "execute", json!({"command": "project.new", "params": {"name": "other"}}));
        assert_eq!(denied["isError"], true);
        assert_eq!(harness.state().session().inspect(), moved);

        click(&mut harness, "Undo");
        let undone = payload(tool(&mut harness, connection, "inspect_project", json!({})));
        assert_eq!(undone["project"], before["project"]);
        assert_eq!(painted_wire(&harness), before_wire);
        click(&mut harness, "Redo");
        let redone = payload(tool(&mut harness, connection, "inspect_project", json!({})));
        assert_eq!(redone["project"], moved["project"]);
        assert_eq!(painted_wire(&harness), after_wire);

        choose(&mut harness, "Help", "Connect AI assistants…");
        click(&mut harness, "Disable connection");
        assert!(harness.state().live_control.is_none());
        let disconnected = tool(&mut harness, connection, "inspect_project", json!({}));
        assert_eq!(disconnected["isError"], true);
    }
}

/// A placement preview rotates without editing history, and hovering another part takes priority
/// over a previous selection when R is pressed on the canvas.
#[test]
fn placement_and_hover_rotation_use_one_shared_edit_each() {
    let mut harness = harness();
    new_project(&mut harness);
    click(&mut harness, "Add symbol (A)");
    select_symbol(&mut harness, "Resistor");
    let before = harness.state().session().inspect();
    for _ in 0..2 {
        harness.key_press(Key::R);
        harness.run();
    }
    assert_eq!(harness.state().session().inspect(), before);
    click_canvas(&mut harness);
    assert_eq!(first_symbol(&harness, "rotation"), 180);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    harness.run();
    let revision = harness.state().session().revision();
    harness.state_mut().session.apply(PLACE_SYMBOL, &json!({"definition":"led","x_nm":25_400_000,"y_nm":0,"expected_revision":revision})).unwrap();
    harness.run();
    click_canvas(&mut harness);
    let point = canvas_center(&harness) + egui::vec2(203.2, 0.0);
    harness.hover_at(point);
    harness.run();
    harness.key_press(Key::R);
    harness.run();
    let project = harness.state().session().project().unwrap();
    assert_eq!(project.symbols()[0].turns(), 2);
    assert_eq!(project.symbols()[1].turns(), 1);
}

/// Empty-space routes freeze corners locally, support backtracking/elbow flips, and commit once.
/// Explicit whole-wire selection and Delete remove the edge; one undo restores its geometry
/// and endpoint identities, while a plain click remains a segment selection.
#[test]
fn free_wire_gesture_finishes_saves_and_deletes_with_shared_history() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.key_press(Key::W);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(-101.6, -101.6));
    click_canvas_offset(&mut harness, egui::vec2(0.0, -50.8));
    assert_eq!(harness.state().session().revision(), 1);
    harness.key_press(Key::Backspace);
    harness.run();
    harness.key_press(Key::Space);
    harness.run();
    let finish = canvas_center(&harness) + egui::vec2(101.6, 101.6);
    harness.hover_at(finish);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().views.active.tool.is_none());
    let before = harness.state().session().snapshot().unwrap();
    assert_eq!(before.wires().len(), 1);
    let path = before.wire_path(&before.wires()[0]);
    assert_eq!(path.len(), 3);
    assert_eq!(path[0].x_nm, path[1].x_nm, "Space selects the vertical first elbow");
    let bytes = pcbcraft_engine::persistence::encode(before.clone()).unwrap();
    let reopened = pcbcraft_engine::persistence::decode(&bytes).unwrap();
    assert_eq!(reopened.inspect_schematic(), before.inspect_schematic());
    click_canvas_offset(&mut harness, egui::vec2(-101.6, 0.0));
    assert!(harness.state().views.active.selection.single_wire().is_some());
    let pos = canvas_center(&harness) + egui::vec2(-101.6, 0.0);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.run();
    }
    click(&mut harness, "Select Wire");
    assert!(harness.state().views.active.selected_leg.is_none());
    harness.key_press(Key::Delete);
    harness.run();
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().inspect_schematic(), before.inspect_schematic());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(harness.state().session().project().unwrap().wires().is_empty());
}

/// A concurrent command cancels an unfinished route, while Escape leaves no authored objects.
#[test]
fn stale_or_cancelled_free_wire_drafts_never_commit() {
    let mut harness = harness();
    new_project(&mut harness);
    for stale in [false, true] {
        harness.key_press(Key::W);
        harness.run();
        click_canvas_offset(&mut harness, egui::vec2(-101.6, 0.0));
        click_canvas_offset(&mut harness, egui::vec2(0.0, 50.8));
        if stale {
            harness.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
        } else {
            harness.key_press(Key::Escape);
        }
        harness.run();
        let before = harness.state().session().inspect();
        harness.key_press(Key::Enter);
        harness.run();
        assert!(harness.state().views.active.tool.is_none());
        assert_eq!(harness.state().session().inspect(), before);
        assert_eq!(harness.state().session().project().unwrap().junctions().count(), 0);
    }
}

/// Each ribbon group's caption opens a menu of the same actions, as in PCBcraft Next: the
/// Panels menu toggles the Browser and the Connect menu starts the Wire tool, without
/// editing the project.
#[test]
fn ribbon_group_menus_run_their_actions() {
    let mut harness = harness();
    new_project(&mut harness);
    harness.get_by_label("Collapse Browser");

    click(&mut harness, "View");
    choose(&mut harness, "Panels menu", "Browser");
    assert!(harness.query_by_label("Collapse Browser").is_none());
    choose(&mut harness, "Panels menu", "Browser");
    harness.get_by_label("Collapse Browser");

    click(&mut harness, "Capture");
    choose(&mut harness, "Connect menu", "Wire");
    assert!(matches!(harness.state().views.active.tool, Some(Tool::Wire { .. })));
    assert_eq!(harness.state().session().revision(), 1);
}

/// Independently created projects have distinct UUIDs; compare the shared command results
/// while checking the sheet list apart from its generated identity and derived path.
fn assert_equivalent_sessions(actual: &Session, expected: &Session) {
    let mut a = actual.inspect();
    let mut b = expected.inspect();
    for state in [&mut a, &mut b] {
        for sheet in state["project"]["sheets"].as_array_mut().unwrap() {
            sheet["id"] = Value::Null;
            sheet["file"] = Value::Null;
        }
    }
    assert_eq!(a, b);
}

/// File and folder choices stay visible on narrow windows without opening an OS picker.
#[test]
fn project_location_and_save_choices_fit_a_narrow_window() {
    let mut harness = harness();
    harness.set_size(egui::vec2(320.0, 520.0));
    harness.run();
    harness.state_mut().dialog = Dialog::OpenLocation;
    harness.run();
    for label in ["Project file or archive…", "Project folder…", "Cancel"] {
        let rect = harness.get_by_label(label).rect();
        assert!(rect.left() >= 0.0 && rect.right() <= 320.0);
    }
    click(&mut harness, "Cancel");
    harness.state_mut().dialog = Dialog::SaveEncoding;
    harness.run();
    for label in ["Project folder…", "Packed project (.pc.zip)…", "Cancel"] {
        let rect = harness.get_by_label(label).rect();
        assert!(rect.left() >= 0.0 && rect.right() <= 320.0);
    }
    click(&mut harness, "Cancel");
    assert!(!harness.state().files.busy());
}
