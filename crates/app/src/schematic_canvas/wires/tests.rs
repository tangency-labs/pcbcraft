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
    h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect())
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
    assert_eq!(h.state().views.active.selection.single_wire(), Some("wire-4"));
    h.get_by_label("Break");
    h.get_by_label("Delete Segment");
    h.get_by_label("Delete Wire");
    let nets = h.state().session.project().unwrap().connectivity().nets.len();
    click(&mut h, "Slice");
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(3, 0)], vec![grid(3, 0), grid(8, 0), grid(8, 6)]]);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), nets);
    assert!(h.state().views.active.selection.is_empty());
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

/// Open both panes through the ribbon, with no floating browser over the test wire.
fn split_harness(narrow: bool) -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.state_mut().browser.open = false;
    h.state_mut().preferences.grid_opacity_percent = if narrow { 35 } else { 0 };
    if narrow {
        h.set_size(vec2(640.0, 800.0));
    }
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    h
}

/// Resolve the stable pane identity even after it has exchanged focus slots.
fn view<'a>(h: &'a Harness<'_, PcbApp>, id: u64) -> &'a crate::SchematicView {
    let views = &h.state().views;
    if views.active.id() == crate::ViewId(id) { &views.active } else { &views.other }
}

/// The actual canvas bounds for one visible pane.
fn canvas(h: &Harness<'_, PcbApp>, id: u64) -> egui::Rect {
    h.get_by_label(&format!("Schematic canvas {}", id + 1)).rect()
}

/// A first secondary press in an inactive pane must activate it and open its own wire menu.
fn right_click_in(h: &mut Harness<'_, PcbApp>, id: u64, point: Point) {
    let pos = view(h, id).camera.screen(point, canvas(h, id));
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Focus the tab at a stable pane position; both tabs show the same sheet path.
fn focus_tab(h: &mut Harness<'_, PcbApp>, id: u64) {
    h.get_all_by_label("Cuts › Main").nth(id as usize).unwrap().click();
    h.run();
}

/// Match the wire's rendered route and colour, excluding menu previews and other pane ink.
fn assert_painted(h: &Harness<'_, PcbApp>, id: u64, route: &[Point], color: egui::Color32) {
    let points: Vec<_> = route.iter().map(|point| view(h, id).camera.screen(*point, canvas(h, id))).collect();
    assert!(
        h.output().shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Path(path) => path.points == points && path.stroke.color == egui::epaint::ColorMode::Solid(color),
            _ => false,
        }),
        "wire route missing from pane {id}: {route:?}"
    );
}

/// Every menu action operates from either pane in both layouts, including the first click in
/// an inactive pane. Highlights are painted without granting edit selection, then both panes
/// repaint the shared result and use one undo history without changing the grid preference.
#[test]
fn split_wire_actions_share_history_and_keep_highlights_view_local() {
    let ink = pcbcraft_chrome::Tokens::get();
    for narrow in [false, true] {
        for source in [0, 1] {
            for (action, expected) in [
                ("Slice", vec![vec![grid(-8, 0), grid(8, 0), grid(8, 3)], vec![grid(8, 3), grid(8, 6)]]),
                ("Break", vec![vec![grid(-8, 0), grid(8, 0)], vec![grid(8, 0), grid(8, 6)]]),
                ("Delete Segment", vec![vec![grid(-8, 0), grid(8, 0)]]),
                ("Delete Wire", vec![]),
            ] {
                let mut h = split_harness(narrow);
                let before = h.state().session.inspect()["project"].clone();
                let revision = h.state().session.revision();
                let original = routes(&h)[0].clone();
                right_click_in(&mut h, source, grid(8, 3));
                assert_eq!(h.state().views.active.id(), crate::ViewId(source));
                assert_eq!(view(&h, source).selection.single_wire(), Some("wire-4"));
                assert!(view(&h, 1 - source).selection.is_empty());
                assert_painted(&h, source, &original, ink.accent);
                assert_painted(&h, 1 - source, &original, ink.schematic_highlight);
                h.hover_at(h.get_by_label(action).rect().center());
                h.run();
                let preview_color = if action.starts_with("Delete") { ink.error } else { ink.accent };
                for id in [0, 1] {
                    let rect = canvas(&h, id);
                    let preview = h.output().shapes.iter().any(|shape| match &shape.shape {
                        egui::Shape::Circle(circle) if action == "Slice" => {
                            circle.center == view(&h, id).camera.screen(grid(8, 3), rect) && circle.stroke.color == preview_color
                        }
                        egui::Shape::Path(path) if action != "Slice" => {
                            path.stroke.width == 2.0 * pcbcraft_chrome::theme::SCHEMATIC_SYMBOL_STROKE
                                && path.stroke.color == egui::epaint::ColorMode::Solid(preview_color)
                                && path.points.iter().all(|point| rect.contains(*point))
                        }
                        _ => false,
                    });
                    assert_eq!(preview, id == source, "{action} preview in pane {id}");
                }
                click(&mut h, action);
                assert_eq!(h.state().message, None, "{action}, pane {source}, narrow {narrow}");
                // Allocating wire/junction IDs can advance the revision by more than one.
                // The Undo assertion below checks that this was one history step.
                assert!(h.state().session.revision() > revision);
                assert_eq!(routes(&h), expected);
                for id in [0, 1] {
                    assert!(view(&h, id).selection.is_empty());
                    assert!(h.state().views.highlight_for(crate::ViewId(id), &h.state().session).is_empty());
                    for route in &expected {
                        assert_painted(&h, id, route, ink.schematic_wire);
                    }
                }
                let after = h.state().session.inspect()["project"].clone();
                focus_tab(&mut h, 1 - source);
                click(&mut h, "Undo");
                assert_eq!(h.state().session.inspect()["project"], before);
                for id in [0, 1] {
                    assert_painted(&h, id, &original, ink.schematic_wire);
                }
                click(&mut h, "Redo");
                assert_eq!(h.state().session.inspect()["project"], after);
                assert_eq!(h.state().preferences.grid_opacity_percent, if narrow { 35 } else { 0 });
            }
        }
    }
}

/// An edit arriving while either pane's menu is open is rejected at its original revision.
/// Reopening even the same snapshot discards the old popup; a fresh menu can edit normally.
#[test]
fn split_wire_menus_reject_intervening_edits_and_reopened_projects() {
    for source in [0, 1] {
        for reopen in [false, true] {
            let mut h = split_harness(false);
            right_click_in(&mut h, source, grid(8, 3));
            let snapshot = h.state().session.snapshot().unwrap();
            if reopen {
                h.state_mut().session.open_snapshot(snapshot);
                h.run();
                assert!(h.query_by_label("Delete Segment").is_none());
            } else {
                let revision = h.state().session.revision();
                h.state_mut()
                    .session
                    .apply(DRAW_WIRE, &json!({"from":grid(-8, 10),"to":grid(8, 10),"bends":[],"expected_revision":revision}))
                    .unwrap();
            }
            let before = h.state().session.inspect()["project"].clone();
            if !reopen {
                click(&mut h, "Delete Segment");
                assert!(h.state().message.as_deref().is_some_and(|message| message.contains("project changed")));
            }
            assert_eq!(h.state().session.inspect()["project"], before);
            right_click_in(&mut h, source, grid(8, 3));
            click(&mut h, "Delete Segment");
            assert_eq!(h.state().message, None);
            assert_eq!(routes(&h)[0], vec![grid(-8, 0), grid(8, 0)]);
        }
    }
}

/// A menu result captured before a focus round trip or split collapse cannot acquire a new
/// view's authority just because the wire and project revision still match.
#[test]
fn delayed_wire_actions_cannot_follow_focus_or_layout_changes() {
    for collapse in [false, true] {
        let mut h = split_harness(false);
        right_click_in(&mut h, 1, grid(8, 3));
        let source = h.state().views.active.input(&h.state().session).unwrap();
        let rect = canvas(&h, 1);
        let app = h.state();
        let camera = &app.views.active.camera;
        let leg = super::leg_at(app.session.project().unwrap(), camera, rect, camera.screen(grid(8, 3), rect), app.session.revision()).unwrap();
        h.key_press(egui::Key::Escape);
        h.run();
        if collapse {
            click(&mut h, "Split editor views");
        } else {
            focus_tab(&mut h, 0);
            focus_tab(&mut h, 1);
        }
        let before = h.state().session.inspect();
        h.state_mut().view_click(source, crate::Click::WireEdit(crate::schematic_canvas::WireAction::DeleteSegment, leg));
        assert_eq!(h.state().session.inspect(), before);
    }
}
