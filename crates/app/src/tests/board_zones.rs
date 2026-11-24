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

/// Escape belongs to each scalar field; it discards text and leaves the zone selected.
#[test]
fn zone_scalar_escape_discards_without_clearing_selection() {
    for (field, value) in [("Priority", "7"), ("Minimum island area", "0.25")] {
        let mut h = fixture();
        draw(&mut h, true);
        let edge = at(&h, 5_080_000, 0);
        click_canvas_at(&mut h, edge);
        h.state_mut().views.active.board.properties_collapsed = false;
        h.run();
        h.get_by_role_and_label(Role::TextInput, field).scroll_to_me();
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.get_by_role_and_label(Role::TextInput, field).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_role_and_label(Role::TextInput, field).type_text(value);
        h.run();
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(h.state().session.revision(), revision, "{field}");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        assert!(h.state().views.active.board.zones.selected.is_some());
        h.get_by_role_and_label(Role::TextInput, field).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_role_and_label(Role::TextInput, field).type_text(value);
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(h.state().session.revision(), revision + 1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Net, layer and connection choices each commit once, while Escape preserves exact bytes.
#[test]
fn zone_properties_choices_commit_and_restore_exact_history() {
    for (field, option) in [("Connection", "Solid"), ("Layer", "Bottom"), ("Net", "Net 2")] {
        let mut h = fixture();
        if field == "Net" {
            let mut symbols = Vec::new();
            for x in [0, 25_400_000] {
                run(&mut h, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":50_800_000}));
                symbols.push(h.state().session.project().unwrap().symbols().last().unwrap().id());
            }
            run(&mut h, CONNECT_PINS, json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"}}));
            h.run();
        }
        draw(&mut h, true);
        let edge = at(&h, 5_080_000, 0);
        click_canvas_at(&mut h, edge);
        h.state_mut().views.active.board.properties_collapsed = false;
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        let original = h.state().session.project().unwrap().boards().next().unwrap().zones()[0].clone();
        let nets = h.state().session.project().unwrap().inspect_net_registry().unwrap();
        let chosen_net = nets["nets"][1]["anchor"].clone();
        let option = if field == "Net" { nets["nets"][1]["name"].as_str().unwrap() } else { option };
        h.get_by_role_and_label(Role::ComboBox, field).scroll_to_me();
        h.run();
        h.get_by_role_and_label(Role::ComboBox, field).click();
        h.run();
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.get_by_role_and_label(Role::ComboBox, field).click();
        h.run();
        click(&mut h, option);
        assert_eq!(h.state().session.revision(), revision + 1, "{field}");
        let project = h.state().session.project().unwrap();
        let zone = project.boards().next().unwrap().zones()[0].clone();
        let mut expected = original;
        match field {
            "Connection" => expected.options.connection = pcbcraft_engine::board::zones::Connection::Solid,
            "Layer" => expected.layer = project.boards().next().unwrap().layers().into_iter().find(|l| l.name == "Bottom").unwrap().id,
            _ => {
                let rows = project.inspect_net_registry().unwrap();
                let row = rows["nets"].as_array().unwrap().iter().find(|r| r["anchor"] == chosen_net).unwrap();
                assert_eq!(row["id"], json!(zone.net));
                assert_ne!(zone.net, expected.net);
                expected.net = zone.net;
            }
        }
        assert_eq!(zone, expected, "{field}");
        let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
    }
}

/// An armed chooser's captured net must expire even before the first drawing point exists.
#[test]
fn armed_zone_net_choice_expires_before_the_first_point() {
    let mut h = fixture();
    click(&mut h, "Draw zone outlines");
    h.get_by_role_and_label(Role::Button, "Draw").click();
    h.run();
    assert!(h.state().views.active.board.zones.tool.is_some());
    run(&mut h, "project.rename", json!({"name":"Changed after choosing the net"}));
    let changed = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.run();
    assert!(h.state().views.active.board.zones.tool.is_none());
    for (x, y) in [(0, 0), (10_160_000, 0), (10_160_000, 10_160_000)] {
        let point = at(&h, x, y);
        click_canvas_at(&mut h, point);
    }
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), changed);
}

/// Accepted repeated creations renew only their own net choice; an external edit expires it.
#[test]
fn repeated_zone_creation_keeps_its_net_until_an_external_edit() {
    let mut h = fixture();
    click(&mut h, "Draw zone outlines");
    click(&mut h, "Rectangle");
    h.get_by_role_and_label(Role::Button, "Draw").click();
    h.run();
    for offset in [0, 20_320_000] {
        let revision = h.state().session.revision();
        for (x, y) in [(offset, 0), (offset + 10_160_000, 10_160_000)] {
            let point = at(&h, x, y);
            click_canvas_at(&mut h, point);
        }
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().views.active.board.zones.tool.is_some());
    }
    let project = h.state().session.project().unwrap();
    let zones = project.boards().next().unwrap().zones();
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0].net, zones[1].net);
    run(&mut h, "project.rename", json!({"name":"Changed between repeated outlines"}));
    let changed = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.run();
    assert!(h.state().views.active.board.zones.tool.is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), changed);
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

/// Separate authored rectangles expose real click and box selection without overlapping targets.
fn two_zones() -> Harness<'static, PcbApp> {
    let mut h = fixture();
    draw(&mut h, true);
    let zone = h.state().session.project().unwrap().boards().next().unwrap().zones()[0].clone();
    let points: Vec<_> = zone.points.iter().map(|p| Point { x_nm: p.x_nm - 20_320_000, y_nm: p.y_nm }).collect();
    run(&mut h, pcbcraft_engine::board::zones::SET_ZONE, json!({"zone":zone.id,"points":points}));
    h.run();
    draw(&mut h, true);
    h
}

/// Preserve modifiers for the whole native-style press/release pair, then restore ordinary input.
fn zone_click(h: &mut Harness<'_, PcbApp>, point: egui::Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(point));
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run();
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// The canvas owns a directional window from press through release, including modifier state.
fn zone_box(h: &mut Harness<'_, PcbApp>, from: egui::Pos2, to: egui::Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run();
    h.event(egui::Event::PointerMoved(from + (to - from) * 0.5));
    h.run();
    h.event(egui::Event::PointerMoved(to));
    h.run();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Ctrl/Command toggle zones independently; Shift adds, and selecting never changes documents.
#[test]
fn zone_modifier_clicks_keep_multiple_targets() {
    for modifier in [Modifiers::CTRL, Modifiers::COMMAND] {
        let mut h = two_zones();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let a = at(&h, -15_240_000, 0);
        let b = at(&h, 5_080_000, 0);
        zone_click(&mut h, a, Modifiers::NONE);
        zone_click(&mut h, b, modifier);
        assert_eq!(h.state().views.active.board.zones.selection().len(), 2);
        zone_click(&mut h, a, modifier);
        assert_eq!(h.state().views.active.board.zones.selection().len(), 1);
        zone_click(&mut h, a, Modifiers::SHIFT);
        assert_eq!(h.state().views.active.board.zones.selection().len(), 2);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        let revision = h.state().session.revision();
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().session.project().unwrap().boards().next().unwrap().zones().is_empty());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Left-to-right boxes contain whole zones, right-to-left crosses edges; modifier boxes combine.
#[test]
fn zone_boxes_follow_direction_modifiers_and_visibility() {
    let mut h = two_zones();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let from = at(&h, -22_860_000, -2_540_000);
    let to = at(&h, 12_700_000, 12_700_000);
    zone_box(&mut h, from, to, Modifiers::NONE);
    assert_eq!(h.state().views.active.board.zones.selection().len(), 2);
    let partial = at(&h, -15_240_000, 2_540_000);
    zone_box(&mut h, from, partial, Modifiers::NONE);
    assert!(h.state().views.active.board.zones.selection().is_empty());
    zone_box(&mut h, partial, from, Modifiers::NONE);
    assert_eq!(h.state().views.active.board.zones.selection().len(), 1);
    zone_box(&mut h, from, to, Modifiers::SHIFT);
    assert_eq!(h.state().views.active.board.zones.selection().len(), 2);
    zone_box(&mut h, partial, from, Modifiers::CTRL);
    assert_eq!(h.state().views.active.board.zones.selection().len(), 1);
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"zones":false})).unwrap();
    zone_box(&mut h, from, to, Modifiers::NONE);
    assert!(h.state().views.active.board.zones.selection().is_empty());
    h.state_mut().set_selection_filters(id, &json!({"zones":true})).unwrap();
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    h.state_mut().views.active.board.hidden_layers.insert(layer);
    zone_box(&mut h, from, to, Modifiers::NONE);
    assert!(h.state().views.active.board.zones.selection().is_empty());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The ordinary shared context menu enables Delete for zone-only groups in either theme.
#[test]
fn zone_context_menu_deletes_the_complete_selection() {
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        let mut h = two_zones();
        h.state_mut().preferences.theme = theme;
        h.run();
        let a = at(&h, -15_240_000, 0);
        let b = at(&h, 5_080_000, 0);
        zone_click(&mut h, a, Modifiers::NONE);
        zone_click(&mut h, b, Modifiers::CTRL);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: b, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
            h.run();
        }
        assert!(!h.get_by_label("Delete").accesskit_node().is_disabled());
        click(&mut h, "Delete");
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().session.project().unwrap().boards().next().unwrap().zones().is_empty());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// The shared context menu can split and remove vertices after drawing a zone. Both writes
/// retain its identity and fill options, and removing a fourth vertex is independently undoable.
#[test]
fn zone_vertex_menu_splits_and_removes_an_edge_point_in_both_themes() {
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        let mut h = fixture();
        h.state_mut().preferences.theme = theme;
        h.run();
        draw(&mut h, true);
        let edge = at(&h, 5_080_000, 0);
        click_canvas_at(&mut h, edge);
        let zones = |h: &Harness<'_, PcbApp>| h.state().session.project().unwrap().boards().next().unwrap().zones().to_vec();
        let original = zones(&h)[0].clone();
        let revision = h.state().session.revision();
        for label in ["Add vertex", "Remove vertex"] {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: edge, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
                h.run();
            }
            assert!(!h.get_by_label(label).accesskit_node().is_disabled());
            assert!(!h.get_by_label("Delete").accesskit_node().is_disabled());
            click(&mut h, label);
            let changed = zones(&h).remove(0);
            assert_eq!(changed.id, original.id);
            assert_eq!(changed.options, original.options);
            if label == "Add vertex" {
                assert_eq!(changed.points.len(), original.points.len() + 1);
                assert!(changed.points.contains(&Point { x_nm: 5_080_000, y_nm: 0 }));
            } else {
                assert_eq!(changed, original);
            }
        }
        assert_eq!(h.state().session.revision(), revision + 2);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(zones(&h)[0].points.len(), original.points.len() + 1);
    }
}

/// Modifier clicks preserve other categories in either order; a shared mixed Delete is one edit.
#[test]
fn zones_join_mixed_selection_without_losing_footprints() {
    let mut h = two_zones();
    for (symbol, x) in [("symbol-2", 0), ("symbol-3", 25_400_000)] {
        run(
            &mut h,
            pcbcraft_engine::PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0}),
        );
    }
    h.run();
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().last().unwrap().position;
    let fp = at(&h, footprint.x_nm, footprint.y_nm);
    let zone = at(&h, -15_240_000, 0);
    zone_click(&mut h, zone, Modifiers::NONE);
    zone_click(&mut h, fp, Modifiers::CTRL);
    assert_eq!(h.state().views.active.board.selection.len(), 1);
    assert_eq!(h.state().views.active.board.zones.selection().len(), 1);
    zone_click(&mut h, zone, Modifiers::CTRL);
    assert_eq!(h.state().views.active.board.selection.len(), 1);
    assert!(h.state().views.active.board.zones.selection().is_empty());
    zone_click(&mut h, zone, Modifiers::CTRL);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    assert_eq!(board.zones().len(), 1);
    assert_eq!(board.footprints().count(), 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    assert_eq!(h.state().views.active.board.zones.selection().len(), 2);
}

/// Multiple footprints and a full mixed selection move rigidly from an empty-space M pickup.
#[test]
fn board_group_m_moves_all_selected_categories() {
    for mixed in [false, true] {
        let mut h = two_zones();
        for (symbol, x) in [("symbol-2", 0), ("symbol-3", 25_400_000)] {
            run(
                &mut h,
                pcbcraft_engine::PLACE_FOOTPRINT,
                json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0}),
            );
        }
        h.run();
        if mixed {
            let board = h.state().session.project().unwrap().boards().next().unwrap();
            let layer = board.zones()[0].layer;
            let outline = board.layers().into_iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline).unwrap().id;
            run(
                &mut h,
                pcbcraft_engine::ADD_DRAWING,
                json!({"layer":outline,"closed":false,"points":[{"x_nm":0,"y_nm":15_240_000},{"x_nm":10_160_000,"y_nm":15_240_000}]}),
            );
            run(
                &mut h,
                pcbcraft_engine::ADD_TRACKS,
                json!({"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":20_320_000},{"x_nm":10_160_000,"y_nm":20_320_000}]}),
            );
            run(&mut h, pcbcraft_engine::ADD_VIA, json!({"at":{"x_nm":10_160_000,"y_nm":20_320_000}}));
            h.run();
            h.state_mut().select_all_board();
        } else {
            let ids = h.state().session.project().unwrap().boards().next().unwrap().footprint_ids().collect();
            h.state_mut().views.active.board.selection = ids;
        }
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let bytes = encode_documents(before.clone()).unwrap();
        let revision = h.state().session.revision();
        let start = at(&h, -5_080_000, -5_080_000);
        let end = at(&h, 0, -2_540_000);
        h.hover_at(start);
        h.run();
        h.key_press(Key::M);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        h.hover_at(end);
        h.run();
        click_canvas_at(&mut h, end);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.revision(), revision + 1);
        let after = h.state().session.snapshot().unwrap();
        let old = before.boards().next().unwrap();
        let new = after.boards().next().unwrap();
        for (a, b) in old.footprints().zip(new.footprints()) {
            assert_eq!(b.position, Point { x_nm: a.position.x_nm + 5_080_000, y_nm: a.position.y_nm + 2_540_000 }, "mixed={mixed}");
        }
        for (a, b) in old.zones().iter().zip(new.zones()) {
            for (a, b) in a.points.iter().zip(&b.points) {
                assert_eq!(*b, if mixed { Point { x_nm: a.x_nm + 5_080_000, y_nm: a.y_nm + 2_540_000 } } else { *a });
            }
        }
        if mixed {
            for (a, b) in old.drawings().iter().zip(new.drawings()) {
                for (a, b) in a.points.iter().zip(b.points) {
                    assert_eq!(b, Point { x_nm: a.x_nm + 5_080_000, y_nm: a.y_nm + 2_540_000 });
                }
            }
            for (a, b) in old.tracks().iter().zip(new.tracks()) {
                assert_eq!(b.start, Point { x_nm: a.start.x_nm + 5_080_000, y_nm: a.start.y_nm + 2_540_000 });
                assert_eq!(b.end, Point { x_nm: a.end.x_nm + 5_080_000, y_nm: a.end.y_nm + 2_540_000 });
            }
            for (a, b) in old.vias().iter().zip(new.vias()) {
                assert_eq!(b.at, Point { x_nm: a.at.x_nm + 5_080_000, y_nm: a.at.y_nm + 2_540_000 });
            }
        }
        assert_eq!(new.footprints().count(), old.footprints().count());
        let after_bytes = encode_documents(after.clone()).unwrap();
        assert_eq!(encode_documents(pcbcraft_engine::persistence::decode_documents(&after_bytes).unwrap()).unwrap(), after_bytes);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
}

/// Cancellation and authority changes discard the whole pickup; a fresh M at the same revision
/// captures its new pointer anchor rather than reusing the cancelled gesture's cached origin.
#[test]
fn board_group_move_cancellation_and_stale_authority_are_atomic() {
    for reason in ["escape", "secondary", "focus", "revision", "filter", "hidden"] {
        let mut h = two_zones();
        h.state_mut().select_all_board();
        h.run();
        let items = crate::board_clipboard::selection(&h.state().views.active.board, h.state().session.project().unwrap());
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let start = at(&h, -5_080_000, -5_080_000);
        let end = at(&h, 0, -2_540_000);
        h.hover_at(start);
        h.run();
        h.key_press(Key::M);
        h.run();
        assert!(h.state().views.active.board.movement.active(), "{reason}");
        h.hover_at(end);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        match reason {
            "escape" => h.key_press(Key::Escape),
            "secondary" => {
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
                    h.run();
                }
            }
            "focus" => h.event(egui::Event::WindowFocused(false)),
            "revision" => run(&mut h, "project.rename", json!({"name":"Unrelated edit during group pickup"})),
            "filter" => {
                let id = h.state().views.active.id();
                h.state_mut().set_selection_filters(id, &json!({"zones":false})).unwrap();
            }
            "hidden" => {
                let layer = h.state().views.active.board.active_layer.clone().unwrap();
                h.state_mut().views.active.board.hidden_layers.insert(layer);
            }
            _ => panic!("Unknown test cancellation reason {reason}"),
        }
        h.run();
        let cancelled = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.event(egui::Event::WindowFocused(true));
        h.run();
        assert!(!h.state().views.active.board.movement.active(), "{reason}");
        click_canvas_at(&mut h, end);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), cancelled);
        if reason == "escape" {
            h.state_mut().select_board_items(items);
            h.run();
            h.hover_at(end);
            h.run();
            h.key_press(Key::M);
            h.run();
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
            let next = at(&h, 5_080_000, 0);
            h.hover_at(next);
            h.run();
            click_canvas_at(&mut h, next);
            let points = &h.state().session.project().unwrap().boards().next().unwrap().zones()[0].points;
            let old = pcbcraft_engine::persistence::decode_documents(&before).unwrap();
            assert_eq!(points[0].x_nm, old.boards().next().unwrap().zones()[0].points[0].x_nm + 5_080_000);
        }
    }
}

/// The shared ribbon exposes group pickup too; its off-canvas invocation uses a selected anchor.
#[test]
fn board_group_ribbon_move_places_the_whole_zone_selection() {
    let mut h = two_zones();
    h.state_mut().select_all_board();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let first = before.boards().next().unwrap().zones()[0].points[0];
    assert!(!h.get_by_label("Move selected PCB items or text").accesskit_node().is_disabled());
    click(&mut h, "Move selected PCB items or text");
    assert!(h.state().views.active.board.movement.active());
    let destination = at(&h, first.x_nm + 5_080_000, first.y_nm + 2_540_000);
    click_canvas_at(&mut h, destination);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let after = h.state().session.project().unwrap().boards().next().unwrap();
    for (old, new) in before.boards().next().unwrap().zones().iter().zip(after.zones()) {
        assert_eq!(new.points[0], Point { x_nm: old.points[0].x_nm + 5_080_000, y_nm: old.points[0].y_nm + 2_540_000 });
    }
}
