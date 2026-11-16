//! Zone acceptance uses real ribbon, net chooser, canvas and Properties controls.
use super::*;
use egui::accesskit::Role;
use pcbcraft_engine::{CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_RESISTOR, persistence::encode_documents, schematic::Point};
/// Build a current unnamed net and configured board without placing artificial project records.
fn fixture() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1600., 1000.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    for x in [0, 25_400_000] {
        run(&mut h, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0}));
    }
    run(&mut h, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}}));
    run(&mut h, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true}));
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB Layout");
    h.state_mut().browser.open = false;
    h.state_mut().views.active.board.layers_collapsed = true;
    h.state_mut().views.active.board.properties_collapsed = true;
    h.state_mut().views.active.camera.set_zoom(1.0);
    h.state_mut().views.active.camera.center_on(Point { x_nm: 0, y_nm: 0 });
    h.run();
    h
}
fn run(h: &mut Harness<'_, PcbApp>, id: &str, mut params: Value) {
    if id != "project.rename" {
        params["expected_revision"] = json!(h.state().session.revision());
    }
    h.state_mut().run(id, &params);
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
}
fn at(h: &Harness<'_, PcbApp>, x: i64, y: i64) -> egui::Pos2 {
    h.state().views.active.camera.screen(Point { x_nm: x, y_nm: y }, h.get_by_label("PCB canvas").rect())
}
/// Shape/net selection authors nothing; only the completed outline promotes and adds one zone.
fn draw(h: &mut Harness<'_, PcbApp>, rectangle: bool) {
    click(h, "Draw zone outlines");
    assert!(matches!(h.state().dialog, Dialog::Zone(_)));
    if rectangle {
        click(h, "Rectangle");
    }
    h.get_by_role_and_label(Role::Button, "Draw").click();
    h.run();
    let points = if rectangle {
        vec![(0, 0), (10_160_000, 10_160_000)]
    } else {
        vec![(-20_320_000, -10_160_000), (-10_160_000, -10_160_000), (-10_160_000, 0)]
    };
    for (x, y) in points {
        let p = at(h, x, y);
        click_canvas_at(h, p);
    }
    if !rectangle {
        h.key_press(Key::Enter);
        h.run();
    }
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.key_press(Key::Escape);
    h.run();
}
/// Both shipped shapes create one transaction with promotion, preserve no-fill copper, and undo exactly.
#[test]
fn real_zone_rectangle_and_polygon_are_single_edits() {
    for rectangle in [true, false] {
        let mut h = fixture();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        draw(&mut h, rectangle);
        let p = h.state().session.project().unwrap();
        let zone = p.boards().next().unwrap().zones();
        assert_eq!(zone.len(), 1);
        assert_eq!(zone[0].points.len(), if rectangle { 4 } else { 3 });
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(p.boards().next().unwrap().tracks().is_empty());
        let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
    }
}
/// Zone edges select/move/delete through ordinary shortcuts; filters cancel and block edits.
#[test]
fn selected_zone_moves_deletes_and_respects_filters() {
    let mut h = fixture();
    draw(&mut h, true);
    let edge = at(&h, 5_080_000, 0);
    click_canvas_at(&mut h, edge);
    assert!(h.state().views.active.board.zones.selected.is_some());
    let before = h.state().session.snapshot().unwrap();
    let original = before.boards().next().unwrap().zones()[0].clone();
    let revision = h.state().session.revision();
    h.hover_at(edge);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.zones.source().is_some());
    let end = at(&h, 10_160_000, 5_080_000);
    click_canvas_at(&mut h, end);
    assert_eq!(h.state().session.revision(), revision + 1);
    let next = h.state().session.project().unwrap().boards().next().unwrap().zones()[0].clone();
    assert_eq!(next.id, original.id);
    assert_eq!(next.points[0], Point { x_nm: 5_080_000, y_nm: 5_080_000 });
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().session.project().unwrap().boards().next().unwrap().zones().is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"zones":false})).unwrap();
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click_canvas_at(&mut h, end);
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}
/// Properties settings use shared fields and one authored command after committing a value.
#[test]
fn zone_properties_edit_the_shared_options() {
    let mut h = fixture();
    draw(&mut h, true);
    let edge = at(&h, 5_080_000, 0);
    click_canvas_at(&mut h, edge);
    h.state_mut().views.active.board.properties_collapsed = false;
    h.run();
    let revision = h.state().session.revision();
    let field = h.get_by_role_and_label(Role::TextInput, "Priority");
    field.scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Priority").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role_and_label(Role::TextInput, "Priority").type_text("7");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().zones()[0].options.priority, 7);
}
/// Vertex reshapes use the common path editor, while stale commands and cancelled drafts author nothing.
#[test]
fn zone_reshape_cancel_and_stale_view_are_safe() {
    let mut h = fixture();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Draw zone outlines");
    h.get_by_role_and_label(Role::Button, "Draw").click();
    h.run();
    let p = at(&h, 0, 0);
    click_canvas_at(&mut h, p);
    h.key_press(Key::Escape);
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    draw(&mut h, true);
    let edge = at(&h, 5_080_000, 0);
    click_canvas_at(&mut h, edge);
    let revision = h.state().session.revision();
    let corner = at(&h, 0, 0);
    let end = at(&h, -5_080_000, -5_080_000);
    h.drag_at(corner);
    h.run();
    h.hover_at(end);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    h.drop_at(end);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().zones()[0].points[0], Point { x_nm: -5_080_000, y_nm: -5_080_000 });
    let zone = h.state().session.project().unwrap().boards().next().unwrap().zones()[0].clone();
    h.drag_at(end);
    h.run();
    run(&mut h, "project.rename", json!({"name":"Changed elsewhere"}));
    h.run();
    h.hover_at(corner);
    h.run();
    h.drop_at(corner);
    h.run();
    assert_eq!(h.state().session.project().unwrap().boards().next().unwrap().zones()[0], zone);
    let layer = zone.layer.to_string();
    let before = h.state().session.snapshot().unwrap();
    h.state_mut().views.active.board.set_layer(&layer, false, Some(false));
    h.run();
    assert!(h.state().views.active.board.zones.selected.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Cancelling or invalidating the net chooser never creates a zone or promotes an identity.
#[test]
fn zone_chooser_cannot_author_after_a_revision_change() {
    let mut h = fixture();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Draw zone outlines");
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Draw zone outlines");
    run(&mut h, "project.rename", json!({"name":"Changed outside the chooser"}));
    let changed = h.state().session.snapshot().unwrap();
    h.run();
    h.get_by_role_and_label(Role::Button, "Draw").click();
    h.run();
    assert!(Arc::ptr_eq(&changed, &h.state().session.snapshot().unwrap()));
    assert!(h.state().session.project().unwrap().boards().next().unwrap().zones().is_empty());
    assert!(h.state().views.active.board.zones.tool.is_none());
}

/// Fit includes authored zone extents even when there are no placed footprints or filled copper.
#[test]
fn zone_extents_are_included_in_fit() {
    let mut h = fixture();
    draw(&mut h, true);
    let zone = h.state().session.project().unwrap().boards().next().unwrap().zones()[0].clone();
    let points: Vec<_> = zone.points.iter().map(|p| Point { x_nm: p.x_nm + 200_000_000, y_nm: p.y_nm + 100_000_000 }).collect();
    run(&mut h, pcbcraft_engine::board::zones::SET_ZONE, json!({"zone":zone.id,"points":points}));
    h.run();
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    let rect = h.get_by_label("PCB canvas").rect().shrink(10.);
    for point in points {
        assert!(rect.contains(at(&h, point.x_nm, point.y_nm)));
    }
}
