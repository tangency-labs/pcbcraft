//! Grid snapping is view state: turned off, placement and drags land at the pointer's exact
//! position in that view, and the project, its revision, unsaved state and history stay as they are.

use pcbcraft_chrome::grid::NM_PER_MIL;
use pcbcraft_engine::{
    persistence::encode_documents,
    schematic::{GRID_NM, Point},
};

use super::*;

const MIL: i64 = NM_PER_MIL;

/// The status bar's grid button, whatever spacing and snapping it shows. There is exactly one.
fn grid_button<'a>(h: &'a Harness<'_, PcbApp>) -> egui_kittest::Node<'a> {
    h.get_by(|node| node.label().is_some_and(|label| label.starts_with("Grid ")))
}

/// Flip snapping in the focused view through the status bar's grid menu, as a person does.
fn toggle_snap(h: &mut Harness<'_, PcbApp>) {
    grid_button(h).click();
    h.run();
    click(h, "Snap to grid");
}

/// What a snap change must leave alone: revision, unsaved state, both history stacks and the
/// saved bytes.
fn project_state(h: &Harness<'_, PcbApp>) -> (u64, bool, bool, bool, pcbcraft_engine::persistence::Documents) {
    let app = h.state();
    let bytes = encode_documents(app.session.snapshot().unwrap()).unwrap();
    (app.session.revision(), app.files.dirty(&app.session), app.session.available("edit.undo"), app.session.available("edit.redo"), bytes)
}

/// The model point under a canvas position with no grid applied.
fn exact(h: &Harness<'_, PcbApp>, pointer: egui::Pos2) -> Point {
    h.state().views.active.camera.unsnapped(pointer, h.get_by_label("Schematic canvas").rect())
}

/// The status-bar menu turns snapping off and on for the focused view and remembers the choice
/// for schematic views. Toggling leaves the revision, unsaved state, Undo and Redo and the saved
/// bytes unchanged. With snapping off a placed part lands at the pointer; on again, it snaps.
#[test]
fn the_grid_menu_turns_snapping_off_without_touching_the_project() {
    let mut h = harness();
    new_project(&mut h);
    // Give both history stacks something to keep: two placements, one undone.
    for x_nm in [0, 20 * GRID_NM] {
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x_nm,"y_nm":20 * GRID_NM,"expected_revision":revision}));
    }
    click(&mut h, "Undo");
    let before = project_state(&h);
    assert!(before.2 && before.3, "undo and redo are both available");
    assert!(h.state().views.active.camera.snap_enabled());

    toggle_snap(&mut h);
    h.get_by_label("Grid 50 mil · snap off");
    assert!(!h.state().views.active.camera.snap_enabled());
    assert!(!h.state().preferences.schematic_grid_snap);
    assert!(h.state().preferences.board_grid_snap, "a schematic choice keeps the board preference");
    assert_eq!(h.state().views.active.camera.snap_nm(), 50 * MIL, "the spacing stays for later");
    assert_eq!(project_state(&h), before);
    toggle_snap(&mut h);
    h.get_by_label("Grid 50 mil");
    assert!(h.state().views.active.camera.snap_enabled() && h.state().preferences.schematic_grid_snap);
    assert_eq!(project_state(&h), before);
    toggle_snap(&mut h);
    assert_eq!(project_state(&h), before);

    // At the default zoom 50 mil is 10.16 points: this pointer is well off the grid.
    let offset = egui::vec2(3.0 * 5.08 + 1.0, -2.0);
    click(&mut h, "Add symbol (A)");
    select_symbol(&mut h, "Resistor");
    let pointer = canvas_center(&h) + offset;
    let expected = exact(&h, pointer);
    assert!(expected.x_nm % GRID_NM != 0 && expected.y_nm % GRID_NM != 0);
    click_canvas_at(&mut h, pointer);
    let placed = |h: &Harness<'_, PcbApp>| h.state().session.project().unwrap().symbols().last().unwrap().position;
    assert_eq!(placed(&h), expected);
    h.key_press(Key::Escape);
    h.run();
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Undo");

    toggle_snap(&mut h);
    click(&mut h, "Add symbol (A)");
    select_symbol(&mut h, "Resistor");
    click_canvas_at(&mut h, pointer);
    assert_eq!(placed(&h), Point { x_nm: 2 * GRID_NM, y_nm: 0 }, "snapping on is unchanged");
}

/// Dragging a part with snapping off moves it by exactly the pointer's travel, off the grid, and
/// the wires attached to its pins follow: connectivity is the same as before and Undo restores it.
#[test]
fn snap_off_moves_symbols_by_the_exact_drag() {
    let mut h = harness();
    wiring_project(&mut h);
    click(&mut h, "Draw wire (W)");
    click_pin(&mut h, 0, 1);
    click_pin(&mut h, 1, 0);
    h.key_press(Key::Escape);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_view_snap(id, false);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let original = before.symbols()[0].position;
    let start = canvas_center(&h);
    let end = start + egui::vec2(3.0, 37.0);
    let delta = (exact(&h, end).x_nm - exact(&h, start).x_nm, exact(&h, end).y_nm - exact(&h, start).y_nm);
    h.drag_at(start);
    h.run();
    h.hover_at(end);
    h.run();
    h.drop_at(end);
    h.run();
    let moved = h.state().session.project().unwrap();
    let position = moved.symbols()[0].position;
    assert_eq!(position, Point { x_nm: original.x_nm + delta.0, y_nm: original.y_nm + delta.1 });
    assert!(position.x_nm % GRID_NM != 0 && position.y_nm % GRID_NM != 0, "{position:?}");
    assert!(before.same_connectivity(moved));
    click(&mut h, "Undo");
    assert_eq!(h.state().session.project().unwrap().symbols(), before.symbols());
}

/// Two views of one sheet hold their own snapping through focus changes; a split seeds the new
/// view from the focused one, and reopening the document keeps each choice. Editor inspection
/// reports every view's setting and `view.set_grid` sets it the way the status bar does.
#[test]
fn each_view_keeps_its_own_snapping_and_inspection_reads_and_sets_it() {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    new_project(&mut h);
    h.state_mut().browser.open = false;
    toggle_snap(&mut h);
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let snap = |h: &Harness<'_, PcbApp>| [0, 1].map(|id| h.state().views.view(ViewId(id)).unwrap().camera.snap_enabled());
    assert_eq!(snap(&h), [false, false], "the split starts from the focused view");

    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    h.get_all_by_label(&path).nth(1).unwrap().click();
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(1));
    toggle_snap(&mut h);
    assert_eq!(snap(&h), [false, true]);
    h.get_by_label("Grid 50 mil");
    h.get_all_by_label(&path).next().unwrap().click();
    h.run();
    assert_eq!(h.state().views.active.id(), ViewId(0));
    h.get_by_label("Grid 50 mil · snap off");
    assert_eq!(snap(&h), [false, true]);

    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert_eq!(snap(&h), [false, true], "rebinding the views keeps each setting");
    let before = project_state(&h);

    let inspected = |h: &Harness<'_, PcbApp>| {
        let state = crate::window_commands::inspect_editor(h.state());
        [0, 1].map(|id| {
            let view = state["views"].as_array().unwrap().iter().find(|view| view["id"] == id).unwrap();
            (view["grid_snap"].as_bool().unwrap(), view["grid_spacing_nm"].as_i64().unwrap())
        })
    };
    assert_eq!(inspected(&h), [(false, 50 * MIL), (true, 50 * MIL)]);
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "view.set_grid", &json!({"view": 1, "snap": false})).unwrap();
    assert_eq!(inspected(&h), [(false, 50 * MIL), (false, 50 * MIL)]);
    h.state_mut().run_window_command(&ctx, "view.set_grid", &json!({"view": 0, "snap": true, "spacing_nm": 25 * MIL})).unwrap();
    assert_eq!(inspected(&h), [(true, 25 * MIL), (false, 50 * MIL)]);
    assert!(h.state_mut().run_window_command(&ctx, "view.set_grid", &json!({"view": 1})).is_err(), "nothing to set");
    assert!(crate::window_commands::check_command("view.set_grid", &json!({"view": 1, "snap": "off"})).is_err());
    assert_eq!(inspected(&h), [(true, 25 * MIL), (false, 50 * MIL)]);
    assert_eq!(project_state(&h), before);
}

/// Collapsing a split and opening it again restores the parked pane with its own snapping,
/// while its navigation and spacing follow the focused pane as before.
#[test]
fn reopening_a_split_keeps_the_restored_panes_snapping() {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    new_project(&mut h);
    h.state_mut().browser.open = false;
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    h.state_mut().set_view_snap(ViewId(0), false);
    h.state_mut().set_view_grid(ViewId(0), 25 * MIL);
    assert_eq!(h.state().views.active.id(), ViewId(0));
    let grid = |h: &Harness<'_, PcbApp>| [0, 1].map(|id| h.state().views.view(ViewId(id)).map(|v| (v.camera.snap_enabled(), v.camera.snap_nm())));
    assert_eq!(grid(&h), [Some((false, 25 * MIL)), Some((true, 50 * MIL))]);
    click(&mut h, "Split editor views");
    click(&mut h, "Split editor views");
    assert!(h.state().views.is_split());
    assert_eq!(grid(&h), [Some((false, 25 * MIL)), Some((true, 25 * MIL))], "pane 1 keeps its snapping; spacing follows navigation");
}

/// At startup each workspace takes its own remembered snapping, as it does its spacing, and a
/// view switching between schematic and PCB keeps one setting for each.
#[test]
fn startup_seeds_snapping_per_workspace() {
    let preferences = Preferences::parse("{\"schematic_grid_snap\":false}");
    let mut views = EditorViews::default();
    views.set_snap_defaults(preferences.schematic_grid_snap, preferences.board_grid_snap);
    assert!(!views.active.camera.snap_enabled());
    views.active.workspace = Workspace::Board;
    views.set_snap_defaults(preferences.schematic_grid_snap, preferences.board_grid_snap);
    assert!(views.active.camera.snap_enabled());

    let mut h = super::board_views::board_harness();
    let id = h.state().views.active.id();
    h.state_mut().set_view_snap(id, false);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB Layout");
    assert!(h.state().views.active.camera.snap_enabled(), "the PCB editor has its own setting");
    assert!(h.state().preferences.board_grid_snap && !h.state().preferences.schematic_grid_snap);
    click(&mut h, "Choose workspace");
    click(&mut h, "Schematic");
    assert!(!h.state().views.active.camera.snap_enabled());
}
