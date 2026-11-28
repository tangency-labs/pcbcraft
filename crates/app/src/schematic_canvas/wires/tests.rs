mod attachment;
mod detachment;
mod healing;
mod joining;
mod redraws;
mod segments;
mod snapping;
mod targets;

use crate::{PcbApp, tests::click};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    CONNECT_PINS, DRAW_WIRE, PLACE_RESISTOR,
    schematic::{GRID_NM, Point},
};
use serde_json::{Value, json};
use std::rc::Rc;

/// A model point on the schematic grid.
fn grid(x: i64, y: i64) -> Point {
    Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
}

/// A project whose wire is styled with the active theme accent at the maximum drawn width through
/// the shared net appearance command: the pathological case where a net color equals the selection
/// ink and the ink is already the widest stroke.
fn styled_harness() -> Harness<'static, PcbApp> {
    styled_harness_with(pcbcraft_chrome::Tokens::get().accent)
}

/// A single-wire project whose net is styled with `color` at the maximum drawn width.
fn styled_harness_with(color: egui::Color32) -> Harness<'static, PcbApp> {
    styled_harness_with_paper(color, |_| {})
}

/// As [`styled_harness_with`], but applies `paper` to personal preferences before the first frame
/// so a test can paint the saved colour over Uniform, Shaded or Custom paper.
fn styled_harness_with_paper(color: egui::Color32, paper: impl Fn(&mut crate::preferences::Preferences)) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Styled"}));
    app.parts.open = false;
    app.browser.open = false;
    for x in [0, 25_400_000] {
        app.run(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":app.session.revision()}));
    }
    let pins: Vec<Value> = app
        .session
        .project()
        .unwrap()
        .symbols()
        .iter()
        .zip(["2", "1"])
        .map(|(symbol, terminal)| json!({"symbol":symbol.id(),"terminal":terminal}))
        .collect();
    app.run(CONNECT_PINS, &json!({"from":pins[0],"to":pins[1],"expected_revision":app.session.revision()}));

    let expected =
        pcbcraft_engine::nets::Appearance { color: Some([color.r(), color.g(), color.b()]), width_cp: Some(pcbcraft_engine::nets::MAX_WIDTH_CP) };
    let project = app.session.project().unwrap();
    let wire = project.wires()[0].id();
    let sheet = project.sheet_target();
    let target = project.net_class_target_for_endpoint(sheet, project.wires()[0].endpoints()[0]).unwrap().unwrap();
    app.run(
        pcbcraft_engine::nets::SET_APPEARANCE,
        &json!({
            "target": serde_json::to_value(&target).unwrap(),
            "appearance": {"color":expected.color.unwrap(), "width_cp":expected.width_cp.unwrap()},
            "expected_revision": app.session.revision(),
        }),
    );
    assert_eq!(app.session.project().unwrap().wire_appearances().unwrap().get(&wire), Some(&expected));

    paper(&mut app.preferences);
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(|ui, app| app.frame(ui), app);
    h.run();
    h
}

/// A project with one bent free wire, (-8, 0) → (8, 0) → (8, 6), drawn through the shared command.
fn harness() -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    for view in app.views.all_mut() {
        view.filters_open = false;
    }
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

/// Whether a polyline with exactly `points`, `width` and `color` was painted this frame.
fn painted_stroke(h: &Harness<'_, PcbApp>, points: &[Pos2], width: f32, color: egui::Color32) -> bool {
    h.output().shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Path(path) => {
            path.points.as_slice() == points && path.stroke.width == width && path.stroke.color == egui::epaint::ColorMode::Solid(color)
        }
        _ => false,
    })
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
    assert_eq!(h.state().views.active.schematic.selection.single_wire(), Some("wire-4"));
    h.get_by_label("Break");
    h.get_by_label("Delete Segment");
    h.get_by_label("Delete Wire");
    let nets = h.state().session.project().unwrap().connectivity().nets.len();
    click(&mut h, "Slice");
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(3, 0)], vec![grid(3, 0), grid(8, 0), grid(8, 6)]]);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), nets);
    assert!(h.state().views.active.schematic.selection.is_empty());
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
    h.get_by_label("Split editor views").scroll_to_me();
    h.run();
    click(&mut h, "Split editor views");
    h
}

/// Resolve the stable pane identity even after it has exchanged focus slots.
fn view<'a>(h: &'a Harness<'_, PcbApp>, id: u64) -> &'a crate::EditorView {
    let views = &h.state().views;
    views.view(crate::ViewId(id)).unwrap()
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

/// Match the wire's rendered route, colour and screen-space stroke width in one pane.
fn assert_painted_width(h: &Harness<'_, PcbApp>, id: u64, route: &[Point], color: egui::Color32, width: f32) {
    let points: Vec<_> = route.iter().map(|point| view(h, id).camera.screen(*point, canvas(h, id))).collect();
    assert!(
        h.output().shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Path(path) =>
                path.points == points && path.stroke.color == egui::epaint::ColorMode::Solid(color) && path.stroke.width == width,
            _ => false,
        }),
        "wire route missing from pane {id} at width {width}: {route:?}"
    );
}

/// A counterpart highlight gets the same contrasting underlay as a local selection. The ink keeps
/// the highlight colour while the other pane keeps its accent-selected ink, so local selection
/// still decides the colour when both states could apply to the same wire.
#[test]
fn highlight_paints_the_contrasting_cue_with_its_own_ink() {
    let mut h = split_harness(false);
    h.run();
    let route = routes(&h)[0].clone();
    right_click_in(&mut h, 0, grid(8, 3));
    let tokens = pcbcraft_chrome::Tokens::get();
    let [(halo_width, halo_color), (ink_width, ink_color)] =
        super::emphasis_strokes(tokens.schematic_highlight, pcbcraft_chrome::theme::SCHEMATIC_SYMBOL_STROKE);
    assert_painted_width(&h, 1, &route, ink_color, ink_width);
    assert_painted_width(&h, 1, &route, halo_color, halo_width);
    assert_painted(&h, 0, &route[1..], tokens.accent);
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
                assert_eq!(view(&h, source).schematic.selection.single_wire(), Some("wire-4"));
                assert!(view(&h, 1 - source).schematic.selection.is_empty());
                assert_painted(&h, source, &original, ink.schematic_wire);
                assert_painted(&h, source, &original[1..], ink.accent);
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
                        egui::Shape::Path(path)
                            if action != "Slice"
                                && path.stroke.width == 2.0 * pcbcraft_chrome::theme::SCHEMATIC_SYMBOL_STROKE
                                && path.stroke.color == egui::epaint::ColorMode::Solid(preview_color) =>
                        {
                            path.points.iter().all(|point| rect.contains(*point))
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
                    assert!(view(&h, id).schematic.selection.is_empty());
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

/// Select the test wire as a click would, so its handles are live.
fn select_wire(h: &mut Harness<'_, PcbApp>) {
    h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire("wire-4".to_owned()));
    h.run();
}

/// Drag with the primary button from one model point to another in small steps.
fn drag(h: &mut Harness<'_, PcbApp>, from: Point, to: Pos2) {
    let start = screen(h, from);
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.run();
    for step in 1..=4 {
        h.hover_at(start + (to - start) * step as f32 / 4.0);
        h.run();
    }
}

/// Finish a drag at `to`.
fn drop(h: &mut Harness<'_, PcbApp>, to: Pos2) {
    h.drop_at(to);
    h.run();
}

/// Dragging a leg of the selected wire slides it across its direction only, through the shared
/// command, as one undo step; the wire stays selected and keeps its identity and ends.
#[test]
fn dragging_a_selected_segment_slides_it() {
    let mut h = harness();
    select_wire(&mut h);
    h.get_by_label("Wire selected · Drag a segment, corner or free end to reshape · Del delete");
    let revision = h.state().session.revision();
    let ends = h.state().session.project().unwrap().wires()[0].endpoints();
    let to = screen(&h, grid(11, 4));
    drag(&mut h, grid(8, 3), to);
    h.get_by_label("Reshape wire · Esc to cancel");
    assert_eq!(h.state().session.revision(), revision, "a drag is only a preview");
    drop(&mut h, to);
    assert_eq!(h.state().message, None);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(11, 0), grid(11, 6), grid(8, 6)]]);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints(), ends);
    assert_eq!(h.state().views.active.schematic.selection.single_wire(), Some("wire-4"));
    click(&mut h, "Undo");
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(8, 0), grid(8, 6)]]);
}

/// A corner drags both of its legs; a free end drags its end leg and leaves the far end put.
#[test]
fn dragging_a_corner_or_a_free_end_reshapes_the_route() {
    let mut h = harness();
    select_wire(&mut h);
    let to = screen(&h, grid(4, 3));
    drag(&mut h, grid(8, 0), to);
    drop(&mut h, to);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(-8, 3), grid(4, 3), grid(4, 6), grid(8, 6)]]);
    click(&mut h, "Undo");

    let to = screen(&h, grid(10, 9));
    drag(&mut h, grid(8, 6), to);
    drop(&mut h, to);
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(10, 0), grid(10, 9)]]);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.endpoint_position(project.wires()[0].endpoints()[1]), Some(grid(10, 9)));
}

/// Escape, a release over a card, or an edit made during the drag (by an agent, say) leaves the
/// route as it was.
#[test]
fn cancelled_or_stale_reshapes_change_nothing() {
    let mut h = harness();
    select_wire(&mut h);
    let before = h.state().session.inspect();
    let to = screen(&h, grid(11, 3));
    drag(&mut h, grid(8, 3), to);
    h.key_press(egui::Key::Escape);
    h.run();
    drop(&mut h, to);
    assert_eq!(h.state().session.inspect(), before);

    // The ribbon is outside the canvas.
    select_wire(&mut h);
    drag(&mut h, grid(8, 3), to);
    let ribbon = h.get_by_label("Capture").rect().center();
    h.hover_at(ribbon);
    h.run();
    drop(&mut h, ribbon);
    assert_eq!(h.state().session.inspect(), before);

    select_wire(&mut h);
    drag(&mut h, grid(8, 3), to);
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE, &json!({"from":grid(-8, 10),"to":grid(8, 10),"bends":[],"expected_revision":revision})).unwrap();
    h.run();
    drop(&mut h, to);
    assert_eq!(routes(&h)[0], vec![grid(-8, 0), grid(8, 0), grid(8, 6)]);
}

/// Whole-wire translation is an explicit choice; an ordinary drag now reshapes one segment.
#[test]
fn dragging_an_explicitly_selected_wire_moves_it_whole() {
    let mut h = harness();
    right_click(&mut h, grid(8, 3), egui::Vec2::ZERO);
    click(&mut h, "Select Wire");
    let to = screen(&h, grid(8, 5));
    drag(&mut h, grid(8, 3), to);
    drop(&mut h, to);
    assert_eq!(routes(&h), vec![vec![grid(-8, 2), grid(8, 2), grid(8, 8)]]);
}

/// The paint cache reuses one derivation for the same committed snapshot and revision, sharing
/// the reference-counted map instead of copying its entries. It never stores a preview snapshot
/// and recomputes once the document revision moves on.
#[test]
fn appearance_cache_reuses_committed_snapshot_and_bypasses_previews() {
    let h = harness();
    let project = h.state().session.project().unwrap().clone();
    let revision = h.state().session.revision();
    let draft = super::Draft::default();

    let first = draft.appearances(&project, revision, false);
    assert!(draft.appearance_cache.borrow().snapshot.is_some());
    let second = draft.appearances(&project, revision, false);
    assert_eq!(second, first);
    assert!(Rc::ptr_eq(&first, &second), "a cache hit must share the map allocation, not clone it");

    let cached_revision = draft.appearance_cache.borrow().revision;
    let cached_wires = draft.appearance_cache.borrow().snapshot.as_ref().map(|snapshot| snapshot.wires().as_ptr());
    // A preview at another revision is derived fresh and must not replace the committed cache.
    let preview = draft.appearances(&project, revision + 1, true);
    assert!(!Rc::ptr_eq(&preview, &first), "a different revision cannot reuse the cached map");
    assert_eq!(draft.appearance_cache.borrow().revision, cached_revision);
    assert_eq!(draft.appearance_cache.borrow().snapshot.as_ref().map(|snapshot| snapshot.wires().as_ptr()), cached_wires);

    let recomputed = draft.appearances(&project, revision + 1, false);
    assert_eq!(recomputed, first);
    assert_eq!(draft.appearance_cache.borrow().revision, revision + 1);
}

/// Headless stand-in for the camera-movement and drag-preview replay: repeated paints at one
/// revision share one map, a reshape preview borrows it when endpoints stay identical, and a
/// changed revision derives a fresh map. No preview snapshot is ever stored in the cache.
#[test]
fn appearance_replay_shares_one_map_for_camera_and_drag_previews() {
    let h = harness();
    let project = h.state().session.project().unwrap().clone();
    let revision = h.state().session.revision();
    let draft = super::Draft::default();
    let committed = draft.appearances(&project, revision, false);
    assert_eq!(Rc::strong_count(&committed), 2, "one local handle and one cache handle");

    // Simulated camera-only repaints: same snapshot and revision, no new derivation.
    for _ in 0..8 {
        assert!(Rc::ptr_eq(&draft.appearances(&project, revision, false), &committed));
    }
    assert_eq!(Rc::strong_count(&committed), 2);

    // A reshape drag rebuilds the wire collection but keeps every endpoint identity, so the
    // net-wide appearance still comes from the committed map.
    let edit = pcbcraft_engine::wire::Reshape::Segment { wire: "wire-4".to_owned(), at: grid(0, 0) };
    let preview = project.reshaped(&edit, grid(0, 1)).unwrap();
    assert_ne!(preview.wires().as_ptr(), project.wires().as_ptr(), "the preview must be fresh geometry");
    for _ in 0..8 {
        assert!(Rc::ptr_eq(&draft.appearances(&preview, revision, true), &committed));
    }
    assert_eq!(Rc::strong_count(&committed), 2);
    assert!(draft.appearance_cache.borrow().snapshot.as_ref().is_some_and(|snapshot| std::ptr::eq(snapshot.wires(), project.wires())));

    // A genuine revision change cannot reuse the stale map.
    let changed = draft.appearances(&project, revision + 1, true);
    assert!(!Rc::ptr_eq(&changed, &committed));
}

/// The emphasis cue is a contrasting underlay wider than the ink, distinct from both the accent
/// and the paper in either theme. It covers selected, hover/error and ordinary ink at the narrow
/// and maximum screen-space widths.
#[test]
fn emphasis_cue_contrasts_with_paper_and_ink_in_both_themes() {
    let ctx = egui::Context::default();
    for dark in [false, true] {
        pcbcraft_chrome::theme::apply(&ctx, dark);
        let tokens = pcbcraft_chrome::Tokens::get();
        for (color, width) in [
            (tokens.accent, pcbcraft_engine::nets::MAX_WIDTH_CP as f32 / 100.0),
            (tokens.error, 2.0 * pcbcraft_chrome::theme::SCHEMATIC_SYMBOL_STROKE),
            (tokens.schematic_wire, pcbcraft_chrome::theme::SCHEMATIC_SYMBOL_STROKE),
        ] {
            let [(halo_width, halo_color), (ink_width, ink_color)] = super::emphasis_strokes(color, width);
            assert_eq!((ink_width, ink_color), (width, color));
            assert!(halo_width > ink_width, "the underlay must extend past the ink at every zoom");
            assert_eq!(halo_color, tokens.text);
            assert_ne!(halo_color, ink_color, "the cue must survive a net color equal to the ink");
            assert_ne!(halo_color, tokens.schematic_paper, "the cue must read against the paper");
        }
    }
    pcbcraft_chrome::theme::apply(&ctx, true);
}

/// A whole-wire selection paints both the accent ink and its wider contrasting underlay on the
/// wire's route. The net color equals the accent and the drawn width is the maximum, so this is the
/// pathological case where color and width alone cannot show that the wire is selected.
#[test]
fn selected_wire_paints_the_contrasting_cue_under_its_ink() {
    let mut h = styled_harness();
    let wire = h.state().session.project().unwrap().wires()[0].id();
    let project = h.state().session.project().unwrap();
    let route = project.wire_path(&project.wires()[0]);
    let points: Vec<_> = route.iter().map(|point| screen(&h, *point)).collect();
    h.state_mut().select(pcbcraft_engine::selection::Selection::one_wire(wire));
    h.run();
    let tokens = pcbcraft_chrome::Tokens::get();
    let [(halo_width, halo_color), (ink_width, ink_color)] =
        super::emphasis_strokes(tokens.accent, pcbcraft_engine::nets::MAX_WIDTH_CP as f32 / 100.0);
    assert!(painted_stroke(&h, &points, ink_width, ink_color), "accent ink on the styled selected wire");
    assert!(painted_stroke(&h, &points, halo_width, halo_color), "selection cue under the styled selected wire");
}

/// Assert both the saved ink and its wider contrasting underlay were painted on the unselected
/// wire, i.e. paint recognised the saved colour as matching its surface.
fn assert_matching_wire_has_cue(h: &Harness<'_, PcbApp>, color: egui::Color32, mode: &str) {
    let project = h.state().session.project().unwrap();
    let route = project.wire_path(&project.wires()[0]);
    let points: Vec<_> = route.iter().map(|point| screen(h, *point)).collect();
    let [(halo_width, halo_color), (ink_width, ink_color)] = super::emphasis_strokes(color, pcbcraft_engine::nets::MAX_WIDTH_CP as f32 / 100.0);
    assert!(painted_stroke(h, &points, ink_width, ink_color), "{mode}: matching ink on the unselected wire");
    assert!(painted_stroke(h, &points, halo_width, halo_color), "{mode}: contrasting cue under the unselected wire");
}

/// Resolve the surface colour under the styled wire's midpoint for a paper preference, using the
/// same [`super::super::surface_at`] that painting uses. A probe frame applies the preference so
/// Uniform, Shaded and Custom fills are all reflected.
fn wire_surface_color(paper: impl Fn(&mut crate::preferences::Preferences)) -> egui::Color32 {
    let probe = styled_harness_with_paper(egui::Color32::WHITE, paper);
    let app = probe.state();
    let project = app.session.project().unwrap();
    let route = project.wire_path(&project.wires()[0]);
    let (a, b) = (route[0], *route.last().unwrap());
    let midpoint = pcbcraft_engine::schematic::Point { x_nm: a.x_nm + (b.x_nm - a.x_nm) / 2, y_nm: a.y_nm + (b.y_nm - a.y_nm) / 2 };
    super::super::surface_at(project, midpoint, app.preferences.paper())
}

/// A wire crossing onto custom paper with the same colour as its ink and the light-theme text
/// needs a visible underlay. Its midpoint is on the canvas and would miss the matching fill.
#[test]
fn wire_crossing_canvas_and_matching_custom_paper_keeps_a_cue() {
    use crate::preferences::{PaperBackground, ThemeChoice};

    let rgb = [0x26, 0x2A, 0x32];
    let color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    let mut h = styled_harness_with_paper(color, |preferences| {
        preferences.theme = ThemeChoice::Light;
        preferences.schematic_paper_background = PaperBackground::Custom;
        preferences.schematic_paper_color_light = Some(rgb);
    });
    let project = h.state().session.project().unwrap();
    let route = project.wire_path(&project.wires()[0]);
    let [from, to] = route.as_slice() else { panic!("expected a straight route, got {route:?}") };
    let symbols = project.symbols();
    assert!(symbols[0].position.x_nm < from.x_nm && from.x_nm < to.x_nm && to.x_nm < symbols[1].position.x_nm);
    let midpoint = Point { x_nm: from.x_nm + (to.x_nm - from.x_nm) / 2, y_nm: from.y_nm };
    let paper_at = Point { x_nm: midpoint.x_nm + (to.x_nm - midpoint.x_nm) / 2, y_nm: midpoint.y_nm - 5_000_000 };
    let sheet = project.sheet_target().id();
    let expected_revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply("sheet.set_page", &json!({"sheet":sheet,"paper":"a4","orientation":"landscape","at":paper_at,"expected_revision":expected_revision}))
        .unwrap();
    h.run();

    let app = h.state();
    let project = app.session.project().unwrap();
    let paper = app.preferences.paper();
    assert_eq!(pcbcraft_chrome::Tokens::get().text, color);
    assert_ne!(paper.canvas, color);
    assert_eq!(super::super::surface_at(project, *from, paper), paper.canvas);
    assert_eq!(super::super::surface_at(project, midpoint, paper), paper.canvas);
    assert_eq!(super::super::surface_at(project, *to, paper), color);
    let points: Vec<_> = route.iter().map(|point| screen(&h, *point)).collect();
    let width = f32::from(pcbcraft_engine::nets::MAX_WIDTH_CP) / 100.0;
    assert!(painted_stroke(&h, &points, width, color));
    assert!(painted_stroke(&h, &points, width + 2.0 * super::EMPHASIS_HALO, egui::Color32::WHITE));
}

/// On the default dark, uniform background the whole sheet is painted with `tokens.canvas`
/// (#2E3440), not the shaded-paper token. A wire saved with that exact colour must still get the
/// contrasting underlay, without selection or highlight, or it vanishes into the canvas.
#[test]
fn default_uniform_background_canvas_colored_wire_keeps_a_cue() {
    let ctx = egui::Context::default();
    pcbcraft_chrome::theme::apply(&ctx, true);
    let canvas = pcbcraft_chrome::Tokens::get().canvas;
    assert_eq!(canvas, egui::Color32::from_rgb(0x2E, 0x34, 0x40), "the default dark canvas is #2E3440");
    assert_ne!(canvas, pcbcraft_chrome::Tokens::get().schematic_paper, "the canvas differs from the shaded-paper token");

    let h = styled_harness_with(canvas);
    assert_eq!(h.state().preferences.schematic_paper_background, crate::preferences::PaperBackground::Uniform);
    assert_eq!(h.state().preferences.paper().fill(), canvas, "uniform paper fills with the resolved canvas");
    assert_matching_wire_has_cue(&h, canvas, "default uniform canvas");
}

/// A wire whose saved colour matches the resolved surface also gets the cue under Shaded and Custom
/// paper, where the sheet is filled with the shaded or custom colour rather than the canvas.
#[test]
fn shaded_and_custom_paper_matching_wires_keep_a_cue() {
    let ctx = egui::Context::default();
    pcbcraft_chrome::theme::apply(&ctx, true);

    let shaded = |p: &mut crate::preferences::Preferences| p.schematic_paper_background = crate::preferences::PaperBackground::Shaded;
    let shaded_surface = wire_surface_color(shaded);
    assert_eq!(shaded_surface, pcbcraft_chrome::Tokens::get().schematic_paper, "the shaded wire sits on paper");
    let h = styled_harness_with_paper(shaded_surface, shaded);
    assert_matching_wire_has_cue(&h, shaded_surface, "shaded paper");

    let custom_rgb = [0x12, 0x34, 0x56];
    let custom = |p: &mut crate::preferences::Preferences| {
        p.schematic_paper_background = crate::preferences::PaperBackground::Custom;
        p.schematic_paper_color_dark = Some(custom_rgb);
    };
    let custom_surface = wire_surface_color(custom);
    assert_eq!(custom_surface, egui::Color32::from_rgb(custom_rgb[0], custom_rgb[1], custom_rgb[2]), "the custom wire sits on paper");
    let h = styled_harness_with_paper(custom_surface, custom);
    assert_matching_wire_has_cue(&h, custom_surface, "custom paper");
}
