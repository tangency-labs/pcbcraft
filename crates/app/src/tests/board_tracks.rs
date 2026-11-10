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

/// Inspect › DRC checks the board without editing it and lists its findings; clicking one
/// centers the view there. An edit marks the result out of date until it is run again.
#[test]
fn drc_lists_findings_focuses_them_and_goes_stale_after_edits() {
    let mut h = routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Run design rules check");
    assert_eq!(h.state().session.revision(), revision, "DRC edits nothing");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    assert!(h.query_by_label_contains("No outline · The board has no closed Board Outline").is_some());
    let unconnected = h.get_all_by_label_contains("Unconnected · Unrouted connection").count();
    assert_eq!(unconnected, airwires(&h).len());
    assert!(h.query_by_label_contains("DRC 2 issues").is_some(), "the status bar counts the findings");

    let rect = h.get_by_label("PCB canvas").rect();
    let line = airwires(&h)[0].clone();
    let finding =
        Point { x_nm: line.from.position.x_nm / 2 + line.to.position.x_nm / 2, y_nm: line.from.position.y_nm / 2 + line.to.position.y_nm / 2 };
    let camera_before = h.state().views.active.camera.screen(finding, rect);
    h.get_all_by_label_contains("Unconnected · Unrouted connection").next().unwrap().click();
    h.run();
    assert_ne!(h.state().views.active.camera.screen(finding, rect), camera_before, "the view moved to the finding");

    add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    assert!(h.query_by_label_contains("out of date").is_some());
    click(&mut h, "Run again");
    assert!(h.query_by_label_contains("out of date").is_none());
    let board = h.state().views.active.board.target;
    assert!(!h.state().views.active.board.drc.markers(board).is_empty(), "open findings mark the canvas");
    click(&mut h, "Close");
    assert!(h.query_by_label_contains("Unrouted connection").is_none());
    assert!(h.state().views.active.board.drc.markers(board).is_empty(), "closing the card clears its markers");
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

/// Design › Outputs explains a missing outline, then saves one deterministic archive of Gerber
/// and drill files without editing the project. A subsequent cancelled export clears the old
/// success notice without replacing it with another message.
#[test]
fn outputs_export_one_archive_of_fabrication_files() {
    let mut h = routable();
    click(&mut h, "Generate fabrication outputs");
    assert!(h.state().message.as_deref().is_some_and(|m| m.contains("closed Board Outline")), "{:?}", h.state().message);
    let outline = layer_named(&h, "Board Outline");
    let revision = h.state().session.revision();
    let p = |x: i64, y: i64| json!({"x_nm":x,"y_nm":y});
    h.state_mut().run(
        pcbcraft_engine::ADD_DRAWING,
        &json!({"layer":outline,"points":[p(-5_000_000,-5_000_000),p(15_000_000,-5_000_000),p(15_000_000,5_000_000),p(-5_000_000,5_000_000)],"closed":true,"expected_revision":revision}),
    );
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Generate fabrication outputs");
    assert_eq!(h.state().message, None);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "exporting edits nothing");
    let (name, bytes) = h.state().fabrication_exports.captured.clone().expect("an archive was handed to the save worker");
    assert!(name.ends_with("-fabrication.zip"), "{name}");
    let outputs = h.state().session.export_fabrication(&json!({})).unwrap();
    assert!(outputs.iter().any(|o| o.name.ends_with("-Top.gbr")) && outputs.iter().any(|o| o.name.ends_with("-PTH.drl")));
    assert_eq!(bytes, pcbcraft_engine::board::fabrication::zip(&outputs).unwrap(), "the saved archive is the engine's deterministic one");
    assert!(h.state().fabrication_exports.busy());
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::Saved)).unwrap();
    h.run();
    assert!(!h.state().fabrication_exports.busy());
    assert!(h.query_by_label("Saved fabrication outputs").is_some(), "the board status bar shows file results");
    click(&mut h, "Generate fabrication outputs");
    assert!(h.state().fabrication_exports.busy());
    assert!(h.state().file_notice.is_none(), "a new attempt clears the previous success notice");
    assert!(h.query_by_label("Saved fabrication outputs").is_none());
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::Cancelled)).unwrap();
    h.run();
    assert!(!h.state().fabrication_exports.busy());
    assert!(h.state().file_notice.is_none() && h.state().message.is_none(), "cancellation is quiet");
    assert!(h.query_by_label("Saved fabrication outputs").is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "cancelling edits nothing");
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

/// Design › Flip view mirrors this view left-to-right around the centered point, keeps the zoom,
/// and maps pointer picks back to the same model points, so clicks still select what they hit.
#[test]
fn flip_view_mirrors_drawing_and_picking_without_editing() {
    let mut h = routable();
    let left = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    let right = Point { x_nm: 7_620_000, y_nm: 2_540_000 };
    let track = add_track(&mut h, left, right);
    let revision = h.state().session.revision();
    let (left_before, right_before) = (at(&h, left), at(&h, right));
    assert!(left_before.x < right_before.x);
    click(&mut h, "View the board from below");
    assert!(h.state().views.active.camera.mirrored());
    let (left_after, right_after) = (at(&h, left), at(&h, right));
    assert!(left_after.x > right_after.x, "model X now runs right to left");
    assert!(((right_before - left_before).length() - (left_after - right_after).length()).abs() < 0.01, "the zoom is unchanged");
    assert!(h.query_by_label_contains("Bottom view").is_some());
    h.state_mut().message = Some("Test board error".into());
    h.run();
    assert!(h.query_by_label("Test board error").is_some());
    assert!(h.query_by_label("Bottom view").is_some(), "an error does not hide the mirrored orientation");
    h.state_mut().message = None;
    h.run();
    let middle = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "picking follows the mirror");
    click(&mut h, "View the board from below");
    assert!(!h.state().views.active.camera.mirrored());
    assert!((at(&h, left) - left_before).length() < 0.5, "flipping back restores the view");
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    h.state_mut().run_window_command(&ctx, "view.set_board_flipped", &json!({"view":view,"flipped":true})).unwrap();
    assert!(h.state().views.active.camera.mirrored(), "live MCP flips through the same handler");
    assert!(h.state_mut().run_window_command(&ctx, "view.set_board_flipped", &json!({"view":999,"flipped":true})).is_err());
    assert_eq!(h.state().session.revision(), revision, "flipping never edits the project");
}

/// Ctrl+Shift+M measures between snapped pad centers in either view without selecting or editing;
/// Esc clears the ruler, then leaves the tool, while flipped orientation remains visible.
#[test]
fn measure_reports_the_distance_between_snapped_points() {
    for flipped in [false, true] {
        let mut h = routable();
        h.state_mut().views.active.camera.set_mirrored(flipped);
        h.run();
        h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
        let line = airwires(&h)[0].clone();
        let (a, b) = (line.from.position, line.to.position);
        let revision = h.state().session.revision();
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::M);
        h.run();
        assert!(h.state().views.active.board.measure.tool);
        assert_eq!(h.query_by_label("Bottom view").is_some(), flipped);
        let near_a = at(&h, a) + egui::vec2(2., 1.);
        click_canvas_at(&mut h, near_a);
        h.run_steps(20);
        let near_b = at(&h, b) + egui::vec2(-2., 1.);
        click_canvas_at(&mut h, near_b);
        h.run_steps(20);
        let expected = crate::board_measure::label(a, b, pcbcraft_chrome::units::Unit::Mm);
        let painted = h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == expected));
        assert!(painted, "the ruler shows {expected}");
        assert!(h.state().views.active.board.selection.is_empty() && h.state().views.active.board.tracks.selected.is_empty());
        assert_eq!(h.state().session.revision(), revision, "measuring edits nothing");
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.active.board.measure.tool, "the first Esc clears the ruler");
        h.key_press(Key::Escape);
        h.run();
        assert!(!h.state().views.active.board.measure.tool);
    }
}

/// A ruler owns canvas input without changing an existing mixed selection. Delete, Cut, Flip and
/// the shared delete action cannot mutate its footprints or copper until measurement is left.
#[test]
fn measure_preserves_mixed_selection_against_delete_and_cut() {
    let mut h = routable();
    add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    h.state_mut().select_all_board();
    h.run();
    assert!(!h.state().views.active.board.selection.is_empty());
    assert!(!h.state().views.active.board.tracks.selected.is_empty());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Measure distance");
    h.key_press(Key::Delete);
    h.run();
    h.event(egui::Event::Cut);
    h.run();
    h.key_press(Key::F);
    h.run();
    h.state_mut().delete_board_items();
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    assert!(!h.state().views.active.board.selection.is_empty());
    assert!(!h.state().views.active.board.tracks.selected.is_empty());
    assert!(h.state().views.active.board.measure.tool);
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.measure.tool);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// F flips the hovered footprint to the bottom in one edit, Properties shows its side, the
/// ribbon's Flip turns it back, and F during an M move flips the draft committed with the move.
#[test]
fn f_flips_footprints_between_sides() {
    let mut h = routable();
    let (id, position) = {
        let project = h.state().session.project().unwrap();
        let f = project.boards().next().unwrap().footprints().next().unwrap();
        (f.id.clone(), f.position)
    };
    let bottom =
        |h: &Harness<'_, PcbApp>| h.state().session.project().unwrap().boards().next().unwrap().footprints().find(|f| f.id == id).unwrap().bottom;
    let revision = h.state().session.revision();
    h.hover_at(at(&h, position));
    h.run();
    h.key_press(Key::F);
    h.run();
    assert!(bottom(&h), "F flipped the hovered footprint");
    assert_eq!(h.state().session.revision(), revision + 1);
    let center = at(&h, position);
    click_canvas_at(&mut h, center);
    assert!(h.query_by_label("Bottom").is_some(), "Properties shows the side");
    click(&mut h, "Flip footprint to the other side");
    assert!(!bottom(&h), "the ribbon flips it back");
    assert_eq!(h.state().session.revision(), revision + 2);

    h.hover_at(at(&h, position));
    h.run();
    h.key_press(Key::M);
    h.run();
    h.key_press(Key::F);
    h.run();
    assert_eq!(h.state().session.revision(), revision + 2, "a flipped draft is not an edit yet");
    let target = at(&h, Point { x_nm: position.x_nm + 1_270_000, y_nm: position.y_nm });
    click_canvas_at(&mut h, target);
    assert_eq!(h.state().session.revision(), revision + 3, "the move and the flip are one edit");
    assert!(bottom(&h));
}

/// Properties edits a selected footprint's X, Y, rotation and side exactly, each as one
/// transform edit that keeps the other values.
#[test]
fn properties_set_exact_footprint_position_rotation_and_side() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let (id, position) = {
        let project = h.state().session.project().unwrap();
        let f = project.boards().next().unwrap().footprints().next().unwrap();
        (f.id.clone(), f.position)
    };
    let placed = |h: &Harness<'_, PcbApp>| {
        let project = h.state().session.project().unwrap();
        let f = project.boards().next().unwrap().footprints().find(|f| f.id == id).unwrap();
        (f.position, f.rotation, f.bottom)
    };
    let center = at(&h, position);
    click_canvas_at(&mut h, center);
    let revision = h.state().session.revision();
    let set = |h: &mut Harness<'_, PcbApp>, label: &str, text: &str| {
        h.get_by_label(label).click();
        h.run();
        h.get_by_label(label).type_text(text);
        h.run();
        h.key_press(Key::Enter);
        h.run();
    };
    set(&mut h, "Footprint X", "12.7");
    assert_eq!(placed(&h), (Point { x_nm: 12_700_000, y_nm: position.y_nm }, 0, false));
    set(&mut h, "Footprint Y", "-2.54");
    assert_eq!(placed(&h).0, Point { x_nm: 12_700_000, y_nm: -2_540_000 });
    set(&mut h, "Footprint rotation", "90");
    assert_eq!(placed(&h).1, 90);
    click(&mut h, "Bottom");
    assert_eq!(placed(&h), (Point { x_nm: 12_700_000, y_nm: -2_540_000 }, 90, true));
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 4, "one edit per field");

    // Rotations wrap into 0–359°, and coordinates past the board range clamp to its edge.
    set(&mut h, "Footprint rotation", "-90");
    assert_eq!(placed(&h).1, 270);
    set(&mut h, "Footprint rotation", "360");
    assert_eq!(placed(&h).1, 0);
    set(&mut h, "Footprint X", "5000");
    assert_eq!(placed(&h).0.x_nm, pcbcraft_engine::schematic::MAX_CENTER_NM);
    set(&mut h, "Footprint Y", "-5000");
    assert_eq!(placed(&h).0.y_nm, -pcbcraft_engine::schematic::MAX_CENTER_NM);
    assert_eq!(h.state().message, None);
}

/// Mid-route, PgDn drops a via and continues on back copper and PgUp returns to the front; a key
/// for the side the route is already on adds nothing. Outside a route they only change layer.
#[test]
fn page_keys_place_vias_mid_route() {
    let mut h = routable();
    let (top, bottom) = (layer_named(&h, "Top"), layer_named(&h, "Bottom"));
    h.key_press(Key::X);
    h.run();
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    let corner = Point { x_nm: 5_080_000, y_nm: 2_540_000 };
    click_route(&mut h, start);
    h.hover_at(at(&h, corner));
    h.run();
    h.key_press(Key::PageDown);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()));
    let next = Point { x_nm: 5_080_000, y_nm: -1_270_000 };
    click_route(&mut h, next);
    h.hover_at(at(&h, next));
    h.run();
    h.key_press(Key::PageUp);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    h.key_press(Key::PageUp);
    h.run();
    click_route(&mut h, Point { x_nm: 7_620_000, y_nm: -1_270_000 });
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().message, None);
    assert_eq!(vias(&h).len(), 2, "one via per side change, none for the repeated PgUp");
    assert!(tracks(&h).iter().any(|t| t.layer.to_string() == bottom));
    h.key_press(Key::Escape);
    h.run();
    h.key_press(Key::PageDown);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()), "outside a route PgDn only changes layer");
    assert_eq!(vias(&h).len(), 2);
}

/// Backtick over a pad highlights its net, over routed copper the net that copper carries, and over
/// empty space clears the highlight; nothing is edited.
#[test]
fn backtick_highlights_the_net_under_the_pointer() {
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let (a, b) = (line.from.position, line.to.position);
    let net =
        h.state().session.project().unwrap().boards().next().unwrap().footprints().flat_map(|f| f.pads).find(|p| p.center == a).unwrap().net.unwrap();
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    let path = [a, Point { x_nm: a.x_nm, y_nm: -2_540_000 }, Point { x_nm: b.x_nm, y_nm: -2_540_000 }, b];
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":path,"expected_revision":revision}));
    h.run();
    let revision = h.state().session.revision();
    h.hover_at(at(&h, a));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net.as_deref(), Some(net.as_str()));
    assert!(h.query_by_label_contains(&format!("Net {net} highlighted")).is_some());
    let highlight = pcbcraft_chrome::Tokens::get().board_highlight;
    let outlined = h.output().shapes.iter().filter(|shape| match &shape.shape {
        egui::Shape::Path(path) => path.stroke.color == egui::epaint::ColorMode::Solid(highlight),
        _ => false,
    });
    assert!(outlined.count() >= 2, "both pads and the route are outlined");
    h.hover_at(at(&h, Point { x_nm: (a.x_nm + b.x_nm) / 2, y_nm: 1_270_000 }));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, None, "clear the pad highlight before testing track picking");
    h.hover_at(at(&h, Point { x_nm: (a.x_nm + b.x_nm) / 2, y_nm: -2_540_000 }));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net.as_deref(), Some(net.as_str()), "the track carries the net");
    h.hover_at(at(&h, Point { x_nm: (a.x_nm + b.x_nm) / 2, y_nm: 1_270_000 }));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, None);
    assert_eq!(h.state().session.revision(), revision);
}

/// Net overlays hide surface pads and tracks with their layers while retaining visible-layer
/// copper and the ordinary renderer's layer-independent via policy.
#[test]
fn net_highlight_respects_copper_layer_visibility() {
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let a = line.from.position;
    let c = Point { x_nm: a.x_nm, y_nm: a.y_nm - 2_540_000 };
    let d = Point { x_nm: line.to.position.x_nm, y_nm: c.y_nm };
    let top = layer_named(&h, "Top");
    let bottom = layer_named(&h, "Bottom");
    for (layer, path) in [(&top, [a, c]), (&bottom, [c, d])] {
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":layer,"width_nm":250_000,"points":path,"expected_revision":revision}));
    }
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_VIA, &json!({"at":c,"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
    h.hover_at(at(&h, a));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    let revision = h.state().session.revision();
    let highlighted = h.state().views.active.board.highlight_net.clone();
    assert!(highlighted.is_some());
    let outlines = |h: &Harness<'_, PcbApp>| {
        let highlight = pcbcraft_chrome::Tokens::get().board_highlight;
        let canvas = h.get_by_label("PCB canvas").rect();
        h.output().shapes.iter().filter(|shape| shape.clip_rect == canvas).fold((0, 0), |(paths, vias), shape| match &shape.shape {
            egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(highlight) => (paths + 1, vias),
            egui::Shape::Circle(circle) if circle.stroke.color == highlight => (paths, vias + 1),
            _ => (paths, vias),
        })
    };
    assert_eq!(outlines(&h), (4, 1), "two pads, both tracks and the via");
    h.state_mut().views.active.board.set_layer(&top, false, Some(false));
    h.run();
    assert_eq!(outlines(&h), (1, 1), "only the bottom track and via remain visible");
    h.state_mut().views.active.board.set_layer(&bottom, false, Some(false));
    h.run();
    assert_eq!(outlines(&h), (0, 1), "vias retain the normal renderer's policy even with all copper layers hidden");
    h.state_mut().views.active.board.set_layer(&top, false, Some(true));
    h.run();
    assert_eq!(outlines(&h), (3, 1), "restoring Top restores its pads and track");
    assert_eq!(h.state().views.active.board.highlight_net, highlighted);
    assert_eq!(h.state().session.revision(), revision);
}

/// Backtick over a hidden surface pad clears the highlight; showing its layer makes it pickable.
#[test]
fn net_highlight_does_not_pick_hidden_pads() {
    let mut h = routable();
    let a = airwires(&h)[0].from.position;
    let top = layer_named(&h, "Top");
    h.hover_at(at(&h, a));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    let net = h.state().views.active.board.highlight_net.clone();
    assert!(net.is_some());
    h.state_mut().views.active.board.set_layer(&top, false, Some(false));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, None);
    h.state_mut().views.active.board.set_layer(&top, false, Some(true));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, net);
}

/// Physical pads on an unconfigured board remain pickable without inventing layer identities.
#[test]
fn net_highlight_keeps_unconfigured_pads_pickable() {
    let mut h = routable();
    h.state_mut().run("edit.undo", &json!({})); // Accepted baseline.
    h.state_mut().run("edit.undo", &json!({})); // Initial board setup.
    h.run();
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    assert!(board.setup().is_none());
    let pad = board.footprints().flat_map(|f| f.pads).find(|pad| pad.net.is_some()).unwrap();
    assert!(matches!(pad.copper_layers, Some(pcbcraft_engine::board::view::CopperLayers::Unconfigured { .. })));
    assert!(pad.copper.is_some());
    h.hover_at(at(&h, pad.center));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, pad.net);
}

/// A highlighted net that disappears from the board clears instead of staying named.
#[test]
fn a_vanished_highlight_net_clears() {
    let mut h = routable();
    h.state_mut().views.active.board.highlight_net = Some("NO SUCH NET".into());
    let app = h.state_mut();
    app.views.sync(&app.session);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net, None);
    assert!(h.query_by_label_contains("Net NO SUCH NET highlighted").is_none());
}

/// Connectivity is shared across unchanged frames and navigation, refreshed after edits and undo,
/// and never reused without input authority or across a project reopening.
#[test]
fn copper_net_cache_reuses_frames_and_refreshes_after_changes() {
    /// Borrow a snapshot independently of the UI state so the cache can retain only derived data.
    fn cached(h: &mut Harness<'_, PcbApp>) -> std::sync::Arc<std::collections::BTreeMap<uuid::Uuid, String>> {
        let source = h.state().views.active.input(&h.state().session);
        let project = h.state().session.snapshot().unwrap();
        let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
        h.state_mut().views.active.board.copper_nets(board, source)
    }
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let (a, b) = (line.from.position, line.to.position);
    let path = [a, Point { x_nm: a.x_nm, y_nm: -2_540_000 }, Point { x_nm: b.x_nm, y_nm: -2_540_000 }, b];
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":path,"expected_revision":revision}));
    h.run();
    let saved = h.state().session.snapshot().unwrap();
    let first = cached(&mut h);
    assert_eq!(first.len(), tracks(&h).len());
    assert!(!first.is_empty());
    h.run();
    assert!(std::sync::Arc::ptr_eq(&first, &cached(&mut h)));
    h.state_mut().views.active.camera.center_on(a);
    h.run();
    assert!(std::sync::Arc::ptr_eq(&first, &cached(&mut h)), "navigation keeps the cache");

    let ids: Vec<_> = tracks(&h).iter().map(|track| track.id).collect();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DELETE_TRACKS, &json!({"tracks":ids,"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
    let edited = cached(&mut h);
    assert!(!std::sync::Arc::ptr_eq(&first, &edited));
    assert!(edited.is_empty(), "deleted copper cannot keep stale net assignments");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    let undone = cached(&mut h);
    assert_eq!(undone, first);
    assert!(!std::sync::Arc::ptr_eq(&first, &undone), "undo has a new revision");

    let project = h.state().session.snapshot().unwrap();
    let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
    let uncached = h.state_mut().views.active.board.copper_nets(board, None);
    let another = h.state_mut().views.active.board.copper_nets(board, None);
    assert_eq!(uncached, first);
    assert!(!std::sync::Arc::ptr_eq(&uncached, &another), "missing authority never caches a preview");
    let before_open = cached(&mut h);
    h.state_mut().session.open_snapshot(saved);
    h.run();
    let reopened = cached(&mut h);
    assert_eq!(reopened, first);
    assert!(!std::sync::Arc::ptr_eq(&before_open, &reopened));
}

/// Modify › Align to Top aligns the selected footprints in one edit; Distribute needs three.
#[test]
fn modify_menu_aligns_selected_footprints() {
    let mut h = routable();
    let ids: std::collections::BTreeSet<String> =
        h.state().session.project().unwrap().boards().next().unwrap().footprints().map(|f| f.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    let first = ids.iter().next().unwrap().clone();
    let (at, rotation) = {
        let project = h.state().session.project().unwrap();
        let f = project.boards().next().unwrap().footprints().find(|f| f.id == first).unwrap();
        (f.position, f.rotation)
    };
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":first,"x_nm":at.x_nm,"y_nm":at.y_nm + 2_540_000,"rotation":rotation,"expected_revision":revision}),
    );
    h.run();
    h.state_mut().views.active.board.selection = ids.clone();
    h.run();
    let revision = h.state().session.revision();
    click(&mut h, "Modify menu");
    assert!(h.get_by_label("Distribute Horizontally").accesskit_node().is_disabled(), "distributing needs three footprints");
    click(&mut h, "Align to Top");
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let ys: Vec<_> = h.state().session.project().unwrap().boards().next().unwrap().footprints().map(|f| f.position.y_nm).collect();
    assert_eq!(ys[0], ys[1], "identical footprints aligned by their tops share a Y");
}
