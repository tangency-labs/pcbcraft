//! Track routing exercises real canvas input and the same transactions used by MCP.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL,
    board::{ratsnest::Airwire, track::Track, update::INITIALIZE_BASELINE},
    persistence::encode_documents,
    schematic::Point,
};

/// Two 0402 resistors whose first terminals share an accepted net, on a two-layer board,
/// opened in the PCB layout with the Top copper layer active.
fn routable<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    let mut symbols = Vec::new();
    for index in 0..2 {
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
        let symbol = h.state().session.project().unwrap().symbols().last().unwrap().id();
        symbols.push(symbol.clone());
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":index*10_160_000,"y_nm":0,"expected_revision":revision}));
    }
    let revision = h.state().session.revision();
    h.state_mut().run(
        CONNECT_PINS,
        &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":revision}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BASELINE, &json!({"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    click(&mut h, "Activate layer Top");
    h
}

fn tracks(h: &Harness<'_, PcbApp>) -> Vec<Track> {
    h.state().session.project().unwrap().boards().next().unwrap().tracks().to_vec()
}

fn airwires(h: &Harness<'_, PcbApp>) -> Vec<Airwire> {
    h.state().session.project().unwrap().boards().next().unwrap().ratsnest().unwrap().lines
}

fn at(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("PCB canvas").rect())
}

/// Click, then let the double-click window lapse so the next click is a separate vertex.
fn click_route(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = at(h, point);
    click_canvas_at(h, pos);
    h.run_steps(20);
}

/// Route around the pads: up from the first pad, across, and down into the second.
fn route_airwire(h: &mut Harness<'_, PcbApp>, line: &Airwire) {
    let (a, b) = (line.from.position, line.to.position);
    click_route(h, a);
    click_route(h, Point { x_nm: a.x_nm, y_nm: -2_540_000 });
    click_route(h, Point { x_nm: b.x_nm, y_nm: -2_540_000 });
    click_route(h, b);
}

/// X arms the route tool on Top copper. Clicking a pad starts a route, clicks add 45° legs, and
/// clicking the other pad finishes it as one undoable command that clears the airwire.
#[test]
fn x_routes_between_pads_in_one_undoable_command() {
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    assert!(h.state().views.active.board.tracks.tool);
    route_airwire(&mut h, &line);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1, "the whole route is one edit");
    let routed = tracks(&h);
    assert!(routed.len() >= 3, "{routed:?}");
    let top = h.state().views.active.board.active_layer.clone().unwrap();
    assert!(routed.iter().all(|track| track.layer.to_string() == top && track.width_nm == 250_000));
    let ends: Vec<_> = routed.iter().flat_map(|track| [track.start, track.end]).collect();
    assert!(ends.contains(&line.from.position) && ends.contains(&line.to.position), "the route snaps to both pad centers");
    assert!(airwires(&h).is_empty(), "the routed connection needs no airwire");
    assert!(h.state().views.active.board.tracks.tool, "the tool stays armed for the next route");
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.tracks.tool);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    assert_eq!(airwires(&h).len(), 1);
}

/// Escape and right-click drop an unfinished route without editing; Enter finishes an open-ended
/// route; Backspace removes the last vertex. The width field sets the next route's width.
#[test]
fn unfinished_routes_cancel_and_finish_from_the_keyboard() {
    let mut h = routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Route tracks on the active copper layer");
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    click_route(&mut h, start);
    click_route(&mut h, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.board.tracks.tool, "Escape drops the route but keeps the tool");
    click_route(&mut h, start);
    h.event(egui::Event::PointerButton { pos: at(&h, start), button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "cancelled routes edit nothing");

    h.state_mut().views.active.board.tracks.width_nm = 500_000;
    click(&mut h, "Route tracks on the active copper layer");
    for x in [2_540_000, 5_080_000, 7_620_000] {
        click_route(&mut h, Point { x_nm: x, y_nm: 2_540_000 });
    }
    h.key_press(Key::Backspace);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    let routed = tracks(&h);
    assert_eq!(routed.len(), 1, "Backspace removed the last vertex: {routed:?}");
    assert_eq!(routed[0].width_nm, 500_000);
}

/// Without a tool, clicking a track selects it and Delete removes it through the shared command.
#[test]
fn clicking_a_track_selects_it_and_delete_removes_it() {
    let mut h = routable();
    let top = h.state().views.active.board.active_layer.clone().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":2_540_000},{"x_nm":7_620_000,"y_nm":2_540_000}],"expected_revision":revision}),
    );
    h.run();
    assert_eq!(tracks(&h).len(), 1);
    let id = tracks(&h)[0].id;
    let middle = at(&h, Point { x_nm: 3_810_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([id]));
    h.key_press(Key::Delete);
    h.run();
    assert!(tracks(&h).is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(tracks(&h).len(), 1);
    let empty = at(&h, Point { x_nm: 3_810_000, y_nm: -1_270_000 });
    click_canvas_at(&mut h, middle);
    click_canvas_at(&mut h, empty);
    assert!(h.state().views.active.board.tracks.selected.is_empty(), "clicking empty space clears the track selection");
}

/// The route tool is only available on visible copper: X does nothing on Board Outline, and hiding
/// the routing layer drops the tool and any unfinished route.
#[test]
fn routing_requires_a_visible_copper_layer() {
    let mut h = routable();
    click(&mut h, "Activate layer Board Outline");
    h.key_press(Key::X);
    h.run();
    assert!(!h.state().views.active.board.tracks.tool);
    assert!(h.get_by_label("Route tracks on the active copper layer").accesskit_node().is_disabled());
    click(&mut h, "Activate layer Top");
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 });
    let top = h.state().views.active.board.active_layer.clone().unwrap();
    h.state_mut().views.active.board.set_layer(&top, false, Some(false));
    h.run();
    assert!(!h.state().views.active.board.tracks.busy());
    assert!(tracks(&h).is_empty());
}

fn vias(h: &Harness<'_, PcbApp>) -> Vec<pcbcraft_engine::board::track::Via> {
    h.state().session.project().unwrap().boards().next().unwrap().vias().to_vec()
}

/// The copper layer ID with this name.
fn layer_named(h: &Harness<'_, PcbApp>, name: &str) -> String {
    let project = h.state().session.project().unwrap();
    let board = project.boards().next().unwrap();
    board.layers().into_iter().find(|layer| layer.name == name).unwrap().id.to_string()
}

/// Press V with the pointer over a board point, as when dropping a via mid-route.
fn via_at(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = at(h, point);
    h.hover_at(pos);
    h.run();
    h.key_press(Key::V);
    h.run();
}

/// V drops a via at the pointer and continues on the other copper layer; the view's active
/// layer follows. A second V comes back up, and finishing on the pad commits tracks on both
/// layers with both vias in one undoable command that clears the airwire.
#[test]
fn v_places_vias_and_switches_layers_mid_route() {
    let mut h = routable();
    let (top, bottom) = (layer_named(&h, "Top"), layer_named(&h, "Bottom"));
    let line = airwires(&h)[0].clone();
    let (a, b) = (line.from.position, line.to.position);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, a);
    click_route(&mut h, Point { x_nm: a.x_nm, y_nm: -2_540_000 });
    via_at(&mut h, Point { x_nm: a.x_nm, y_nm: -2_540_000 });
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()), "the view follows the via");
    click_route(&mut h, Point { x_nm: b.x_nm, y_nm: -2_540_000 });
    via_at(&mut h, Point { x_nm: b.x_nm, y_nm: -2_540_000 });
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    assert_eq!(h.state().session.revision(), revision, "nothing is authored before the route finishes");
    click_route(&mut h, b);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(vias(&h).len(), 2);
    let routed = tracks(&h);
    assert!(routed.iter().any(|track| track.layer.to_string() == bottom), "the crossing runs on the bottom");
    assert!(routed.iter().any(|track| track.layer.to_string() == top));
    assert!(airwires(&h).is_empty(), "vias carry the connection between layers");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Backspace right after a via removes it and returns to the previous layer, before removing
/// any vertex. A click on a via selects it, and Delete removes only that via.
#[test]
fn backspace_undoes_a_via_and_vias_select_and_delete() {
    let mut h = routable();
    let (top, bottom) = (layer_named(&h, "Top"), layer_named(&h, "Bottom"));
    h.key_press(Key::X);
    h.run();
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    let corner = Point { x_nm: 5_080_000, y_nm: 2_540_000 };
    click_route(&mut h, start);
    via_at(&mut h, corner);
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()));
    h.key_press(Key::Backspace);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()), "Backspace removed the via");
    via_at(&mut h, corner);
    click_route(&mut h, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(vias(&h).len(), 1);
    assert_eq!(tracks(&h).len(), 2);
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.tracks.tool);
    let via = vias(&h)[0].clone();
    let pos = at(&h, via.at);
    click_canvas_at(&mut h, pos);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([via.id]));
    h.key_press(Key::Delete);
    h.run();
    assert!(vias(&h).is_empty());
    assert_eq!(tracks(&h).len(), 2, "deleting the via keeps its tracks");
}

/// Delete pressed while track input is disabled is dropped, not replayed later. A box-selection
/// drag disables track input but keeps the track selection: Delete during the drag, then release,
/// must leave the track in place on the frames after.
#[test]
fn a_delete_pressed_while_track_input_is_disabled_never_deletes_later() {
    let mut h = routable();
    let top = h.state().views.active.board.active_layer.clone().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":2_540_000},{"x_nm":7_620_000,"y_nm":2_540_000}],"expected_revision":revision}),
    );
    h.run();
    let middle = at(&h, Point { x_nm: 3_810_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected.len(), 1);
    let (from, to) = (at(&h, Point { x_nm: 2_540_000, y_nm: -1_270_000 }), at(&h, Point { x_nm: 5_080_000, y_nm: -3_810_000 }));
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    assert!(h.state().views.active.board.box_drag.is_some(), "a box selection is in progress");
    assert_eq!(h.state().views.active.board.tracks.selected.len(), 1);
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(tracks(&h).len(), 1, "track input is disabled during the box drag");
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    h.run();
    assert!(h.state().views.active.board.box_drag.is_none());
    assert_eq!(tracks(&h).len(), 1, "the earlier Delete is not replayed once track input returns");
}

/// Ctrl+Shift+X arms Place › Via. Each click places one via, snapping to a nearby pad center,
/// and the tool stays armed; Esc leaves it. Each via is its own undo step.
#[test]
fn place_via_tool_drops_vias_until_escape() {
    let mut h = routable();
    let pad = airwires(&h)[0].from.position;
    let free = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    h.key_press_modifiers(Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE }, Key::X);
    h.run();
    assert!(h.state().views.active.board.tracks.via_tool);
    assert!(!h.get_by_label("Place vias").accesskit_node().is_disabled(), "Place › Via is a working button");
    let revision = h.state().session.revision();
    click_route(&mut h, free);
    let near_pad = at(&h, pad) + egui::vec2(3., 2.);
    click_canvas_at(&mut h, near_pad);
    h.run_steps(20);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 2, "one edit per via");
    let placed: Vec<_> = vias(&h).iter().map(|via| via.at).collect();
    assert!(placed.contains(&free) && placed.contains(&pad), "{placed:?}");
    assert!(h.state().views.active.board.tracks.via_tool, "the tool stays armed");
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.tracks.via_tool);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(vias(&h).len(), 1, "undo removes only the last via");
}

/// Add a straight front track through the shared command and return its ID.
fn add_track(h: &mut Harness<'_, PcbApp>, from: Point, to: Point) -> uuid::Uuid {
    let top = layer_named(h, "Top");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":[from, to],"expected_revision":revision}));
    h.run();
    tracks(h).into_iter().find(|track| track.start == from && track.end == to).unwrap().id
}

/// A left-to-right box selects tracks and vias it fully contains, without footprints outside it.
#[test]
fn box_selection_picks_tracks_and_vias() {
    let mut h = routable();
    let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":5_080_000,"y_nm":-1_270_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    h.run();
    let via = vias(&h)[0].id;
    let (from, to) = (at(&h, Point { x_nm: 1_524_000, y_nm: -2_540_000 }), at(&h, Point { x_nm: 8_636_000, y_nm: 3_810_000 }));
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track, via]));
    assert!(h.state().views.active.board.selection.is_empty(), "footprints outside the box stay unselected");
}

/// U selects every track joined to the hovered one, and nothing else.
#[test]
fn u_selects_connected_copper() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    let path = [Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 5_080_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 5_080_000 }];
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":path,"expected_revision":revision}));
    h.run();
    let separate = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: -2_540_000 }, Point { x_nm: 7_620_000, y_nm: -2_540_000 });
    let joined: std::collections::BTreeSet<_> = tracks(&h).into_iter().map(|track| track.id).filter(|id| *id != separate).collect();
    assert_eq!(joined.len(), 2);
    h.hover_at(at(&h, Point { x_nm: 3_810_000, y_nm: 2_540_000 }));
    h.run();
    h.key_press(Key::U);
    h.run();
    assert_eq!(h.state().views.active.board.tracks.selected, joined);
}

/// With copper selected and no footprint selected, M then a click moves it by the snapped offset
/// in one edit; dragging a track moves it too, and Esc cancels a move without editing.
#[test]
fn m_and_drag_move_selected_copper() {
    let mut h = routable();
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    let track = add_track(&mut h, start, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let middle = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]));
    let revision = h.state().session.revision();
    h.hover_at(at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 }));
    h.run();
    h.key_press(Key::M);
    h.run();
    h.hover_at(at(&h, Point { x_nm: 5_080_000, y_nm: 3_810_000 }));
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().session.revision(), revision, "Esc cancels the move");
    h.key_press(Key::M);
    h.run();
    let target = at(&h, Point { x_nm: 6_350_000, y_nm: 1_270_000 });
    click_canvas_at(&mut h, target);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1, "one edit per move");
    let moved = tracks(&h).into_iter().find(|t| t.id == track).unwrap();
    let after_m = moved.start;
    assert_ne!(after_m, start, "M moved the track");

    let grab = at(&h, Point { x_nm: (moved.start.x_nm + moved.end.x_nm) / 2, y_nm: moved.start.y_nm });
    let drop = grab + egui::vec2(0., -60.);
    h.hover_at(grab);
    h.run();
    h.drag_at(grab);
    h.run();
    h.hover_at(drop);
    h.run();
    h.drop_at(drop);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 2, "a drag is one more edit");
    assert_ne!(tracks(&h).into_iter().find(|t| t.id == track).unwrap().start, after_m, "the drag moved the track");
}

/// Properties shows a selected track's width in the display units (millimeters here); typing a
/// new width and pressing Enter sets it through the shared command in one edit.
#[test]
fn properties_edit_the_selected_track_width() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let middle = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert!(h.query_by_label("Selected tracks").is_some());
    let revision = h.state().session.revision();
    h.get_by_label("Selected track width").click();
    h.run();
    h.get_by_label("Selected track width").type_text("0.4");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(tracks(&h).into_iter().find(|t| t.id == track).unwrap().width_nm, 400_000);
}

/// Vias of different sizes show their sizes as "Mixed" and offer no field that would overwrite
/// the other dimension; vias of one size stay editable.
#[test]
fn mixed_via_sizes_are_read_only_in_properties() {
    let mut h = routable();
    for (x, pad) in [(2_540_000, 600_000), (5_080_000, 800_000)] {
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::ADD_VIA,
            &json!({"at":{"x_nm":x,"y_nm":-1_270_000},"via_diameter_nm":pad,"via_drill_nm":300_000,"expected_revision":revision}),
        );
    }
    h.run();
    let ids: std::collections::BTreeSet<_> = vias(&h).iter().map(|via| via.id).collect();
    h.state_mut().views.active.board.tracks.selected = ids.clone();
    h.run();
    assert_eq!(h.get_all_by_label("Mixed").count(), 2);
    assert!(h.query_by_label("Selected via pad").is_none() && h.query_by_label("Selected via drill").is_none());
    h.state_mut().views.active.board.tracks.selected = ids.into_iter().take(1).collect();
    h.run();
    assert!(h.query_by_label("Selected via pad").is_some() && h.query_by_label("Mixed").is_none());
}

/// Right-click cancels a copper move without editing, and the ribbon's Move starts one when only
/// copper is selected, anchoring on the selection.
#[test]
fn right_click_cancels_and_ribbon_starts_copper_moves() {
    let mut h = routable();
    let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let middle = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]));
    let revision = h.state().session.revision();
    h.hover_at(middle);
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.tracks.busy());
    h.event(egui::Event::PointerButton { pos: middle, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos: middle, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert!(!h.state().views.active.board.tracks.busy(), "right-click ended the move");
    let elsewhere = at(&h, Point { x_nm: 6_350_000, y_nm: 1_270_000 });
    click_canvas_at(&mut h, elsewhere);
    assert_eq!(h.state().session.revision(), revision, "the next click does not commit a cancelled move");

    click_canvas_at(&mut h, middle);
    assert!(!h.get_by_label("Move footprint").accesskit_node().is_disabled(), "Move is offered for selected copper");
    click(&mut h, "Move footprint");
    assert!(h.state().views.active.board.tracks.busy(), "the ribbon started a copper move");
}
