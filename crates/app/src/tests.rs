//! Drives the real window through its accessibility tree, with clicks and keys, no GPU needed.

use egui::{Key, Modifiers};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use super::*;

fn harness<'a>() -> Harness<'a, PcbApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), PcbApp::default());
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

fn select_symbol(harness: &mut Harness<'_, PcbApp>, name: &str) {
    harness.get_by_label(name).click();
    harness.run();
}

fn new_project(harness: &mut Harness<'_, PcbApp>, name: &str) {
    choose(harness, "File", "New Project…");
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(name);
    harness.run();
    harness.get_by_label("OK").click();
    harness.run();
}

#[test]
fn it_opens_with_no_project_and_says_so() {
    let harness = harness();
    harness.get_by_label("No project open. Choose File, then New Project.");
    assert!(harness.state().session().project().is_none());
}

/// The inspector never reserves space for the empty workspace, even after undoing project creation.
#[test]
fn the_empty_workspace_stays_centered_after_undo_with_the_inspector_open() {
    let mut harness = harness();
    let empty = "No project open. Choose File, then New Project.";
    let original = harness.get_by_label(empty).rect().center();
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_none());

    new_project(&mut harness, "blinky");
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    harness.get_by_label("Close");
    click(&mut harness, "Undo");

    assert!(harness.query_by_label("Close").is_none());
    assert_eq!(harness.get_by_label(empty).rect().center(), original);
}

#[test]
fn a_person_makes_a_project_through_the_menu() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
    assert!(harness.query_by_role(egui::accesskit::Role::TextInput).is_none());
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

    new_project(&mut harness, "blinky");
    choose(&mut harness, "File", "Save As…");
    harness.get_by_label("Plain JSON");
    harness.get_by_label("ZIP archive");
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

/// A dirty project keeps its marker when the title no longer fits beside the utilities.
#[test]
fn narrow_app_bar_keeps_the_unsaved_marker() {
    let mut harness = harness();
    let name = "Narrow schematic";
    new_project(&mut harness, name);
    harness.get_by_label("Unsaved changes");

    // The desktop minimum of 640 pixels leaves 320 logical points at 200% UI scale.
    let width = 320.0;
    harness.set_size(egui::vec2(width, 520.0));
    harness.run();
    let app_bar_bottom = harness.get_by_label("File").rect().bottom();
    assert!(harness.query_all_by_label(name).all(|node| node.rect().top() > app_bar_bottom));
    let marker = harness.get_by_label("Unsaved changes").rect();
    let preferences = harness.get_by_label("Preferences").rect();
    let about = harness.get_by_label("About PCBCraft").rect();
    assert!(marker.right() <= preferences.left(), "{marker:?} overlaps {preferences:?}");
    assert!(about.right() <= width, "{about:?} extends past the window");

    click(&mut harness, "Undo");
    assert!(harness.query_by_label("Unsaved changes").is_none());
}

/// Undo and Redo from the app bar produce the same project state as the engine commands.
#[test]
fn app_bar_actions_reach_the_same_state_as_commands_an_agent_runs() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    click(&mut harness, "Undo");
    click(&mut harness, "Redo");
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "blinky"})).unwrap();
    session.execute("edit.undo", &Value::Null).unwrap();
    session.execute("edit.redo", &Value::Null).unwrap();
    assert_eq!(harness.state().session().inspect(), session.inspect());
}

#[test]
fn a_bad_name_stays_in_the_dialog_with_the_reason() {
    let mut harness = harness();
    choose(&mut harness, "File", "New Project…");
    harness.get_by_label("OK").click();
    harness.run();
    harness.get_by_label("the name can't be empty");
    assert!(harness.state().session().project().is_none());
}

#[test]
fn cancelling_changes_nothing() {
    let mut harness = harness();
    choose(&mut harness, "File", "New Project…");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().session().revision(), 0);
    assert!(harness.query_by_label("Cancel").is_none());
}

#[test]
fn rename_starts_from_the_current_name_and_can_be_undone() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    choose(&mut harness, "File", "Rename Project…");
    assert_eq!(harness.get_by_role(egui::accesskit::Role::TextInput).value().as_deref(), Some("blinky"));
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(" v2");
    harness.run();
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky v2");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
}

#[test]
fn undo_and_redo_keys_match_kicad_and_do_nothing_when_there_is_nothing_to_do() {
    let mut harness = harness();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().revision(), 0);
    new_project(&mut harness, "blinky");
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
    new_project(&mut harness, "blinky");
    choose(&mut harness, "File", "Rename Project…");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().session().project().unwrap().name, "blinky");
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
    harness.get_by_label("Close").click();
    harness.run();
    click(&mut harness, "About PCBCraft");
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_none());
}

/// Capture placement snaps through the shared engine command and can be undone from the app bar.
#[test]
fn canvas_placement_uses_the_shared_command_and_snaps_to_the_grid() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    click(&mut harness, "Add symbol (A)");
    select_symbol(&mut harness, "Resistor");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    harness.get_by_label("Place Resistor · Esc to cancel");

    // A click near the center snaps to the schematic origin.
    click_canvas(&mut harness);
    let mut session = Session::default();
    session.execute("project.new", &json!({"name": "blinky"})).unwrap();
    session.execute(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": 1})).unwrap();
    assert_eq!(harness.state().session().inspect(), session.inspect());
    click_canvas(&mut harness);
    assert_eq!(harness.state().session().inspect(), session.inspect());
    click(&mut harness, "Undo");
    assert!(harness.state().session().project().unwrap().symbols().is_empty());
    click(&mut harness, "Redo");
    assert_eq!(harness.state().session().project().unwrap().symbols(), session.project().unwrap().symbols());
}

#[test]
fn escape_and_opening_a_dialog_cancel_the_placement_preview() {
    let mut harness = harness();
    new_project(&mut harness, "blinky");
    harness.key_press(Key::A);
    harness.run();
    harness.get_by_label("LED");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("LED").is_none());
    click_canvas(&mut harness);
    assert!(harness.state().session().project().unwrap().symbols().is_empty());

    harness.key_press(Key::A);
    harness.run();
    select_symbol(&mut harness, "Resistor");
    harness.get_by_label("Place Resistor · Esc to cancel");
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
    new_project(&mut harness, "navigation");
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
    new_project(&mut harness, "empty");
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

/// Place a resistor and LED through the canvas so their exact terminal positions are known.
fn wiring_project(harness: &mut Harness<'_, PcbApp>) {
    new_project(harness, "wiring");
    click(harness, "Add symbol (A)");
    select_symbol(harness, "Resistor");
    click_canvas(harness);
    click(harness, "Add symbol (A)");
    select_symbol(harness, "LED");
    harness.get_by_label("Place LED · Esc to cancel");
    click_canvas_offset(harness, egui::vec2(203.2, 60.96));
    assert_eq!(harness.state().session().project().unwrap().symbols()[1].definition.id, "led");
}

/// Capture wiring produces a shared connection that the app bar can undo and redo.
#[test]
fn the_wire_tool_runs_the_shared_command_and_is_undoable() {
    let mut harness = harness();
    wiring_project(&mut harness);
    click(&mut harness, "Draw wire (W)");
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    harness.get_by_label("Wire · Click the destination pin");
    assert_eq!(harness.state().session().revision(), 3);
    click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));

    let inspected = harness.state().session().inspect();
    let schematic = &inspected["project"]["schematic"];
    assert_eq!(
        schematic["wires"],
        json!([{
            "id": "wire-4",
            "from": {"symbol": "symbol-2", "terminal": "2"},
            "to": {"symbol": "symbol-3", "terminal": "1"},
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
    new_project(&mut harness, "compact");

    for (name, x) in [("Resistor", -81.28), ("LED", 81.28)] {
        harness.get_by_label("Add symbol (A)").click();
        harness.run();
        select_symbol(&mut harness, name);
        click_canvas_offset(&mut harness, egui::vec2(x, 0.0));
    }
    harness.get_by_label("Draw wire (W)").click();
    harness.run();
    for x in [-30.48, 30.48] {
        click_canvas_offset(&mut harness, egui::vec2(x, 0.0));
    }
    let wired = harness.state().session().inspect();
    assert_eq!(wired["project"]["schematic"]["wires"].as_array().unwrap().len(), 1);

    let full_width = harness.get_by_label("Schematic canvas").rect().width();
    harness.get_by_label("Toggle Net Inspector").click();
    harness.run();
    harness.get_by_label("R1.2, D1.1");
    let canvas = harness.get_by_label("Schematic canvas").rect();
    let close = harness.get_by_label("Close");
    assert!(canvas.width() >= 0.6 * full_width - 1.0, "canvas: {canvas:?}");
    assert!(close.rect().left() >= canvas.right());
    close.click();
    harness.run();
    assert!(harness.query_by_label("R1.2, D1.1").is_none());
    assert_eq!(harness.state().session().inspect(), wired);
}

#[test]
fn wiring_ignores_empty_space_and_escape_discards_the_preview() {
    let mut harness = harness();
    wiring_project(&mut harness);
    let before = harness.state().session().inspect();
    harness.key_press(Key::W);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    click_canvas_offset(&mut harness, egui::vec2(100.0, 36.0));
    harness.get_by_label("Wire · Click the destination pin");
    harness.key_press(Key::Escape);
    harness.run();
    click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
    assert_eq!(harness.state().session().inspect(), before);
}

/// Open the context menu during an active wire draft with a real secondary click.
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
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    open_wire_context(&mut harness);
    let destination = canvas_center(&harness) + egui::vec2(152.4, 60.96);
    click_canvas_at(&mut harness, destination);
    assert!(harness.query_by_label("Cancel").is_none());
    assert!(harness.state().session().project().unwrap().wires().is_empty());
    harness.get_by_label("Wire · Click the destination pin");
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
        click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
        open_wire_context(&mut harness);
        if escape {
            harness.key_press(Key::Escape);
        } else {
            harness.get_by_label("Cancel").click();
        }
        harness.run();
        assert!(harness.query_by_label("Cancel").is_none());
        assert!(harness.query_by_label("Wire · Click the destination pin").is_none());
        click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
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
    click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
    click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
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
        click_canvas_offset(&mut harness, egui::vec2(50.8, 0.0));
        click_canvas_offset(&mut harness, egui::vec2(152.4, 60.96));
        let before_wire = painted_wire(&harness);

        choose(&mut harness, "File", "Local MCP…");
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

        choose(&mut harness, "File", "Local MCP…");
        click(&mut harness, "Disable connection");
        assert!(harness.state().live_control.is_none());
        let disconnected = tool(&mut harness, connection, "inspect_project", json!({}));
        assert_eq!(disconnected["isError"], true);
    }
}
