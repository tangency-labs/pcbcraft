use crate::{PcbApp, tests::click};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    DRAW_WIRE,
    schematic::{GRID_NM, Point},
};
use serde_json::{Value, json};

/// A model point on the schematic grid.
fn grid(x: i64, y: i64) -> Point {
    Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
}

/// A project with one bent free wire, (-8, 0) → (8, 0) → (8, 6), drawn through the shared command.
fn harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({"name":"Cuts"})).unwrap();
    app.session
        .apply(DRAW_WIRE, &json!({"from":grid(-8, 0),"to":grid(8, 6),"bends":[grid(8, 0)],"expected_revision":app.session.revision()}))
        .unwrap();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(20).build_ui_state(|ui, app| app.frame(ui), app);
    h.run();
    h
}

/// The screen position of a model point through the actual canvas transform.
fn screen(h: &Harness<'_, PcbApp>, point: Point) -> Pos2 {
    h.state().camera.screen(point, h.get_by_label("Schematic canvas").rect())
}

/// Press and release the secondary button `offset` points from a model point, opening the
/// context menu there.
fn right_click(h: &mut Harness<'_, PcbApp>, point: Point, offset: egui::Vec2) {
    let pos = screen(h, point) + offset;
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Every wire's drawn route.
fn routes(h: &Harness<'_, PcbApp>) -> Vec<Vec<Point>> {
    let project = h.state().session.project().unwrap();
    project.wires().iter().map(|wire| project.wire_path(wire)).collect()
}

/// Right-clicking a wire selects it and offers Slice, Break, Delete Segment and Delete Wire.
/// Slice splits it at the grid point under the click, keeping one net, and undoes as one step.
#[test]
fn the_wire_menu_slices_at_the_clicked_grid_point() {
    let mut h = harness();
    let before = routes(&h);
    // A little off the wire and between grid points still targets (3, 0).
    right_click(&mut h, grid(3, 0), vec2(2.0, 2.0));
    assert_eq!(h.state().selection.single_wire(), Some("wire-4"));
    h.get_by_label("Break");
    h.get_by_label("Delete Segment");
    h.get_by_label("Delete Wire");
    let nets = h.state().session.project().unwrap().connectivity().nets.len();
    click(&mut h, "Slice");
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(3, 0)], vec![grid(3, 0), grid(8, 0), grid(8, 6)]]);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), nets);
    assert!(h.state().selection.is_empty());
    click(&mut h, "Undo");
    assert_eq!(routes(&h), before);
}

/// Delete Segment removes only the leg under the click, and Delete Wire the whole route.
#[test]
fn the_wire_menu_deletes_a_segment_or_the_whole_wire() {
    let mut h = harness();
    right_click(&mut h, grid(8, 2), egui::Vec2::ZERO);
    click(&mut h, "Delete Segment");
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(8, 0)]]);
    right_click(&mut h, grid(-2, 0), egui::Vec2::ZERO);
    click(&mut h, "Delete Wire");
    assert!(routes(&h).is_empty());
    assert_eq!(h.state().session.project().unwrap().junctions().count(), 0);
}

/// Slice is unavailable at either end of a route, where there is nothing to split.
#[test]
fn slice_is_disabled_at_the_ends_of_a_route() {
    let mut h = harness();
    right_click(&mut h, grid(-8, 0), vec2(1.0, 0.0));
    assert!(h.get_by_label("Slice").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Delete Segment").accesskit_node().is_disabled());
}

/// A change made while the menu is open, such as an agent's edit, makes the row fail visibly
/// instead of cutting a wire the person no longer sees as it was.
#[test]
fn a_stale_wire_menu_edits_nothing() {
    let mut h = harness();
    right_click(&mut h, grid(2, 0), egui::Vec2::ZERO);
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE, &json!({"from":grid(-8, 10),"to":grid(8, 10),"bends":[],"expected_revision":revision})).unwrap();
    let before: Value = h.state().session.inspect()["project"].clone();
    click(&mut h, "Delete Segment");
    assert!(h.state().message.as_deref().is_some_and(|message| message.contains("project changed")), "{:?}", h.state().message);
    assert_eq!(h.state().session.inspect()["project"], before);
}

/// Break separates the leg under the click from its neighbour: the drawing is unchanged, but
/// the corner becomes two unconnected ends. A one-leg wire has nothing to break from.
#[test]
fn the_wire_menu_breaks_a_leg_from_its_neighbour() {
    let mut h = harness();
    let before = routes(&h);
    right_click(&mut h, grid(8, 3), egui::Vec2::ZERO);
    click(&mut h, "Break");
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![before[0][..=1].to_vec(), before[0][1..].to_vec()]);
    let project = h.state().session.project().unwrap();
    assert_ne!(project.wires()[0].endpoints()[1], project.wires()[1].endpoints()[0]);

    right_click(&mut h, grid(8, 3), egui::Vec2::ZERO);
    assert!(h.get_by_label("Break").accesskit_node().is_disabled());
}
