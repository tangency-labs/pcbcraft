//! Track routing exercises real canvas input and the same transactions used by MCP.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL,
    board::{ratsnest::Airwire, track::Track, update::INITIALIZE_BASELINE},
    persistence::encode_documents,
    schematic::Point,
};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Sweeping into fixed copper keeps a visible legal ghost; exiting it restores a detour.
#[test]
fn walkaround_preview_does_not_disappear_inside_an_obstacle() {
    let mut h = routable();
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS,&json!({"layer":layer,"width_nm":500_000,"points":[{"x_nm":4_000_000,"y_nm":-7_620_000},{"x_nm":4_000_000,"y_nm":-2_540_000}],"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, Point { x_nm: 1_270_000, y_nm: -5_080_000 });
    let tint = pcbcraft_chrome::Tokens::get().board_front.lerp_to_gamma(egui::Color32::WHITE, 0.35);
    for x_nm in [2_540_000, 3_810_000, 4_000_000, 4_190_000, 7_620_000, 4_000_000, 2_540_000] {
        h.hover_at(at(&h, Point { x_nm, y_nm: -5_080_000 }));
        h.run_steps(200);
        assert!(
            h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::LineSegment{stroke,..} if stroke.color==tint)),
            "missing preview at x={x_nm}, status={:?}",
            h.state().views.active.board.tracks.preparation_status()
        );
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The real route tool detours around fixed, locked copper and publishes one native transaction.
#[test]
fn interactive_walkaround_avoids_fixed_via_and_round_trips() {
    let mut h = routable();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":4_000_000,"y_nm":-5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    let via = h.state().session.project().unwrap().boards().next().unwrap().vias()[0].id;
    let revision = h.state().session.revision();
    let target = h.state().session.project().unwrap().board_target().unwrap().board;
    h.state_mut()
        .run(pcbcraft_engine::board::track::SET_COPPER_LOCKED, &json!({"board":target,"vias":[via],"locked":true,"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, Point { x_nm: 1_270_000, y_nm: -5_080_000 });
    click_route(&mut h, Point { x_nm: 7_620_000, y_nm: -5_080_000 });
    h.key_press(Key::Enter);
    h.run_steps(30);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let routed = tracks(&h);
    assert!(routed.len() > 1, "the straight leg intersects the fixed via");
    assert!(routed.iter().any(|t| t.start.y_nm != -5_080_000 || t.end.y_nm != -5_080_000));
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    assert_eq!(board.vias()[0].at, Point { x_nm: 4_000_000, y_nm: -5_080_000 });
    assert!(board.vias()[0].locked);
    let report = board.check_design(board.effective_rules().values);
    assert!(
        !report.violations.iter().any(|v| matches!(v.kind, pcbcraft_engine::board::drc::Kind::Clearance | pcbcraft_engine::board::drc::Kind::Short))
    );
    let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let packed = pcbcraft_engine::persistence::encode_packed(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(pcbcraft_engine::persistence::decode_packed(&packed).unwrap().project).unwrap(), after);
    h.state_mut().run("edit.undo", &json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.state_mut().run("edit.redo", &json!({}));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), after);
}

/// A required class clearance that leaves no legal escape blocks the click without mutation.
#[test]
fn walkaround_refuses_an_impossible_pad_escape_without_publishing() {
    let mut h = routable();
    classify_route(&mut h, "Tight escape", json!({"clearance_nm":700_000,"min_track_width_nm":400_000,"preferred_track_width_nm":500_000}));
    let line = airwires(&h)[0].clone();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    click_route(&mut h, Point { x_nm: line.from.position.x_nm, y_nm: -2_540_000 });
    assert!(h.state().message.as_deref().is_some_and(|message| message.contains("blocked")));
    h.key_press(Key::Enter);
    h.run_steps(20);
    assert!(tracks(&h).is_empty());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A route from an unconnected pad cannot acquire a named net through overlap, either
/// directly or after fixing a free vertex. Rejected clicks leave copper and history intact.
#[test]
fn an_unconnected_pad_route_cannot_join_a_named_terminal() {
    for vertex in [false, true] {
        let mut h = routable();
        let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap();
        let unknown = footprint.pads.iter().find(|pad| pad.net.is_none()).unwrap().center;
        let named = footprint.pads.iter().find(|pad| pad.net.is_some()).unwrap().center;
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, unknown);
        if vertex {
            click_route(&mut h, Point { x_nm: unknown.x_nm, y_nm: unknown.y_nm - 2_540_000 });
            assert_eq!(h.state().message, None);
        }
        click_route(&mut h, named);
        h.run_steps(200);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.revision(), revision, "a clamped click fixes a vertex without auto-joining or auto-finishing");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press(Key::Enter);
        h.run_steps(200);
        assert_eq!(h.state().message, None);
        assert!(
            h.state().session.project().unwrap().boards().next().unwrap().copper_nets().expect("arc-free test board has a copper net map").is_empty()
        );
        assert_eq!(h.state().session.revision(), revision + 1);
        let target = h.state().views.active.board.target.unwrap();
        let query = h.state().session.clearance_query_with_rules(target, &json!({})).unwrap();
        let named_pad = query.objects().iter().find(|o| o.at == named && !o.intended_nets.is_empty()).unwrap();
        for track in tracks(&h) {
            let id = track.id.to_string();
            let object = query.objects().iter().find(|o| o.name == id).unwrap();
            assert!(object.intended_nets.is_empty(), "unknown pad route adopted the named net");
            assert!(object.contour.gap(&named_pad.contour) >= 200_000., "clamped endpoint broke foreign-pad clearance");
        }
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press(Key::Escape);
        h.run();
        assert!(tracks(&h).is_empty());
    }
}

/// Two 0402 resistors whose first terminals share an accepted net, on a two-layer board,
/// opened in the PCB layout with the Top copper layer active.
pub(super) fn routable<'a>() -> Harness<'a, PcbApp> {
    routable_with("resistor-0402")
}

/// Select a physical package with enough pad spacing for the fixture's authored class rules.
fn routable_with<'a>(definition: &str) -> Harness<'a, PcbApp> {
    routable_spaced(definition, 10_160_000)
}

/// Resolve the same command-created board at a chosen footprint pitch for routing channels.
fn routable_spaced<'a>(definition: &str, pitch_nm: i64) -> Harness<'a, PcbApp> {
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
        h.state_mut().run(
            PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":definition,"mapping":{"1":"2","2":"1"},"x_nm":index*pitch_nm,"y_nm":0,"expected_revision":revision}),
        );
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
    click(&mut h, "PCB Layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    click(&mut h, "Activate layer Top");
    h
}

/// Unsupported curved copper is visibly refused; opening and cancelling Board Setup still preserves the board.
#[test]
fn circular_copper_keeps_native_bytes_and_explains_the_unavailable_canvas() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    let board = h.state().session.project().unwrap().board_target().unwrap().board;
    h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board,"arcs":[{
        "id":"00000000-0000-0000-0000-000000000001","layer":top,"width_nm":250000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5000000,"y_nm":0},"end":{"x_nm":0,"y_nm":5000000},"direction":"clockwise"}],"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.run();
    assert!(h.query_all_by_label(pcbcraft_engine::board::arc::UNSUPPORTED).next().is_some());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    let revision = h.state().session.revision();
    click(&mut h, "Board setup…");
    assert!(matches!(h.state().dialog, Dialog::BoardSetup(_)));
    click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert!(h.state().session.project().unwrap().boards().next().unwrap().arc_tracks().is_empty());
}

/// The canvas names the teardrop recovery command and returns after the last intent is removed.
#[test]
fn teardrop_copper_explains_how_to_resume_board_editing() {
    use pcbcraft_engine::{
        ADD_DRAWING, ADD_TRACKS, ADD_VIA,
        board::{setup::LayerPurpose, teardrop::EDIT_TEARDROPS},
    };

    let mut h = routable();
    let top = layer_named(&h, "Top");
    let session = &mut h.state_mut().session;
    let target = session.project().unwrap().board_target().unwrap();
    let outline =
        session.project().unwrap().board_view(target).unwrap().layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let at = Point { x_nm: 0, y_nm: 20_000_000 };
    let end = Point { x_nm: 600_000, y_nm: 20_000_000 };
    session.apply(ADD_VIA, &json!({"at":at,"via_diameter_nm":500_000,"via_drill_nm":300_000,"expected_revision":session.revision()})).unwrap();
    session.apply(ADD_TRACKS, &json!({"layer":top,"width_nm":100_000,"points":[at,end],"expected_revision":session.revision()})).unwrap();
    let corners = [(-5_000_000, -5_000_000), (25_000_000, -5_000_000), (25_000_000, 45_000_000), (-5_000_000, 45_000_000)]
        .map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    session
        .apply(ADD_DRAWING, &json!({"board":target.board,"layer":outline,"points":corners,"closed":true,"expected_revision":session.revision()}))
        .unwrap();
    let board = session.project().unwrap().board_view(target).unwrap();
    let (via, track) = (board.vias()[0].id, board.tracks()[0].id);
    let id = Uuid::from_u128(1);
    let report = session
        .execute(
            EDIT_TEARDROPS,
            &json!({"board":target.board,"expected_revision":session.revision(),"edits":{"add":[{
                "id":id,"via":via,"track":track,"parameters":{"length":{"percent":20},"width_pct":80}
            }],"remove":[]}}),
        )
        .unwrap();
    assert_eq!(report["teardrops"]["added"].as_array().unwrap().len(), 1, "{report}");

    let notice = "Remove the saved teardrop intents with pcb.edit_teardrops to resume board editing.";
    h.run();
    h.get_by_label(notice);
    assert!(h.query_all_by_label("PCB canvas").next().is_none());
    assert!(
        h.query_all_by_label("Exact arc records are preserved. Board display, picking and edits are unavailable until circular copper is supported.")
            .next()
            .is_none()
    );

    let session = &mut h.state_mut().session;
    session.execute(EDIT_TEARDROPS, &json!({"board":target.board,"expected_revision":session.revision(),"edits":{"add":[],"remove":[id]}})).unwrap();
    h.run();
    assert!(h.query_all_by_label(notice).next().is_none());
    h.get_by_label("PCB canvas");
}

/// Browser selection remains view-local on arc-bearing boards even though the canvas is unavailable.
#[test]
fn circular_copper_keeps_browser_footprint_picks_available() {
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        let mut h = routable();
        h.state_mut().preferences.theme = theme;
        h.state_mut().browser.open = true;
        let top = layer_named(&h, "Top");
        let revision = h.state().session.revision();
        let board = h.state().session.project().unwrap().board_target().unwrap().board;
        h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board,"arcs":[{
            "id":"00000000-0000-0000-0000-000000000001","layer":top,"width_nm":250000,
            "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5000000,"y_nm":0},"end":{"x_nm":0,"y_nm":5000000},"direction":"clockwise"}],"expected_revision":revision}));
        assert_eq!(h.state().message, None);
        let footprints: Vec<_> =
            h.state().session.project().unwrap().boards().next().unwrap().footprints().map(|footprint| footprint.id.clone()).collect();
        let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.run();
        for (label, footprint) in ["Footprint R1  R", "Footprint R2  R"].into_iter().zip(footprints) {
            click(&mut h, label);
            assert_eq!(h.state().views.active.board.selection, BTreeSet::from([footprint]));
            assert!(h.query_all_by_label(pcbcraft_engine::board::arc::UNSUPPORTED).next().is_some());
            assert_eq!(h.state().session.revision(), revision);
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        }
    }
}

/// A Delete pressed while measurement owns the arc-bearing canvas must not delete a
/// previously selected object when the measurement ends on the first linear frame.
fn arc_board_measure_does_not_replay_delete(zone: bool) {
    let mut h = routable();
    let board_target = h.state().session.project().unwrap().board_target().unwrap();
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    let project = h.state().session.project().unwrap();
    let outline = project
        .board_view(board_target)
        .unwrap()
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline)
        .unwrap()
        .id
        .to_string();
    let net = project.inspect_net_registry().unwrap()["nets"][0]["id"].clone();
    let mut params = json!({"board":board_target.board,"layer":if zone {&top} else {&outline},"points":[
        {"x_nm":-5_080_000,"y_nm":5_080_000},{"x_nm":5_080_000,"y_nm":5_080_000},{"x_nm":5_080_000,"y_nm":10_160_000}],"expected_revision":revision});
    if zone {
        params["net"] = net;
    } else {
        params["closed"] = json!(true);
    }
    h.state_mut().run(if zone { pcbcraft_engine::board::zones::ADD_ZONE } else { pcbcraft_engine::ADD_DRAWING }, &params);
    assert_eq!(h.state().message, None);
    h.run();
    let board = h.state().session.project().unwrap().board_view(board_target).unwrap();
    let mut items = pcbcraft_engine::board::clipboard::BoardSelection::default();
    if zone {
        items.zones.insert(board.zones()[0].id);
    } else {
        items.drawings.insert(board.drawings()[0].id);
    }
    h.state_mut().select_board_items(items);
    h.state_mut().start_measure();
    h.run();
    assert!(h.state().views.active.board.measure.tool);

    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board_target.board,"arcs":[{
        "id":"00000000-0000-0000-0000-000000000001","layer":top,"width_nm":250000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5000000,"y_nm":0},"end":{"x_nm":0,"y_nm":5000000},"direction":"clockwise"}],"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
    assert!(h.query_all_by_label(pcbcraft_engine::board::arc::UNSUPPORTED).next().is_some());
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().views.active.board.measure.tool);
    assert_eq!(h.state().session.revision(), revision + 1);

    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board_target.board,"arcs":[],"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.measure.tool);
    assert_eq!(h.state().session.revision(), revision, "leaving measurement must not replay Delete");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

#[test]
fn unsupported_copper_does_not_replay_zone_delete_after_measurement() {
    arc_board_measure_does_not_replay_delete(true);
}

#[test]
fn unsupported_copper_does_not_replay_drawing_delete_after_measurement() {
    arc_board_measure_does_not_replay_delete(false);
}

pub(super) fn tracks(h: &Harness<'_, PcbApp>) -> Vec<Track> {
    h.state().session.project().unwrap().boards().next().unwrap().tracks().to_vec()
}

/// Find a preview segment of the given copper color ending at the expected screen point.
fn preview_ends_at(h: &Harness<'_, PcbApp>, color: egui::Color32, end: egui::Pos2) -> bool {
    let tint = color.lerp_to_gamma(egui::Color32::WHITE, 0.35);
    h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
        if stroke.color == tint && points[1].distance(end) < 0.001)
    })
}

/// Warm, clear pointer targets must paint in their input frame while authored copper stays fixed.
#[test]
fn warm_walkaround_preview_follows_continuous_pointer_input() {
    let mut h = routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, Point { x_nm: 0, y_nm: -5_080_000 });
    h.hover_at(at(&h, Point { x_nm: 1_270_000, y_nm: -5_080_000 }));
    h.run_steps(30);
    for x_nm in [2_540_000, 3_810_000, 5_080_000, 6_350_000, 7_620_000] {
        let end = at(&h, Point { x_nm, y_nm: -5_080_000 });
        h.hover_at(end);
        h.step();
        assert!(preview_ends_at(&h, pcbcraft_chrome::Tokens::get().board_front, end), "warm preview trails pointer at x={x_nm}");
    }
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Pointer changes invalidate prepared candidates, but the draft keeps its copper color both
/// during background preparation and after completion, on either copper side and in both themes.
#[test]
fn route_preview_color_does_not_flash_while_preparation_catches_up() {
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        for layer in ["Top", "Bottom"] {
            let mut h = routable();
            h.state_mut().preferences.theme = theme;
            h.run();
            click(&mut h, &format!("Activate layer {layer}"));
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            h.key_press(Key::X);
            h.run();
            click_route(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 });
            for x_nm in [3_810_000, 5_080_000, 6_350_000] {
                let end = at(&h, Point { x_nm, y_nm: 2_540_000 });
                let color = if layer == "Top" { pcbcraft_chrome::Tokens::get().board_front } else { pcbcraft_chrome::Tokens::get().board_back };
                h.hover_at(end);
                h.step();
                assert_ne!(h.state().views.active.board.tracks.preparation_status().as_deref(), Some("Route prepared"));
                // A new pointer keeps the previous legal path while its target is solved.
                // The first request has no certified leg to paint yet.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while h.state().views.active.board.tracks.preparation_status().as_deref() != Some("Route prepared") {
                    assert!(std::time::Instant::now() < deadline, "route preparation did not settle");
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                assert!(preview_ends_at(&h, color, end), "ready {theme:?} {layer}");
            }
            h.key_press(Key::Escape);
            h.run();
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// Whether the real canvas drew the specified track stroke in its effective color.
fn track_has_color(h: &Harness<'_, PcbApp>, [a, b]: [Point; 2], color: egui::Color32) -> bool {
    let [a, b] = [at(h, a), at(h, b)];
    h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
        if stroke.color == color && points[0].distance(a) < 0.001 && points[1].distance(b) < 0.001)
    })
}

fn airwires(h: &Harness<'_, PcbApp>) -> Vec<Airwire> {
    h.state().session.project().unwrap().boards().next().unwrap().ratsnest().unwrap().lines
}

pub(super) fn at(h: &Harness<'_, PcbApp>, point: Point) -> egui::Pos2 {
    h.state().views.active.camera.screen(point, h.get_by_label("PCB canvas").rect())
}

/// Click, then let the double-click window lapse so the next click is a separate vertex.
fn click_route(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = at(h, point);
    click_canvas_at(h, pos);
    h.run_steps(20);
}

/// Send an exact route click and leave asynchronous settling to the caller.
fn click_route_with_modifiers(h: &mut Harness<'_, PcbApp>, point: Point, modifiers: Modifiers) {
    let pos = at(h, point);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
        h.step();
    }
}

/// Open the route menu using a complete right-click at the requested board point.
fn right_click_route(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = at(h, point);
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
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

/// Author classes through the shared command path until #303 provides editor controls.
fn classify_route(h: &mut Harness<'_, PcbApp>, name: &str, rules: Value) -> Uuid {
    let previous: BTreeSet<_> = h.state().session.project().unwrap().net_classes().definitions.iter().map(|class| class.id).collect();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::net_classes::CREATE, &json!({"name":name,"rules":rules,"expected_revision":revision}));
    let class = h.state().session.project().unwrap().net_classes().definitions.iter().find(|class| !previous.contains(&class.id)).unwrap().id;
    let report = h.state().session.project().unwrap().inspect_net_registry().unwrap();
    let anchor = report["nets"][0]["anchor"].clone();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::nets::SET_CLASSES,
        &json!({"target":{"kind":"derived","anchor":anchor},"classes":[class],"expected_revision":revision}),
    );
    h.run();
    assert_eq!(h.state().message, None);
    class
}

/// The actual X/click path adopts class dimensions once, paints their clearance, commits the
/// same engine-validated width and restores the classified project through ordinary Undo.
#[test]
fn classified_route_uses_defaults_halo_and_shared_undo() {
    for (theme, size) in [(ThemeChoice::Dark, egui::vec2(1400., 900.)), (ThemeChoice::Light, egui::vec2(960., 640.))] {
        let mut h = routable_with("resistor-0805");
        let id = h.state().views.active.id();
        h.state_mut().set_view_grid(id, 50 * pcbcraft_chrome::grid::NM_PER_MIL);
        h.set_size(size);
        h.state_mut().preferences.theme = theme;
        h.run();
        h.query_all_by_label("Fit board").next().unwrap().click();
        h.run();
        classify_route(&mut h, "Power", json!({"clearance_nm":700000,"min_track_width_nm":400000,"preferred_track_width_nm":500000}));
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        let line = airwires(&h)[0].clone();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, line.from.position);
        assert_eq!(h.state().views.active.board.tracks.width_nm, 500000);
        assert_eq!(h.state().message, None);
        let vertex =
            h.state().views.active.camera.snap(at(&h, Point { x_nm: line.from.position.x_nm, y_nm: -2540000 }), h.get_by_label("PCB canvas").rect());
        let end = at(&h, vertex);
        h.hover_at(end);
        h.run_steps(20);
        let radius = at(&h, Point { x_nm: 950000, y_nm: 0 }).distance(at(&h, Point { x_nm: 0, y_nm: 0 }));
        let ink = pcbcraft_chrome::Tokens::get().board_ink.gamma_multiply(0.19);
        assert!(
            h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Path(path)
            if path.fill==ink && path.points.iter().any(|point|(point.distance(end)-radius).abs()<0.01))),
            "{theme:?}: {:?}",
            h.state().views.active.board.tracks.preparation_status()
        );
        click_route(&mut h, vertex);
        click_route(&mut h, Point { x_nm: line.to.position.x_nm, y_nm: -2540000 });
        click_route(&mut h, line.to.position);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while h.state().session.revision() == revision {
            assert!(std::time::Instant::now() < deadline, "class route did not publish: {:?}", h.state().message);
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(tracks(&h).iter().all(|track| track.width_nm == 500000));
        assert!(airwires(&h).is_empty());
        h.key_press(Key::Escape);
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// A rule edit revokes an active draft; conflicting defaults refuse pickup visibly instead of
/// routing with a stale board preference or submitting a hidden partial edit.
#[test]
fn class_edits_revoke_drafts_and_unresolved_widths_report_errors() {
    let mut h = routable();
    let class = classify_route(&mut h, "Power", json!({"min_track_width_nm":400000,"preferred_track_width_nm":500000}));
    let line = airwires(&h)[0].clone();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    assert!(h.state().views.active.board.tracks.routing());
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::net_classes::UPDATE,
        &json!({"class":class,"name":"Power","rules":{"min_track_width_nm":500000,"preferred_track_width_nm":600000},"expected_revision":revision}),
    );
    h.run();
    assert!(!h.state().views.active.board.tracks.routing());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click_route(&mut h, line.from.position);
    assert_eq!(h.state().views.active.board.tracks.width_nm, 600000);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::net_classes::CREATE,
        &json!({"name":"Alternative","rules":{"preferred_track_width_nm":700000},"expected_revision":revision}),
    );
    let alternative = h.state().session.project().unwrap().net_classes().definitions.iter().find(|definition| definition.id != class).unwrap().id;
    let anchor = h.state().session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::nets::SET_CLASSES,
        &json!({"target":{"kind":"derived","anchor":anchor},"classes":[class,alternative],"expected_revision":revision}),
    );
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    assert!(!h.state().views.active.board.tracks.routing());
    assert!(h.state().message.as_ref().is_some_and(|message| message.contains("unresolved")));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.revision(), revision);
    assert!(tracks(&h).is_empty());
    let anchor = h.state().session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
    h.state_mut()
        .run(pcbcraft_engine::nets::SET_CLASSES, &json!({"target":{"kind":"derived","anchor":anchor},"classes":[],"expected_revision":revision}));
    h.run();
    click_route(&mut h, line.from.position);
    assert_eq!(h.state().message, None);
    assert!(h.state().views.active.board.tracks.routing());
    assert_eq!(h.state().views.active.board.tracks.width_nm, 250000);
}

/// Pads keep their exact copper contacts when the board uses a coarser or uneven grid. A route
/// started and finished on the two pads connects the same terminals regardless of snap spacing.
#[test]
fn pads_stay_reachable_on_coarse_and_uneven_board_grids() {
    const MIL: i64 = pcbcraft_chrome::grid::NM_PER_MIL;
    for spacing in [100 * MIL, 30 * MIL, 330_000] {
        let mut h = routable();
        let line = airwires(&h)[0].clone();
        let view = h.state().views.active.id();
        h.state_mut().set_view_grid(view, spacing);
        h.run();
        h.key_press(Key::X);
        h.run();
        route_airwire(&mut h, &line);
        assert_eq!(h.state().message, None, "{spacing}");
        assert!(airwires(&h).is_empty(), "{spacing}: the pads still connect");
        let ends: Vec<_> = tracks(&h).iter().flat_map(|track| [track.start, track.end]).collect();
        assert!(ends.contains(&line.from.position) && ends.contains(&line.to.position), "{spacing}: both pad centers are route endpoints");
    }
}

/// The crosshair follows the mouse while the preview and click target snap only within reach.
/// The preview's clearance extends past the track by the design-rule distance.
#[test]
fn routing_crosshair_follows_the_mouse_between_grid_points() {
    use pcbcraft_chrome::theme::{WIRE_CURSOR_GAP, WIRE_CURSOR_SNAP_RADIUS};

    let mut h = routable();
    let id = h.state().views.active.id();
    h.state_mut().set_view_grid(id, 50 * pcbcraft_chrome::grid::NM_PER_MIL);
    h.key_press(Key::X);
    h.run();
    let grid = Point { x_nm: 5_080_000, y_nm: 2_540_000 };
    let grid_screen = at(&h, grid);
    let ink = pcbcraft_chrome::Tokens::get().accent;
    for started in [false, true] {
        if started {
            click_route(&mut h, Point { x_nm: 2_540_000, y_nm: grid.y_nm });
        }
        for distance in [2.0, 6.0] {
            let pointer = grid_screen + egui::vec2(distance, 0.0);
            let right_arm = pointer + egui::vec2(WIRE_CURSOR_GAP, 0.0);
            h.hover_at(pointer);
            h.run();
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
            let shapes = &h.output().shapes;
            assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
                if stroke.color == ink && points[0].distance(right_arm) < 0.001)));
            assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle)
                if circle.radius == WIRE_CURSOR_SNAP_RADIUS && circle.center.distance(grid_screen) < 0.001)));
            assert_eq!(started, preview_ends_at(&h, pcbcraft_chrome::Tokens::get().board_front, grid_screen));
        }
    }
    let step = h.state().views.active.camera.snap_step();
    let off_grid = grid_screen + egui::vec2(step * 0.4, step * 0.4);
    assert!(off_grid.distance(grid_screen) > 8.0);
    h.hover_at(off_grid);
    h.run();
    let front = pcbcraft_chrome::Tokens::get().board_front;
    assert!(preview_ends_at(&h, front, off_grid));
    let halo_color = pcbcraft_chrome::Tokens::get().board_ink.gamma_multiply(0.19);
    let thickness = h.state().views.active.board.tracks.width_nm / 2 + pcbcraft_engine::board::drc::DEFAULT_CLEARANCE_NM;
    let radius = at(&h, Point { x_nm: thickness, y_nm: 0 }).distance(at(&h, Point { x_nm: 0, y_nm: 0 }));
    h.hover_at(grid_screen);
    h.run();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path)
        if path.fill == halo_color && path.points.iter().any(|point| point.distance(grid_screen + egui::vec2(radius, 0.0)) < 0.001))));
    h.get_by_label("Route tracks on the active copper layer").hover();
    h.run();
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    assert!(tracks(&h).is_empty());
}

/// Held Ctrl updates the preview without pointer movement, places free vertices and vias off-grid,
/// and still attaches exactly to a nearby pad. Releasing Ctrl restores the current view grid.
#[test]
fn ctrl_bypasses_the_routing_grid_but_keeps_copper_contacts() {
    let mut h = routable();
    let before = h.state().session.snapshot().unwrap();
    let pad = airwires(&h)[0].from.position;
    let rect = h.get_by_label("PCB canvas").rect();
    let grid = Point { x_nm: 3_810_000, y_nm: 2_540_000 };
    let grid_screen = at(&h, grid);
    let corner_screen = grid_screen + egui::vec2(4.0, 3.0);
    let via_screen = at(&h, Point { x_nm: 5_080_000, y_nm: 3_810_000 }) + egui::vec2(5.0, 4.0);
    let camera = &h.state().views.active.camera;
    let corner = camera.unsnapped(corner_screen, rect);
    let via = camera.unsnapped(via_screen, rect);
    let spacing = camera.snap_nm();
    assert_eq!(camera.snap(corner_screen, rect), grid);
    assert_ne!(corner.x_nm % spacing, 0);
    assert_ne!(via.x_nm % spacing, 0);

    let click_ctrl = |h: &mut Harness<'_, PcbApp>, pos| {
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::CTRL });
            h.run();
        }
        h.run_steps(20);
    };
    let copper = pcbcraft_chrome::Tokens::get().board_front;

    h.key_press(Key::X);
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    let pad_screen = at(&h, pad);
    let near_pad = pad_screen + egui::vec2(3.0, 2.0);
    h.hover_at(near_pad);
    h.run();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle)
        if circle.radius == pcbcraft_chrome::theme::WIRE_CURSOR_TARGET_RADIUS && circle.center.distance(pad_screen) < 0.001)));
    click_ctrl(&mut h, near_pad);
    h.hover_at(corner_screen);
    h.run();
    assert!(preview_ends_at(&h, copper, corner_screen));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert!(preview_ends_at(&h, copper, grid_screen));
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    h.run();
    assert!(preview_ends_at(&h, copper, corner_screen));
    click_ctrl(&mut h, corner_screen);
    h.hover_at(via_screen);
    h.run();
    h.key_press_modifiers(Modifiers::CTRL, Key::PageDown);
    h.run();
    let bottom = layer_named(&h, "Bottom");
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press_modifiers(Modifiers::CTRL, Key::Enter);
    h.run();

    assert_eq!(h.state().message, None);
    let ends: Vec<_> = tracks(&h).iter().flat_map(|track| [track.start, track.end]).collect();
    assert!(ends.contains(&pad) && ends.contains(&corner) && ends.contains(&via), "the route uses the exact pad and previewed free points");
    assert_eq!(vias(&h).iter().map(|via| via.at).collect::<Vec<_>>(), [via]);
    assert_eq!(h.state().views.active.camera.snap_nm(), spacing);
}

/// Escape and the menu's Cancel drop an unfinished route without editing; Enter finishes an open-ended
/// route; Backspace removes the last vertex. The width field sets the next route's width.
#[test]
fn unfinished_routes_cancel_and_finish_from_the_keyboard() {
    let mut h = routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Layout");
    click(&mut h, "Route tracks on the active copper layer");
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    click_route(&mut h, start);
    click_route(&mut h, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.board.tracks.tool, "Escape drops the route but keeps the tool");
    click_route(&mut h, start);
    right_click_route(&mut h, start);
    click(&mut h, "Cancel");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "cancelled routes edit nothing");

    h.state_mut().views.active.board.tracks.width_nm = 500_000;
    click(&mut h, "Layout");
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

/// The routing popup keeps the draft through dismissal. Via placement uses its opening point,
/// and Finish commits through the same track transaction as Enter.
#[test]
fn route_menu_owns_input_and_places_a_via_at_its_opening_point() {
    let mut h = routable();
    let before = h.state().session.snapshot().unwrap();
    let bottom = layer_named(&h, "Bottom");
    let start = Point { x_nm: 2_540_000, y_nm: 2_540_000 };
    let via = Point { x_nm: 5_080_000, y_nm: 2_540_000 };
    let end = Point { x_nm: 7_620_000, y_nm: 2_540_000 };
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, start);
    right_click_route(&mut h, via);
    let close = at(&h, Point { x_nm: 7_620_000, y_nm: -2_540_000 });
    click_canvas_at(&mut h, close);
    assert!(h.query_by_label("Place through via").is_none());
    assert!(h.state().views.active.board.tracks.routing());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    right_click_route(&mut h, via);
    click(&mut h, "Place through via");
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()));
    click_route(&mut h, end);
    right_click_route(&mut h, end);
    click(&mut h, "Finish track");
    assert_eq!(h.state().message, None);
    assert_eq!(vias(&h).iter().map(|via| via.at).collect::<Vec<_>>(), [via]);
    let ends: Vec<_> = tracks(&h).iter().flat_map(|track| [track.start, track.end]).collect();
    assert!(ends.contains(&start) && ends.contains(&end));
    assert!(!ends.contains(&Point { x_nm: 7_620_000, y_nm: -2_540_000 }));
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

/// An unsupported-copper frame discards a queued Delete instead of replaying it once arcs are removed.
#[test]
fn unsupported_copper_discards_pending_track_delete() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let board = h.state().session.project().unwrap().board_target().unwrap().board;
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":2_540_000},{"x_nm":7_620_000,"y_nm":2_540_000}],"expected_revision":revision}),
    );
    let id = tracks(&h)[0].id;
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board,"arcs":[{
        "id":"00000000-0000-0000-0000-000000000001","layer":top,"width_nm":250000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5000000,"y_nm":0},"end":{"x_nm":0,"y_nm":5000000},"direction":"clockwise"}],"expected_revision":revision}));
    h.run();
    assert!(h.query_all_by_label(pcbcraft_engine::board::arc::UNSUPPORTED).next().is_some());
    let state = &mut h.state_mut().views.active.board.tracks;
    state.selected.insert(id);
    state.delete_requested = true; // Already queued by input dispatch, before canvas interaction.
    h.run();
    assert!(!h.state().views.active.board.tracks.delete_requested);
    assert_eq!(tracks(&h)[0].id, id);

    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::board::arc::SET_ARCS, &json!({"board":board,"arcs":[],"expected_revision":revision}));
    h.state_mut().views.active.board.tracks.selected.insert(id);
    h.run();
    assert!(h.state().session.project().unwrap().boards().next().unwrap().arc_tracks().is_empty());
    assert_eq!(tracks(&h)[0].id, id);
}

/// The route tool is only available on visible copper: X does nothing on Board Outline, and hiding
/// the routing layer drops the tool and any unfinished route.
#[test]
fn routing_requires_a_visible_copper_layer() {
    let mut h = routable();
    click(&mut h, "Layout");
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

pub(super) fn vias(h: &Harness<'_, PcbApp>) -> Vec<pcbcraft_engine::board::track::Via> {
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
/// layers with both vias in one undoable command that clears the airwire. The preview retains
/// the front layer's tint and picks up the back layer's custom color after the first via.
#[test]
fn v_places_vias_and_switches_layers_mid_route() {
    let mut h = routable();
    let (top, bottom) = (layer_named(&h, "Top"), layer_named(&h, "Bottom"));
    let view = h.state().views.active.id();
    let custom_rgb = [35, 180, 130];
    h.state_mut().set_layer_color(view, &bottom, Some(custom_rgb)).unwrap();
    let line = airwires(&h)[0].clone();
    let (a, b) = (line.from.position, line.to.position);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, a);
    let near = at(&h, Point { x_nm: a.x_nm, y_nm: -2_540_000 });
    let via = h.state().views.active.camera.snap(near, h.get_by_label("PCB canvas").rect());
    click_route(&mut h, via);
    via_at(&mut h, via);
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()), "the view follows the via");
    let tail = at(&h, Point { x_nm: 5_080_000, y_nm: -2_540_000 });
    h.hover_at(tail);
    h.run();
    assert!(preview_ends_at(&h, pcbcraft_chrome::Tokens::get().board_front, at(&h, via)));
    assert!(preview_ends_at(&h, egui::Color32::from_rgb(custom_rgb[0], custom_rgb[1], custom_rgb[2]), tail));
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
    click(&mut h, "Layout");
    assert!(!h.get_by_label("Place vias").accesskit_node().is_disabled(), "Route › Via is a working button");
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
pub(super) fn add_track(h: &mut Harness<'_, PcbApp>, from: Point, to: Point) -> uuid::Uuid {
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

    // #281: a mil suffix overrides the millimetre display; a width outside the track limits is
    // refused with the allowed range and leaves the model and history unchanged.
    let type_width = |h: &mut Harness<'_, PcbApp>, text: &str| {
        h.get_by_label("Selected track width").click();
        h.run();
        h.get_by_label("Selected track width").type_text(text);
        h.run();
        h.key_press(Key::Enter);
        h.run();
    };
    type_width(&mut h, "20 mil");
    assert_eq!(tracks(&h).into_iter().find(|t| t.id == track).unwrap().width_nm, 508_000);
    assert_eq!(h.state().session.revision(), revision + 2);
    type_width(&mut h, "0 mm");
    use pcbcraft_chrome::units::{Unit, label};
    use pcbcraft_engine::board::track::{MAX_WIDTH_NM, MIN_WIDTH_NM};
    let range = format!("Enter {} to {}", label(MIN_WIDTH_NM, Unit::Mm), label(MAX_WIDTH_NM, Unit::Mm));
    assert!(h.query_by_label(&range).is_some(), "{range}");
    assert_eq!(tracks(&h).into_iter().find(|t| t.id == track).unwrap().width_nm, 508_000);
    assert_eq!(h.state().session.revision(), revision + 2);
}

/// Accepted copper edits disable all properties until publication, then expose the new model
/// values without disturbing the existing form's layout or accessible control identities.
#[test]
fn copper_properties_wait_for_pending_publication() {
    let mut h = routable();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":0,"y_nm":-5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    let id = vias(&h)[0].id;
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([id]);
    h.run();
    let source = h.state().views.active.input(&h.state().session).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().prepare_canvas_copper(crate::board_drawing::Command::new(
        source,
        pcbcraft_engine::SET_VIA_SIZE,
        json!({"vias":[id],"via_diameter_nm":800_000,"via_drill_nm":300_000,"expected_revision":revision}),
    ));
    h.step();
    for label in ["Copper lock", "Selected via pad", "Selected via drill"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
    h.run();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(vias(&h)[0].diameter_nm, 800_000);
    for label in ["Copper lock", "Selected via pad", "Selected via drill"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
}

/// Let the asynchronous checker finish through ordinary application frames.
fn finish_drc(h: &mut Harness<'_, PcbApp>) {
    for _ in 0..1000 {
        h.step();
        if !h.state().views.active.board.drc.pending() {
            h.step();
            return;
        }
    }
    panic!("checker did not complete");
}

/// Inspect › DRC checks the board without editing it and lists its findings; clicking one
/// centers the view there. An edit marks the result out of date until it is run again.
#[test]
fn drc_lists_findings_focuses_them_and_goes_stale_after_edits() {
    let mut h = routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Analyze");
    click(&mut h, "Run design rules check");
    finish_drc(&mut h);
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
    finish_drc(&mut h);
    assert!(h.query_by_label_contains("out of date").is_none());
    let board = h.state().views.active.board.target;
    assert!(!h.state().views.active.board.drc.markers(board).is_empty(), "open findings mark the canvas");
    click(&mut h, "Close");
    assert!(h.query_by_label_contains("Unrouted connection").is_none());
    assert!(h.state().views.active.board.drc.markers(board).is_empty(), "closing the card clears its markers");
}

/// Keep a legal routed connection while accepting two independently named schematic nets.
/// Every fixture edit uses the same engine commands as the app and MCP.
fn shorted_routable<'a>() -> Harness<'a, PcbApp> {
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let layer = layer_named(&h, "Top");
    let s = &mut h.state_mut().session;
    let board = line.from.terminal.board.board;
    s.apply(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"board":board,"layer":layer,"width_nm":250_000,
        "points":[line.from.position,line.to.position],"expected_revision":s.revision()}),
    )
    .unwrap();
    let terminals: Vec<_> =
        s.project().unwrap().symbols().iter().map(|symbol| s.project().unwrap().terminal_position(symbol.terminals()[0]).unwrap()).collect();
    for (index, at) in terminals.into_iter().enumerate() {
        s.apply(
            "schematic.place_net_label",
            &json!({"name":format!("INTENDED_{index}"),"style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,
            "expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let wire = s.project().unwrap().wires()[0].id();
    s.apply(pcbcraft_engine::DELETE_WIRE, &json!({"wire":wire,"expected_revision":s.revision()})).unwrap();
    s.apply(pcbcraft_engine::board::update::PREVIEW_UPDATE, &json!({"board":board,"expected_revision":s.revision()})).unwrap();
    let token = s.cached_board_update().unwrap().plan_token;
    s.apply(pcbcraft_engine::board::update::APPLY_UPDATE, &json!({"board":board,"plan_token":token,"expected_revision":s.revision()})).unwrap();
    h.run();
    h
}

/// The existing DRC card exposes the engine's short finding and focuses the corresponding marker.
/// GUI and shared command checks agree and leave design bytes and revision untouched.
#[test]
fn drc_shows_and_focuses_an_existing_short() {
    let mut h = shorted_routable();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let board = h.state().views.active.board.target.unwrap();
    let report = h.state().session.check_design(&json!({"board":board.board})).unwrap();
    assert_eq!(report["counts"]["short"], 1);
    let finding = report["violations"][0]["message"].as_str().unwrap();
    click(&mut h, "Analyze");
    click(&mut h, "Run design rules check");
    finish_drc(&mut h);
    assert!(h.query_by_label_contains("1 short").is_some());
    let label = format!("Short · {finding}");
    h.get_by_label(&label).click();
    h.run();
    assert!(h.state().views.active.board.drc.markers(Some(board)).iter().any(|(_, focused)| *focused));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
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

/// Outputs explains a missing outline, then captures the engine's complete ZIP without editing.
/// Injected outcomes exercise the notices, not the native dialog or browser runtime.
#[test]
fn outputs_export_one_archive_of_fabrication_files() {
    let mut h = routable();
    click(&mut h, "Manufacture");
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
    click(&mut h, "Manufacture");
    click(&mut h, "Generate fabrication outputs");
    assert_eq!(h.state().message, None);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "exporting edits nothing");
    let (name, archive) = h.state().fabrication_exports.captured.clone().expect("an archive was handed to the save worker");
    assert!(name.ends_with("-fabrication.zip"), "{name}");
    let outputs = h.state().session.export_fabrication(&json!({})).unwrap();
    assert!(outputs.iter().any(|o| o.name.ends_with("-Top.gbr")) && outputs.iter().any(|o| o.name.ends_with("-PTH.drl")));
    assert_eq!(archive.bytes(), pcbcraft_engine::board::fabrication::zip(&outputs).unwrap(), "the saved archive is the engine's deterministic one");
    assert!(h.state().fabrication_exports.busy());
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::Saved)).unwrap();
    h.run();
    assert!(!h.state().fabrication_exports.busy());
    let writes_notice = "Saved fabrication ZIP";
    assert!(h.query_by_label(writes_notice).is_some(), "the board status bar shows file results");
    click(&mut h, "Manufacture");
    click(&mut h, "Generate fabrication outputs");
    assert!(h.state().fabrication_exports.busy());
    assert!(h.state().file_notice.is_none(), "a new attempt clears the previous success notice");
    assert!(h.query_by_label(writes_notice).is_none());
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::Cancelled)).unwrap();
    h.run();
    assert!(!h.state().fabrication_exports.busy());
    assert!(h.state().file_notice.is_none() && h.state().message.is_none(), "cancellation is quiet");
    assert!(h.query_by_label(writes_notice).is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "cancelling edits nothing");
    click(&mut h, "Manufacture");
    click(&mut h, "Generate fabrication outputs");
    h.state_mut().fabrication_exports.complete_captured(Err("Archive published, but syncing its directory failed".into())).unwrap();
    h.run();
    assert!(h.state().message.as_deref().unwrap().contains("syncing its directory failed"));
    assert!(h.state().file_notice.is_none());
    assert!(h.query_by_label(writes_notice).is_none());
    click(&mut h, "Manufacture");
    click(&mut h, "Generate fabrication outputs");
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::DownloadRequested)).unwrap();
    h.run();
    assert_eq!(h.state().file_notice.as_deref(), Some("Requested fabrication ZIP download; check your browser"));
    assert!(h.query_by_label(writes_notice).is_none());
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
    assert!(!h.get_by_label("Move selected PCB items or text").accesskit_node().is_disabled(), "Move is offered for selected copper");
    click(&mut h, "Move selected PCB items or text");
    assert!(h.state().views.active.board.tracks.busy(), "the ribbon started a copper move");
}

/// View › Flip view mirrors this view left-to-right around the centered point, keeps the zoom,
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
    click(&mut h, "View");
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
    // Flip back before selecting: selection changes the Properties card, which moves the
    // uncovered canvas center that flipping reflects about.
    click(&mut h, "View");
    click(&mut h, "View the board from below");
    assert!(!h.state().views.active.camera.mirrored());
    assert!((at(&h, left) - left_before).length() < 0.5, "flipping back restores the view");
    click(&mut h, "View");
    click(&mut h, "View the board from below");
    let middle = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, middle);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "picking follows the mirror");
    click(&mut h, "View");
    click(&mut h, "View the board from below");
    assert!(!h.state().views.active.camera.mirrored());
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
    click(&mut h, "Analyze");
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

    // Rotations wrap into 0–359°.
    set(&mut h, "Footprint rotation", "-90");
    assert_eq!(placed(&h).1, 270);
    set(&mut h, "Footprint rotation", "360");
    assert_eq!(placed(&h).1, 0);
    assert_eq!(h.state().message, None);

    // #281: a coordinate past the board range or with an unknown unit is refused with an error
    // instead of being clamped, leaving the model and history unchanged; Escape restores the value.
    let revision = h.state().session.revision();
    set(&mut h, "Footprint X", "5000");
    assert!(h.query_by_label("Enter -987.3 mm to 987.3 mm").is_some());
    set(&mut h, "Footprint Y", "3 cm");
    assert!(h.query_by_label("Unknown unit \"cm\". Use mil, mm or in").is_some());
    for label in ["Footprint X", "Footprint Y"] {
        h.get_by_label(label).click();
        h.run();
        h.key_press(Key::Escape);
        h.run();
    }
    assert!(h.query_by_label("Enter -987.3 mm to 987.3 mm").is_none());
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("12.7 mm"));
    assert_eq!((placed(&h).0, h.state().session.revision()), (Point { x_nm: 12_700_000, y_nm: -2_540_000 }, revision));
}

/// #281: footprint, track and via length fields accept either unit's suffix over the displayed
/// default, and untouched nanometre-resolution values survive focus, unit switches and edits of
/// other properties without rounding, an extra revision or changed saved bytes.
#[test]
fn properties_lengths_accept_either_unit_and_keep_untouched_nanometres() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let id = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().id.clone();
    let placed = |h: &Harness<'_, PcbApp>| {
        let project = h.state().session.project().unwrap();
        let f = project.boards().next().unwrap().footprints().find(|f| f.id == id).unwrap();
        (f.position, f.rotation)
    };
    let set = |h: &mut Harness<'_, PcbApp>, label: &str, text: &str| {
        h.get_by_label(label).click();
        h.run();
        h.get_by_label(label).type_text(text);
        h.run();
        h.key_press(Key::Enter);
        h.run();
    };
    let origin = placed(&h).0;
    let center = at(&h, origin);
    click_canvas_at(&mut h, center);
    set(&mut h, "Footprint X", "100 mil");
    assert_eq!(placed(&h).0.x_nm, 2_540_000);
    // The typed edit produces exactly the document the canonical integer-nanometre command does.
    let encode = |h: &Harness<'_, PcbApp>| pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let typed = encode(&h);
    h.state_mut().run("edit.undo", &json!({}));
    let board = h.state().session.project().unwrap().boards().next().unwrap().target.board;
    let canonical = json!({"board":board,"footprint":id,"x_nm":2_540_000,"y_nm":origin.y_nm,"rotation":0,"side":"front","expected_revision":h.state().session.revision()});
    h.state_mut().session.apply(pcbcraft_engine::SET_FOOTPRINT_TRANSFORM, &canonical).unwrap();
    h.run();
    assert_eq!(encode(&h), typed);
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mil;
    h.run();
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("100 mil"));
    set(&mut h, "Footprint Y", "-2.54 mm");
    assert_eq!(placed(&h).0, Point { x_nm: 2_540_000, y_nm: -2_540_000 });

    // A nanometre-resolution coordinate is exact in both units and is never re-entered by
    // focusing, confirming unchanged text, switching units or rotating the footprint.
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    h.run();
    set(&mut h, "Footprint X", "1.234567");
    set(&mut h, "Footprint Y", "-0.000007 mm");
    let exact = Point { x_nm: 1_234_567, y_nm: -7 };
    let saved = pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    for unit in [pcbcraft_chrome::units::Unit::Mil, pcbcraft_chrome::units::Unit::Mm, pcbcraft_chrome::units::Unit::Mil] {
        h.state_mut().preferences.units = unit;
        h.run();
        for label in ["Footprint X", "Footprint Y"] {
            h.get_by_label(label).click();
            h.run();
            h.key_press(Key::Enter);
            h.run();
        }
    }
    assert_eq!((placed(&h).0, h.state().session.revision()), (exact, revision));
    assert_eq!(pcbcraft_engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    for project in [
        pcbcraft_engine::persistence::decode_documents(&saved).unwrap(),
        pcbcraft_engine::persistence::decode_packed(&pcbcraft_engine::persistence::encode_packed(h.state().session.snapshot().unwrap()).unwrap())
            .unwrap()
            .project,
    ] {
        assert_eq!(project.boards().next().unwrap().footprints().find(|f| f.id == id).unwrap().position, exact);
        assert_eq!(pcbcraft_engine::persistence::encode_documents(project).unwrap(), saved);
    }
    set(&mut h, "Footprint rotation", "90");
    assert_eq!(placed(&h), (exact, 90));
    assert_eq!(h.state().session.revision(), revision + 1);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(placed(&h), (exact, 0));
    h.state_mut().run("edit.redo", &json!({}));
    h.run();
    assert_eq!(placed(&h), (exact, 90));

    // A compact Properties panel keeps the same field and parser.
    h.set_size(egui::vec2(420., 900.));
    h.run();
    set(&mut h, "Footprint X", "50 mil");
    assert_eq!(placed(&h).0, Point { x_nm: 1_270_000, y_nm: -7 });
    assert_eq!(h.state().message, None);
}

/// #281 review: a via pad no larger than its drill is refused with the pad range and leaves both
/// dimensions and the revision unchanged; a valid pad keeps the drill exactly.
#[test]
fn properties_refuse_a_via_pad_smaller_than_its_drill() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":0,"y_nm":-5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    let id = vias(&h)[0].id;
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([id]);
    h.run();
    let type_pad = |h: &mut Harness<'_, PcbApp>, text: &str| {
        h.get_by_label("Selected via pad").click();
        h.run();
        h.get_by_label("Selected via pad").type_text(text);
        h.run();
        h.key_press(Key::Enter);
        h.run();
    };
    let revision = h.state().session.revision();
    type_pad(&mut h, "0.2 mm");
    use pcbcraft_chrome::units::{Unit, label};
    let range = format!("Enter {} to {}", label(300_001, Unit::Mm), label(pcbcraft_engine::board::track::MAX_VIA_DIAMETER_NM, Unit::Mm));
    assert!(h.query_by_label(&range).is_some(), "{range}");
    assert_eq!((vias(&h)[0].diameter_nm, vias(&h)[0].drill_nm, h.state().session.revision()), (600_000, 300_000, revision));
    type_pad(&mut h, "32 mil");
    assert_eq!((vias(&h)[0].diameter_nm, vias(&h)[0].drill_nm, h.state().session.revision()), (812_800, 300_000, revision + 1));
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
    let other = [Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 }];
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":other,"expected_revision":revision}));
    h.run();
    let revision = h.state().session.revision();
    h.hover_at(at(&h, a));
    h.run();
    h.key_press(Key::Backtick);
    h.run();
    assert_eq!(h.state().views.active.board.highlight_net.as_deref(), Some(net.as_str()));
    assert!(h.query_by_label_contains(&format!("Net {net} highlighted")).is_some());
    let front = pcbcraft_chrome::Tokens::get().board_front;
    let bright = pcbcraft_chrome::board::copper_emphasis(front, Some(true));
    let dim = pcbcraft_chrome::board::copper_emphasis(front, Some(false));
    assert!(track_has_color(&h, [path[1], path[2]], bright));
    assert!(track_has_color(&h, other, dim));
    assert_eq!(h.output().shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.fill == bright)).count(), 2);
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
    assert!(track_has_color(&h, other, front));
    assert_eq!(h.state().session.revision(), revision);
}

/// Net emphasis hides surface pads and tracks with their layers while retaining visible-layer
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
    let emphasized = |h: &Harness<'_, PcbApp>| {
        let tokens = pcbcraft_chrome::Tokens::get();
        let layer_colors = [tokens.board_front, tokens.board_back].map(|color| pcbcraft_chrome::board::copper_emphasis(color, Some(true)));
        let via_color = pcbcraft_chrome::board::copper_emphasis(tokens.board_through, Some(true));
        let canvas = h.get_by_label("PCB canvas").rect();
        h.output().shapes.iter().filter(|shape| shape.clip_rect == canvas).fold((0, 0, 0), |(pads, tracks, vias), shape| match &shape.shape {
            egui::Shape::Path(path) if layer_colors.contains(&path.fill) => (pads + 1, tracks, vias),
            egui::Shape::LineSegment { stroke, .. } if layer_colors.contains(&stroke.color) => (pads, tracks + 1, vias),
            egui::Shape::Circle(circle) if circle.fill == via_color => (pads, tracks, vias + 1),
            _ => (pads, tracks, vias),
        })
    };
    assert_eq!(emphasized(&h), (2, 2, 1), "two pads, both tracks and the via");
    h.state_mut().views.active.board.set_layer(&top, false, Some(false));
    h.run();
    assert_eq!(emphasized(&h), (0, 1, 1), "only the bottom track and via remain visible");
    h.state_mut().views.active.board.set_layer(&bottom, false, Some(false));
    h.run();
    assert_eq!(emphasized(&h), (0, 0, 1), "vias retain the normal renderer's policy even with all copper layers hidden");
    h.state_mut().views.active.board.set_layer(&top, false, Some(true));
    h.run();
    assert_eq!(emphasized(&h), (2, 1, 1), "restoring Top restores its pads and track");
    assert_eq!(h.state().views.active.board.highlight_net, highlighted);
    assert_eq!(h.state().session.revision(), revision);
}

/// Routing from a track body joins it at the projected point. An island without a named net
/// stays bright through the draft and unrelated copper returns to normal after cancellation.
#[test]
fn routing_from_an_unassigned_track_emphasizes_its_island() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let points = |y_nm| [Point { x_nm: 2_540_000, y_nm }, Point { x_nm: 7_620_000, y_nm }];
    let start = points(2_540_000);
    let other = points(-2_540_000);
    for path in [start, other] {
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":path,"expected_revision":revision}));
    }
    assert_eq!(h.state().message, None);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let contact = Point { x_nm: 5_080_000, y_nm: start[0].y_nm };
    let pointer = at(&h, contact) + egui::vec2(0.0, 3.0);
    let front = pcbcraft_chrome::Tokens::get().board_front;
    h.key_press(Key::X);
    h.run();
    click_canvas_at(&mut h, pointer);
    h.run_steps(20);
    assert!(track_has_color(&h, start, pcbcraft_chrome::board::copper_emphasis(front, Some(true))));
    assert!(track_has_color(&h, other, pcbcraft_chrome::board::copper_emphasis(front, Some(false))));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.key_press(Key::Escape);
    h.run();
    assert!(track_has_color(&h, start, front) && track_has_color(&h, other, front));
    click_canvas_at(&mut h, pointer);
    h.run_steps(20);
    let end = Point { x_nm: contact.x_nm, y_nm: 5_080_000 };
    click_route(&mut h, end);
    h.key_press(Key::Enter);
    h.run();
    assert!(tracks(&h).iter().any(|track| track.start == contact && track.end == end));
    assert!(track_has_color(&h, other, front));
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
    fn cached(h: &mut Harness<'_, PcbApp>) -> std::sync::Arc<crate::board_appearance::Connectivity> {
        let source = h.state().views.active.input(&h.state().session);
        let project = h.state().session.snapshot().unwrap();
        let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
        h.state_mut().views.active.board.copper_connectivity(board, source).unwrap()
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
    assert_eq!(first.nets.len(), tracks(&h).len());
    assert!(!first.nets.is_empty());
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
    assert!(edited.nets.is_empty(), "deleted copper cannot keep stale net assignments");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    let undone = cached(&mut h);
    assert_eq!(undone, first);
    assert!(!std::sync::Arc::ptr_eq(&first, &undone), "undo has a new revision");

    let project = h.state().session.snapshot().unwrap();
    let board = project.board_view(h.state().views.active.board.target.unwrap()).unwrap();
    let uncached = h.state_mut().views.active.board.copper_connectivity(board, None).unwrap();
    let another = h.state_mut().views.active.board.copper_connectivity(board, None).unwrap();
    assert_eq!(uncached, first);
    assert!(!std::sync::Arc::ptr_eq(&uncached, &another), "missing authority never caches a preview");
    let before_open = cached(&mut h);
    h.state_mut().session.open_snapshot(saved);
    h.run();
    let reopened = cached(&mut h);
    assert_eq!(reopened, first);
    assert!(!std::sync::Arc::ptr_eq(&before_open, &reopened));
}

/// Align › Align to Top aligns the selected footprints in one edit; Distribute needs three.
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

/// D drags the segment under the pointer while its neighbors stretch to stay attached; a click
/// commits all of them as one edit, and Esc cancels a drag without editing.
#[test]
fn d_drags_a_segment_and_keeps_neighbors_attached() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let y = 2_540_000;
    let p = |x: i64| Point { x_nm: x, y_nm: y };
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,"points":[p(1_270_000), p(3_810_000), p(6_350_000), p(8_890_000)],"expected_revision":revision}),
    );
    h.run();
    let revision = h.state().session.revision();
    h.hover_at(at(&h, p(5_080_000)));
    h.run();
    h.key_press(Key::D);
    h.run();
    assert!(h.state().views.active.board.tracks.busy());
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().views.active.board.tracks.busy());
    h.hover_at(at(&h, p(5_080_000)));
    h.run();
    h.key_press(Key::D);
    h.run();
    let lifted = Point { x_nm: 5_080_000, y_nm: 1_270_000 };
    let target = at(&h, lifted);
    click_canvas_at(&mut h, target);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1, "one edit for the segment and its neighbors");
    let up = |x: i64| Point { x_nm: x, y_nm: 1_270_000 };
    let key = |a: Point, b: Point| (a.x_nm, a.y_nm, b.x_nm, b.y_nm);
    let ends: std::collections::BTreeSet<_> = tracks(&h).iter().map(|t| key(t.start, t.end)).collect();
    let expected =
        std::collections::BTreeSet::from([key(p(1_270_000), up(3_810_000)), key(up(3_810_000), up(6_350_000)), key(up(6_350_000), p(8_890_000))]);
    assert_eq!(ends, expected, "the route stays connected");
}

/// With nothing selected, Properties summarizes the board: outline size, copper layers,
/// footprints, tracks, vias and unrouted connections.
#[test]
fn properties_summarize_the_board_when_nothing_is_selected() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    h.run();
    assert!(h.query_by_label("No outline").is_some());
    let outline = layer_named(&h, "Board Outline");
    let revision = h.state().session.revision();
    let p = |x: i64, y: i64| json!({"x_nm":x,"y_nm":y});
    h.state_mut().run(
        pcbcraft_engine::ADD_DRAWING,
        &json!({"layer":outline,"points":[p(-5_000_000,-5_000_000),p(15_000_000,-5_000_000),p(15_000_000,5_000_000),p(-5_000_000,5_000_000)],"closed":true,"expected_revision":revision}),
    );
    add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    h.run();
    // Each value sits in its own label's row: right of it and on the same line.
    let in_row = |h: &Harness<'_, PcbApp>, label: &str, value: &str| {
        h.query_all_by_label(label).any(|label| {
            let row = label.rect();
            h.query_all_by_label(value).any(|value| (value.rect().center().y - row.center().y).abs() < 2. && value.rect().left() > row.right())
        })
    };
    for (label, value) in [("Size", "20 mm × 10 mm"), ("Copper layers", "2"), ("Footprints", "2"), ("Tracks", "1"), ("Vias", "0"), ("Unrouted", "1")]
    {
        assert!(in_row(&h, label, value), "{label} shows {value}");
    }
    let line = &airwires(&h)[0];
    let (a, b) = (line.from.position, line.to.position);
    let path = [a, Point { x_nm: a.x_nm, y_nm: -2_540_000 }, Point { x_nm: b.x_nm, y_nm: -2_540_000 }, b];
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":top,"width_nm":250_000,"points":path,"expected_revision":revision}));
    h.run();
    assert!(in_row(&h, "Unrouted", "0"), "the next frame uses the new connectivity revision");
}

/// Independent track and via filters apply to clicking, marquee and connected selection.
#[test]
fn vias_filter_separately_from_tracks() {
    let mut h = routable();
    let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":7_620_000,"y_nm":2_540_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    h.run();
    let via = vias(&h)[0].id;
    let view = h.state().views.active.id();
    let ctx = h.ctx.clone();
    let on_track = at(&h, Point { x_nm: 5_080_000, y_nm: 2_540_000 });
    h.hover_at(on_track);
    h.run();
    h.key_press(Key::U);
    h.run();
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track, via]));
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([track, via]);
    h.state_mut().run_window_command(&ctx, "view.set_selection_filters", &json!({"view":view.0,"vias":false})).unwrap();
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "only the via leaves the selection");
    h.run();
    h.key_press(Key::U);
    h.run();
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "connected selection skips filtered vias");
    let on_via = at(&h, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    click_canvas_at(&mut h, on_via);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "the track is picked under the filtered via");
    let (from, to) = (at(&h, Point { x_nm: 1_524_000, y_nm: -2_540_000 }), at(&h, Point { x_nm: 8_636_000, y_nm: 3_810_000 }));
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([track]), "the box skips the via");
    h.state_mut().set_selection_filters(view, &json!({"vias": true, "tracks": false})).unwrap();
    assert!(h.state().views.active.board.tracks.selected.is_empty());
    h.run();
    click_canvas_at(&mut h, on_via);
    assert_eq!(h.state().views.active.board.tracks.selected, std::collections::BTreeSet::from([via]));
}

/// Saved filter settings from before the Vias row keep existing choices and enable vias.
#[test]
fn older_filter_settings_default_vias_to_enabled() {
    let filters: crate::selection_filters::Filters = serde_json::from_value(json!({"tracks":false,"footprints":false})).unwrap();
    assert!(filters.vias);
    assert!(!filters.tracks);
    assert!(!filters.footprints);
}

/// At a T-junction, D keeps the connection: a dragged end on the middle of another track holds
/// there while the segment pivots, and a track ending along a dragged segment follows it.
#[test]
fn d_keeps_t_junctions_attached() {
    let mut h = routable();
    let mm = |x: i64, y: i64| Point { x_nm: x * 1_270_000, y_nm: y * 1_270_000 };
    let bus = add_track(&mut h, mm(1, 2), mm(7, 2));
    let stub = add_track(&mut h, mm(4, 2), mm(4, 4));
    let drag = |h: &mut Harness<'_, PcbApp>, from: Point, to: Point| {
        h.hover_at(at(h, from));
        h.run();
        h.key_press(Key::D);
        h.run();
        let target = at(h, to);
        click_canvas_at(h, target);
        assert_eq!(h.state().message, None);
    };
    let ends = |h: &Harness<'_, PcbApp>, id| tracks(h).into_iter().find(|t| t.id == id).map(|t| (t.start, t.end)).unwrap();

    let revision = h.state().session.revision();
    drag(&mut h, mm(4, 3), mm(5, 3));
    assert_eq!(ends(&h, stub), (mm(4, 2), mm(5, 4)), "the end on the bus holds while the far end moves");
    assert_eq!(ends(&h, bus), (mm(1, 2), mm(7, 2)), "the bus does not move");
    assert_eq!(h.state().session.revision(), revision + 1);

    drag(&mut h, mm(2, 2), mm(2, 1));
    assert_eq!(ends(&h, bus), (mm(1, 1), mm(7, 1)));
    assert_eq!(ends(&h, stub), (mm(4, 1), mm(5, 4)), "the stub's end follows the bus it meets");
    assert_eq!(h.state().session.revision(), revision + 2);
}

/// Moving a footprint with M carries the track on its pad: the placed move commits the footprint
/// and the stretched track as one undo step, and undo restores both.
#[test]
fn footprint_moves_carry_routed_tracks() {
    let mut h = routable();
    let line = airwires(&h)[0].clone();
    let (pad, owner) = (line.from.position, line.from.footprint.clone());
    let free = Point { x_nm: pad.x_nm, y_nm: pad.y_nm + 2_540_000 };
    let stub = add_track(&mut h, pad, free);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let position = h.state().session.project().unwrap().boards().next().unwrap().footprints().find(|f| f.id == owner).unwrap().position;
    h.hover_at(at(&h, position));
    h.run();
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.draft.is_some());
    let destination = at(&h, Point { x_nm: position.x_nm, y_nm: position.y_nm - 2_540_000 });
    click_canvas_at(&mut h, destination);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1, "the footprint and its track move in one edit");
    let track = tracks(&h).into_iter().find(|t| t.id == stub).unwrap();
    assert_eq!((track.start, track.end), (Point { x_nm: pad.x_nm, y_nm: pad.y_nm - 2_540_000 }, free));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The shared lock control handles mixed copper and keeps its geometry fields inspectable but disabled.
#[test]
fn copper_lock_control_preserves_selection_and_guards_delete() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,
        "points":[{"x_nm":1_270_000,"y_nm":2_540_000},{"x_nm":6_350_000,"y_nm":2_540_000}],"expected_revision":revision}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":2_540_000,"y_nm":5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    assert!(h.state().message.is_none());
    let track = tracks(&h)[0].id;
    let via = h.state().session.project().unwrap().boards().next().unwrap().vias()[0].id;
    for id in [track, via] {
        h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([id]);
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        click(&mut h, "Copper lock");
        click(&mut h, "Locked");
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let board = h.state().session.project().unwrap().boards().next().unwrap();
        assert!(board.tracks().iter().any(|track| track.id == id && track.locked) || board.vias().iter().any(|via| via.id == id && via.locked));
        click(&mut h, "Copper lock");
        click(&mut h, "Unlocked");
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
    h.state_mut().views.active.board.tracks.selected.extend([track, via]);
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Copper lock");
    click(&mut h, "Locked");
    assert!(tracks(&h)[0].locked);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().vias()[0].locked);
    assert!(h.get_by_label("Selected track width").accesskit_node().is_disabled());
    assert!(h.get_by_label("Selected via pad").accesskit_node().is_disabled());
    assert!(h.get_by_label("Selected via drill").accesskit_node().is_disabled());
    let locked = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().message.as_ref().is_some_and(|message| message.contains("locked")));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), locked);
    assert!(h.state().views.active.board.tracks.selected.contains(&track));
    click(&mut h, "Copper lock");
    click(&mut h, "Unlocked");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(tracks(&h)[0].locked);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run();
    assert!(!tracks(&h)[0].locked);
    let board = h.state().session.project().unwrap().board_target().unwrap().board;
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_COPPER_LOCKED, &json!({"board":board,"tracks":[track],"locked":true,"expected_revision":revision}));
    h.state_mut().views.active.board.tracks.selected.extend([track, via]);
    h.run();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="Mixed")));
    click(&mut h, "Copper lock");
    click(&mut h, "Locked");
    let before_duplicate = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().duplicate_board_clipboard();
    h.run();
    assert!(h.state().views.active.board.clipboard.active(), "{:?}", h.state().message);
    let target = at(&h, Point { x_nm: 7_620_000, y_nm: 5_080_000 });
    click_canvas_at(&mut h, target);
    h.run();
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(tracks(&h).len(), 2);
    assert!(tracks(&h).iter().all(|track| track.locked));
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    assert_eq!(board.vias().len(), 2);
    assert!(board.vias().iter().all(|via| via.locked));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before_duplicate);
}

/// Dragging an unlocked middle segment cannot stretch a neighboring locked segment.
#[test]
fn segment_drag_cannot_move_a_locked_neighbor() {
    let mut h = routable();
    let top = layer_named(&h, "Top");
    let p = |x| Point { x_nm: x, y_nm: 2_540_000 };
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":top,"width_nm":250_000,
        "points":[p(1_270_000),p(3_810_000),p(6_350_000),p(8_890_000)],"expected_revision":revision}),
    );
    let first = tracks(&h).iter().find(|track| track.start == p(1_270_000)).unwrap().id;
    let board = h.state().session.project().unwrap().board_target().unwrap().board;
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_COPPER_LOCKED, &json!({"board":board,"tracks":[first],"locked":true,"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.hover_at(at(&h, p(5_080_000)));
    h.run();
    h.key_press(Key::D);
    h.run();
    let target = at(&h, Point { x_nm: 5_080_000, y_nm: 1_270_000 });
    click_canvas_at(&mut h, target);
    assert!(h.state().message.as_ref().is_some_and(|message| message.contains("locked")));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Escape cancels a pending check without a model edit; ordinary camera navigation remains
/// available, while releasing the view's input prevents its delayed report from publishing.
#[test]
fn checker_escape_navigation_and_view_release_preserve_authored_state() {
    let mut h = routable();
    let snapshot = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run_drc();
    assert!(h.state().views.active.board.drc.pending());
    h.key_press(Key::Escape);
    h.step();
    assert!(!h.state().views.active.board.drc.pending());
    h.state_mut().run_drc();
    let rect = h.get_by_label("PCB canvas").rect();
    let point = rect.center();
    h.event(egui::Event::PointerMoved(point));
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0., 1.),
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(h.state().views.active.board.drc.pending());
    h.state_mut().views.active.release_input();
    h.step();
    assert!(!h.state().views.active.board.drc.pending());
    assert_eq!(h.state().session.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
}

/// Higher-priority keyboard owners retain Escape while a checker is pending. The unclaimed
/// canvas key still cancels the checker through the same shortcut dispatcher.
#[test]
fn checker_escape_respects_popup_text_and_assistant_keyboard_ownership() {
    let mut h = routable();
    let ctx = h.ctx.clone();
    let popup = egui::Id::unique("checker escape host popup");
    h.state_mut().run_drc();
    egui::Popup::open_id(&ctx, popup);
    h.key_press(Key::Escape);
    h.step();
    assert!(h.state().views.active.board.drc.pending(), "the host must not steal a popup's Escape");
    egui::Popup::close_id(&ctx, popup);
    h.state_mut().views.active.board.drc.cancel();
    for owner in 0..4 {
        h.state_mut().run_drc();
        let popup = egui::Id::unique("checker escape owner");
        if owner == 0 {
            egui::Popup::open_id(&ctx, popup);
        }
        let mut text = String::new();
        let _ = ctx.run_ui(
            egui::RawInput {
                focused: true,
                events: vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                if owner == 1 {
                    ui.text_edit_singleline(&mut text).request_focus();
                    assert!(ctx.egui_wants_keyboard_input());
                }
                let view = h.state().views.active.id();
                h.state_mut().shortcuts(&ctx, owner == 2, view);
                assert_eq!(h.state().views.active.board.drc.pending(), owner != 3);
                assert_eq!(ctx.input(|i| i.key_pressed(Key::Escape)), owner != 3);
            },
        );
        egui::Popup::close_id(&ctx, popup);
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        h.state_mut().views.active.board.drc.cancel();
    }
}

/// A route remains a neutral ghost while preparation is pending. Escape during that interval
/// revokes even a final-click request, while leaving the board and undo revision untouched.
#[test]
fn escape_revokes_a_pending_final_route_before_publication() {
    let mut h = routable();
    // A detour exceeds the warm inline allowance, keeping a real final request pending.
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":2_540_000,"y_nm":-5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    h.run();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, Point { x_nm: 0, y_nm: -5_080_000 });
    let revision = h.state().session.revision();
    h.hover_at(at(&h, Point { x_nm: 5_080_000, y_nm: -5_080_000 }));
    h.run_steps(1);
    assert!(matches!(h.state().views.active.board.tracks.preparation_status().as_deref(), Some("Planning route") | Some("Preparing route")));
    h.key_press(Key::Enter);
    h.run_steps(1);
    assert_eq!(h.state().session.revision(), revision);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert!(tracks(&h).is_empty());
    assert!(!h.state().views.active.board.tracks.routing());
}

/// Real ribbon selections independently override class preferences and survive subsequent
/// unclassified pickup. A hard-invalid explicit width is refused without replacement or edits.
#[test]
fn explicit_sizes_survive_classified_and_unclassified_pickups() {
    for (width_override, via_override) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut h = routable();
        h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
        classify_route(
            &mut h,
            "Power",
            json!({"min_track_width_nm":400000,"preferred_track_width_nm":500000,
            "preferred_via_diameter_nm":1000000,"preferred_via_drill_nm":500000}),
        );
        let line = airwires(&h)[0].clone();
        if width_override {
            click(&mut h, "Track / line width");
            click(&mut h, "0.4 mm");
        }
        if via_override {
            click(&mut h, "Via pad / drill");
            click(&mut h, "0.8 / 0.4 mm");
        }
        let before = h.state().session.snapshot().unwrap();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, line.from.position);
        let state = &h.state().views.active.board.tracks;
        assert_eq!(state.width_nm, if width_override { 400000 } else { 500000 });
        assert_eq!((state.via_diameter_nm, state.via_drill_nm), if via_override { (800000, 400000) } else { (1000000, 500000) });
        click(&mut h, "Track / line width");
        click(&mut h, "0.75 mm");
        click(&mut h, "Via pad / drill");
        click(&mut h, "0.8 / 0.4 mm");
        h.key_press(Key::Escape);
        h.run();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, Point { x_nm: 2540000, y_nm: -2540000 });
        let state = &h.state().views.active.board.tracks;
        assert_eq!((state.width_nm, state.via_diameter_nm, state.via_drill_nm), (750000, 800000, 400000));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        h.key_press(Key::Escape);
        h.run();
        h.key_press_modifiers(Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE }, Key::X);
        h.run();
        let revision = h.state().session.revision();
        click_route(&mut h, line.from.position);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while h.state().session.revision() == revision {
            assert!(std::time::Instant::now() < deadline, "{:?}", h.state().message);
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(h.state().message, None);
        assert_eq!((vias(&h)[0].diameter_nm, vias(&h)[0].drill_nm), (800000, 400000));
        h.key_press(Key::Escape);
        h.run();
        click(&mut h, "Track / line width");
        click(&mut h, "0.1 mm");
        let before = h.state().session.snapshot().unwrap();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, line.to.position);
        assert!(!h.state().views.active.board.tracks.routing());
        assert_eq!(h.state().views.active.board.tracks.width_nm, 100000);
        assert!(h.state().message.as_ref().is_some_and(|message| message.contains("minimum")));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let board = h.state().session.project().unwrap().board_target().unwrap().board;
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::SET_RULES,
            &json!({"board":board,"expected_revision":revision,
            "rules":pcbcraft_engine::board::rules::Values{clearance_nm:300000,..Default::default()}}),
        );
        h.run();
        assert_eq!(h.state().views.active.board.tracks.width_nm, 250000);
        click_route(&mut h, line.to.position);
        assert_eq!(h.state().message, None);
        assert!(h.state().views.active.board.tracks.routing());
        let state = &h.state().views.active.board.tracks;
        assert_eq!((state.width_nm, state.via_diameter_nm, state.via_drill_nm), (500000, 1000000, 500000));
    }
}

/// Unresolved via preferences do not prevent a track-only route, but V, layer keys and the
/// route menu cannot insert stale sizes. An explicit valid pair resolves only that operation.
#[test]
fn unresolved_via_defaults_block_layer_changes_until_explicitly_resolved() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let class = classify_route(
        &mut h,
        "Power",
        json!({"preferred_track_width_nm":500000,"preferred_via_diameter_nm":600000,"preferred_via_drill_nm":300000}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::net_classes::CREATE,
        &json!({"name":"Alternative via","rules":{"preferred_via_diameter_nm":800000,"preferred_via_drill_nm":400000},"expected_revision":revision}),
    );
    let alternative = h.state().session.project().unwrap().net_classes().definitions.iter().find(|definition| definition.id != class).unwrap().id;
    let anchor = h.state().session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::nets::SET_CLASSES,
        &json!({"target":{"kind":"derived","anchor":anchor},"classes":[class,alternative],"expected_revision":revision}),
    );
    h.run();
    let line = airwires(&h)[0].clone();
    let top = layer_named(&h, "Top");
    let bottom = layer_named(&h, "Bottom");
    let first = Point { x_nm: line.from.position.x_nm, y_nm: -2540000 };
    let last = Point { x_nm: line.to.position.x_nm, y_nm: -2540000 };
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    assert!(h.state().views.active.board.tracks.routing());
    assert_eq!(h.state().message, None);
    via_at(&mut h, first);
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    assert!(h.state().message.as_ref().is_some_and(|message| message.contains("unresolved")));
    click(&mut h, "Track / line width");
    click(&mut h, "0.75 mm");
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    assert!(h.state().views.active.board.tracks.routing());
    h.hover_at(at(&h, first));
    h.run();
    h.key_press(Key::PageDown);
    h.run();
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    right_click_route(&mut h, first);
    click(&mut h, "Place through via");
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert!(h.state().views.active.board.tracks.routing());
    assert!(vias(&h).is_empty());
    // Completing a track without a via does not depend on those conflicting preferences.
    click_route(&mut h, first);
    click_route(&mut h, last);
    click_route(&mut h, line.to.position);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while h.state().session.revision() == revision {
        assert!(std::time::Instant::now() < deadline, "{:?}", h.state().message);
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.state().message, None);
    assert!(vias(&h).is_empty());
    h.key_press(Key::Escape);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(tracks(&h).is_empty());
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    click(&mut h, "Via pad / drill");
    click(&mut h, "0.8 / 0.4 mm");
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, line.from.position);
    via_at(&mut h, first);
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(bottom.as_str()));
    via_at(&mut h, last);
    assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(top.as_str()));
    let revision = h.state().session.revision();
    click_route(&mut h, line.to.position);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while h.state().session.revision() == revision {
        assert!(std::time::Instant::now() < deadline, "{:?}", h.state().message);
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.state().message, None);
    assert_eq!(vias(&h).len(), 2);
    assert!(vias(&h).iter().all(|via| via.diameter_nm == 800000 && via.drill_nm == 400000));
    assert_eq!(h.state().session.revision(), revision + 1);
}

/// A Ctrl+PgDn via uses the preview's latched track end through the release band, then
/// follows the unsnapped pointer beyond it. Publication and Undo retain the exact point.
#[test]
fn review408_via_uses_latched_preview_target_until_release() {
    for offset in [10., 13.] {
        let mut h = routable();
        let line = airwires(&h)[0].clone();
        let endpoint = Point { x_nm: 5_080_000, y_nm: -2_540_000 };
        let revision = h.state().session.revision();
        let layer = layer_named(&h, "Top");
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":layer,"width_nm":250000,"points":[line.from.position,Point {x_nm:line.from.position.x_nm,y_nm:endpoint.y_nm},endpoint],"expected_revision":revision}));
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, line.to.position);
        let contact = at(&h, endpoint);
        h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
        h.hover_at(contact);
        h.run_steps(30);
        let pointer = contact + egui::vec2(offset, 0.);
        let free = h.state().views.active.camera.unsnapped(pointer, h.get_by_label("PCB canvas").rect());
        let expected = if offset <= 12. { endpoint } else { free };
        h.hover_at(pointer);
        h.run_steps(30);
        assert!(preview_ends_at(&h, pcbcraft_chrome::Tokens::get().board_front, at(&h, expected)));
        h.key_press_modifiers(Modifiers::CTRL, Key::PageDown);
        h.run_steps(30);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().views.active.board.active_layer.as_deref(), Some(layer_named(&h, "Bottom").as_str()));
        h.key_press_modifiers(Modifiers::CTRL, Key::Enter);
        h.run_steps(30);
        assert_eq!(h.state().message, None);
        assert_eq!(vias(&h).iter().map(|via| via.at).collect::<Vec<_>>(), vec![expected], "offset {offset}: committed via differs from preview");
        h.state_mut().run("edit.undo", &json!({}));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// A one-point reached plan at the fixed head must validate its via there, even while
/// the display retains a longer preview. Landing on a bottom-layer class that rejects
/// the via size cannot change layers or history.
#[test]
fn review408_via_at_fixed_head_validates_adopted_geometry() {
    for blocked in [false, true] {
        let mut h = routable();
        let start = Point { x_nm: 1_270_000, y_nm: -5_080_000 };
        let head = Point { x_nm: 5_080_000, y_nm: -5_080_000 };
        let old_target = Point { x_nm: 7_620_000, y_nm: -5_080_000 };
        let top = layer_named(&h, "Top");
        let bottom = layer_named(&h, "Bottom");
        if blocked {
            let pad = airwires(&h)[0].from.position;
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::ADD_TRACKS,
                &json!({
                    "layer":top,"width_nm":250_000,
                    "points":[pad,Point { x_nm: pad.x_nm, y_nm: 2_540_000 },
                        Point { x_nm: head.x_nm, y_nm: 2_540_000 },head],
                    "layer_changes":[{"at":1,"layer":bottom}],
                    "via_diameter_nm":1_000_000,"via_drill_nm":500_000,
                    "expected_revision":revision
                }),
            );
            assert_eq!(h.state().message, None);
            h.run();
            classify_route(&mut h, "Larger landing via", json!({"min_via_diameter_nm":800_000}));
        }
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.key_press(Key::X);
        h.run();
        click_route(&mut h, start);
        click_route(&mut h, head);
        h.hover_at(at(&h, old_target));
        h.run_steps(30);
        assert!(preview_ends_at(&h, pcbcraft_chrome::Tokens::get().board_front, at(&h, old_target)));
        via_at(&mut h, head);
        h.run_steps(30);
        let state = &h.state().views.active.board;
        assert_eq!(state.active_layer.as_deref(), Some(if blocked { top.as_str() } else { bottom.as_str() }), "blocked={blocked}");
        assert!(state.tracks.routing(), "the fixed route must survive via validation");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        if blocked {
            assert!(h.state().message.is_some(), "the via at the head must fail before layer adoption");
        } else {
            assert_eq!(h.state().message, None);
            h.key_press(Key::Enter);
            h.run_steps(30);
            assert_eq!(h.state().message, None);
            assert_eq!(vias(&h).iter().map(|via| via.at).collect::<Vec<_>>(), vec![head]);
            assert!(tracks(&h).iter().all(|track| track.start != old_target && track.end != old_target));
            h.state_mut().run("edit.undo", &json!({}));
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        }
    }
}

/// The real menu switches to shove, hides displaced originals and publishes incoming and
/// displaced copper in one edit. Cancel restores the authored view without touching history.
#[test]
fn shove_menu_preview_cancel_and_atomic_publication() {
    for cancel in [true, false] {
        let mut h = routable();
        let layer = h.state().views.active.board.active_layer.clone().unwrap();
        let a = Point { x_nm: 0, y_nm: -7_620_000 };
        let b = Point { x_nm: 10_160_000, y_nm: -7_620_000 };
        let start = Point { x_nm: 2_540_000, y_nm: -5_080_000 };
        let end = Point { x_nm: 7_620_000, y_nm: -7_620_000 };
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":layer,"width_nm":250_000,"points":[a,b],"expected_revision":revision}));
        h.run();
        let original = tracks(&h)[0].clone();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.key_press(Key::X);
        h.run();
        right_click_route(&mut h, start);
        click(&mut h, "Shove tracks");
        if !cancel {
            right_click_route(&mut h, start);
            click(&mut h, "Merge same-net copper");
        }
        click_route(&mut h, start);
        h.hover_at(at(&h, end));
        h.run_steps(200);
        assert_eq!(h.state().message, None);
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        let base = pcbcraft_chrome::Tokens::get().board_front;
        assert!(!track_has_color(&h, [a, b], base), "the displaced original was still painted");
        assert!(preview_ends_at(&h, base, at(&h, end)), "missing incoming shove preview");
        if cancel {
            h.key_press(Key::Escape);
            h.run();
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            assert!(track_has_color(&h, [a, b], base));
        } else {
            click_route(&mut h, end);
            h.key_press(Key::Enter);
            h.run_steps(100);
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.revision(), revision + 1);
            assert!(tracks(&h).len() > 2);
            assert_ne!(tracks(&h).iter().find(|t| t.id == original.id), Some(&original));
            let board = h.state().session.project().unwrap().boards().next().unwrap();
            assert!(
                !board
                    .check_design(board.effective_rules().values)
                    .violations
                    .iter()
                    .any(|v| matches!(v.kind, pcbcraft_engine::board::drc::Kind::Clearance | pcbcraft_engine::board::drc::Kind::Short))
            );
            let after = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            h.state_mut().run("edit.undo", &json!({}));
            h.run();
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
            h.state_mut().run("edit.redo", &json!({}));
            h.run();
            assert_eq!(after, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        }
    }
}

/// A first click outside copper but inside a corner's clearance cannot become an inescapable
/// draft. Keep X armed so the next clear click starts an ordinary route without Escape/relaunch.
#[test]
fn rejected_clearance_pickup_keeps_the_route_tool_usable() {
    for shove in [false, true] {
        let mut h = routable();
        let layer = layer_named(&h, "Top");
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS,&json!({"layer":layer,"width_nm":500_000,
            "points":[Point{x_nm:4_000_000,y_nm:-7_620_000},Point{x_nm:4_000_000,y_nm:-2_540_000},Point{x_nm:10_160_000,y_nm:-2_540_000}],"expected_revision":revision}));
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        h.key_press(Key::X);
        h.run();
        let bad = Point { x_nm: 3_500_000, y_nm: -2_540_000 };
        if shove {
            right_click_route(&mut h, bad);
            click(&mut h, "Shove tracks");
        }
        h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
        click_route(&mut h, bad);
        h.run_steps(100);
        assert!(!h.state().views.active.board.tracks.routing(), "invalid start stayed active, shove={shove}");
        assert!(h.state().views.active.board.tracks.tool);
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        click_route(&mut h, Point { x_nm: 1_270_000, y_nm: -5_080_000 });
        click_route(&mut h, Point { x_nm: 2_540_000, y_nm: -5_080_000 });
        h.key_press(Key::Enter);
        h.run_steps(100);
        assert_eq!(h.state().session.revision(), revision + 1, "a clear route could not be placed, shove={shove}; {:?}", h.state().message);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
}

/// Shared-net parallel rails must retain spacing in the actual displayed displacement, even
/// though they are already electrically joined by a branch and ordinary DRC exempts that net.
#[test]
fn shoved_connected_rails_keep_spacing_in_the_editor() {
    use pcbcraft_engine::board::geometry::Contour;
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        let mut h = routable();
        h.state_mut().preferences.theme = theme;
        h.run();
        let layer = layer_named(&h, "Top");
        let mut rail_ids = Vec::new();
        for (points, width) in [
            (vec![Point { x_nm: 0, y_nm: 5_000_000 }, Point { x_nm: 20_000_000, y_nm: 5_000_000 }], 500_000),
            (vec![Point { x_nm: 0, y_nm: 5_800_000 }, Point { x_nm: 20_000_000, y_nm: 5_800_000 }], 250_000),
            (vec![Point { x_nm: 17_000_000, y_nm: 5_000_000 }, Point { x_nm: 17_000_000, y_nm: 8_000_000 }], 250_000),
        ] {
            let old: BTreeSet<_> = tracks(&h).iter().map(|t| t.id).collect();
            let rev = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":layer,"points":points,"width_nm":width,"expected_revision":rev}));
            rail_ids.push(tracks(&h).iter().find(|t| !old.contains(&t.id)).unwrap().id);
        }
        let pad =
            h.state().session.project().unwrap().boards().next().unwrap().footprints().flat_map(|f| f.pads).find(|p| p.net.is_some()).unwrap().center;
        let rev = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS,&json!({"layer":layer,"points":[pad,Point {y_nm:5_000_000,..pad},Point{x_nm:0,y_nm:5_000_000}],"width_nm":250_000,"expected_revision":rev}));
        h.run();
        h.query_all_by_label("Fit board").next().unwrap().click();
        h.run();
        assert_eq!(h.state().message, None);
        let original = tracks(&h);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        let start = Point { x_nm: 6_858_000, y_nm: 7_366_000 };
        let target = Point { x_nm: 14_224_000, y_nm: 4_572_000 };
        h.key_press(Key::X);
        h.run();
        right_click_route(&mut h, start);
        click(&mut h, "Shove tracks");
        h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
        click_route(&mut h, start);
        h.hover_at(at(&h, target));
        h.run_steps(200);
        let displayed = h.state().views.active.board.tracks.replacements().to_vec();
        let paths = |id| {
            let original = original.iter().find(|t| t.id == id).unwrap();
            let points = displayed.iter().find(|r| r.track == id).map_or_else(|| vec![original.start, original.end], |r| r.points.clone());
            points.windows(2).map(|p| Contour::segment(p[0], p[1], original.width_nm).unwrap()).collect::<Vec<_>>()
        };
        assert!(
            displayed.iter().any(|r| {
                let rail = original.iter().find(|t| t.id == r.track).unwrap();
                rail_ids[..2].contains(&r.track) && r.points != [rail.start, rail.end] && r.points != [rail.end, rail.start]
            }),
            "neither rail moved: {displayed:?}"
        );
        let (a, b) = (paths(rail_ids[0]), paths(rail_ids[1]));
        let gap = a.iter().flat_map(|a| b.iter().map(move |b| a.gap(b))).fold(f64::INFINITY, f64::min);
        assert!(gap >= 200_000., "{theme:?} preview collapsed same-net rail clearance to {gap}");
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        click_route(&mut h, target);
        h.key_press_modifiers(Modifiers::CTRL, Key::Enter);
        h.run_steps(100);
        assert!(h.state().session.revision() <= revision + 1);
        if h.state().session.revision() == revision + 1 {
            h.state_mut().run("edit.undo", &json!({}));
            h.run();
        }
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap(), "a refused route left a partial shove behind");
    }
}

/// Routing from an unconnected pad pushes a connected pair of rails ahead instead of
/// sending the new route behind the footprint. Publication retains the pad's separate island.
#[test]
fn shove_from_an_unconnected_pad_pushes_both_rails_forward() {
    let mut h = routable_with("resistor-0805");
    let target = h.state().session.project().unwrap().board_target().unwrap();
    let footprint = h.state().session.project().unwrap().board_view(target).unwrap().footprints().min_by_key(|f| f.position.x_nm).unwrap();
    let revision = h.state().session.revision();
    let (id, position) = (footprint.id.clone(), footprint.position);
    h.state_mut().run(
        pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":position.x_nm,"y_nm":position.y_nm,"rotation":180,"expected_revision":revision}),
    );
    h.run();
    let footprint = h.state().session.project().unwrap().board_view(target).unwrap().footprints().min_by_key(|f| f.position.x_nm).unwrap();
    let start = footprint.pads.iter().find(|p| p.net.is_none()).unwrap().center;
    let named = footprint.pads.iter().find(|p| p.net.is_some()).unwrap().center;
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let p = |x, y| Point { x_nm: named.x_nm + x, y_nm: named.y_nm + y };
    for (points, width) in [
        (vec![named, p(0, 5_000_000)], 500_000),
        (vec![p(0, 5_000_000), p(20_000_000, 5_000_000)], 500_000),
        (vec![p(975_000, 5_800_000), p(20_975_000, 5_800_000)], 250_000),
        (vec![p(17_975_000, 5_000_000), p(17_975_000, 8_000_000)], 250_000),
    ] {
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::ADD_TRACKS, &json!({"layer":layer,"points":points,"width_nm":width,"expected_revision":revision}));
        assert_eq!(h.state().message, None);
    }
    h.run();
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    let originals = tracks(&h);
    let rails: Vec<_> = originals.iter().filter(|t| t.start.y_nm == t.end.y_nm).cloned().collect();
    assert_eq!(rails.len(), 2);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let end = p(4_000_000, 6_500_000);
    {
        use pcbcraft_engine::board::route::{DEFAULT_BUDGET, Mode, Outcome, Posture, Request};
        let request = Request {
            mode: Mode::Shove,
            merge_same_net: false,
            fixed: Vec::new(),
            head: start,
            target: end,
            origin: None,
            layer: Uuid::parse_str(&layer).unwrap(),
            width_nm: 250_000,
            posture: Posture::Diagonal,
            flipped: false,
            previous: Vec::new(),
        };
        let query = h.state().session.clearance_query_with_rules(target, &json!({})).unwrap();
        let plan = h.state().session.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome, Outcome::Reached, "direct plan: {plan:?}");
        assert_eq!(plan.replacements.len(), 2, "direct plan: {plan:?}");
    }
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Shove tracks");
    click_route(&mut h, start);
    h.hover_at(at(&h, end));
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(
        h.state().views.active.board.tracks.displacement_preview(),
        2,
        "start {start:?} end {end:?}, startscreen {:?} endscreen {:?}; {:?}",
        at(&h, start),
        at(&h, end),
        h.state().views.active.board.tracks
    );
    let replacements = h.state().views.active.board.tracks.replacements().to_vec();
    for rail in &rails {
        let path = &replacements.iter().find(|r| r.track == rail.id).unwrap().points;
        assert!(path.iter().all(|p| p.y_nm >= rail.start.y_nm), "a preview leg moved behind the approach: {path:?}");
    }
    let placed = h.state().views.active.camera.snap(at(&h, end), h.get_by_label("PCB canvas").rect());
    click_route(&mut h, end);
    h.key_press(Key::Enter);
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let laid = tracks(&h);
    for rail in &rails {
        assert_ne!(laid.iter().find(|track| track.id == rail.id).unwrap(), rail);
    }
    let displaced = assert_published_replacements(&originals, &laid, &replacements);
    let mut frontier = vec![start];
    let mut reached = BTreeSet::from([(start.x_nm, start.y_nm)]);
    while let Some(point) = frontier.pop() {
        for track in laid.iter().filter(|track| !displaced.contains(&track.id)) {
            let next = if track.start == point {
                Some(track.end)
            } else if track.end == point {
                Some(track.start)
            } else {
                None
            };
            if let Some(next) = next
                && reached.insert((next.x_nm, next.y_nm))
            {
                frontier.push(next);
            }
        }
    }
    assert!(reached.contains(&(placed.x_nm, placed.y_nm)), "the incoming route did not connect its pickup and goal");
    let board = h.state().session.project().unwrap().board_view(target).unwrap();
    assert!(
        !board
            .check_design(board.effective_rules().values)
            .violations
            .iter()
            .any(|v| matches!(v.kind, pcbcraft_engine::board::drc::Kind::Clearance | pcbcraft_engine::board::drc::Kind::Short))
    );
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// Compare every published leg with the placement path and the source track's dimensions.
fn assert_published_replacements(
    originals: &[Track],
    laid: &[Track],
    replacements: &[pcbcraft_engine::board::track::replacements::Replacement],
) -> BTreeSet<Uuid> {
    let mut matched = BTreeSet::new();
    for replacement in replacements {
        let rail = originals.iter().find(|track| track.id == replacement.track).expect("original source identity");
        assert!(laid.iter().any(|track| track.id == rail.id), "missing published source identity {}", rail.id);
        for pair in replacement.points.windows(2) {
            let segment = laid
                .iter()
                .find(|track| {
                    track.layer == rail.layer
                        && track.width_nm == rail.width_nm
                        && ([track.start, track.end] == pair || [track.end, track.start] == pair)
                })
                .unwrap_or_else(|| panic!("published shove is missing {pair:?} for {}", rail.id));
            matched.insert(segment.id);
        }
    }
    matched
}

/// A native-command rail scene with fixed pad contacts, a branch, locked rail and fixed via.
fn parallel_rail_board<'a>() -> (Harness<'a, PcbApp>, Track) {
    let mut h = routable_spaced("resistor-0805", 20_000_000);
    let target = h.state().session.project().unwrap().board_target().unwrap();
    let poses: Vec<_> = h.state().session.project().unwrap().board_view(target).unwrap().footprints().map(|f| (f.id.clone(), f.position)).collect();
    for (id, at) in poses {
        let rev = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":180,"expected_revision":rev}),
        );
    }
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    for (points, width) in [
        (vec![p(-975_000, 0), p(-975_000, 5_000_000), p(19_025_000, 5_000_000), p(19_025_000, 0)], 500_000),
        (vec![p(0, 5_800_000), p(20_000_000, 5_800_000)], 250_000),
        (vec![p(17_000_000, 5_000_000), p(17_000_000, 8_000_000)], 250_000),
        (vec![p(4_000_000, -4_000_000), p(16_000_000, -4_000_000)], 500_000),
    ] {
        let rev = h.state().session.revision();
        h.state_mut()
            .run(pcbcraft_engine::ADD_TRACKS, &json!({"board":target.board,"layer":layer,"points":points,"width_nm":width,"expected_revision":rev}));
        assert_eq!(h.state().message, None);
    }
    let locked = tracks(&h).into_iter().find(|t| t.start.y_nm == -4_000_000).unwrap();
    let rev = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_COPPER_LOCKED, &json!({"board":target.board,"tracks":[locked.id],"locked":true,"expected_revision":rev}));
    let rev = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"board":target.board,"at":p(12_000_000,-4_000_000),"via_diameter_nm":1_000_000,"via_drill_nm":500_000,"expected_revision":rev}),
    );
    h.run();
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    (h, locked)
}

/// Moving past the upper rail retains both displaced paths and commits the same candidate;
/// the nearby locked rail/via stay fixed and the original board returns with one Undo.
#[test]
fn shove_preview_stays_displaced_at_the_top_of_the_channel() {
    let (mut h, locked) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    h.state_mut().views.active.camera.center_on(p(8_128_000, 3_619_500));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let original_vias = vias(&h);
    assert_eq!(original_vias.len(), 1);
    let rev = h.state().session.revision();
    let start = p(8_128_000, 9_398_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Shove tracks");
    click_route(&mut h, start);
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    for y in [1_016_000, -1_524_000, -2_159_000] {
        h.hover_at(at(&h, p(8_128_000, y)));
        h.run_steps(300);
        assert_eq!(
            h.state().views.active.board.tracks.displacement_preview(),
            2,
            "y={y}: {:?}",
            h.state().views.active.board.tracks.preparation_status()
        );
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
    let stopped = p(8_128_000, -2_159_000);
    let beyond = p(8_128_000, -2_667_000);
    h.hover_at(at(&h, beyond));
    h.run_steps(300);
    assert_eq!(h.state().views.active.board.tracks.displacement_preview(), 2, "retain the last valid shove instead of walking around");
    let ink = pcbcraft_chrome::Tokens::get().board_front;
    let retained = if preview_ends_at(&h, ink, at(&h, beyond)) { beyond } else { stopped };
    let forbidden = p(8_128_000, -3_683_000);
    h.hover_at(at(&h, forbidden));
    h.run_steps(300);
    assert_eq!(h.state().views.active.board.tracks.preparation_status().as_deref(), Some("Route blocked"));
    assert_eq!(h.state().views.active.board.tracks.displacement_preview(), 2);
    assert!(preview_ends_at(&h, ink, at(&h, retained)));
    assert!(!preview_ends_at(&h, ink, at(&h, forbidden)));
    let fixed = h.state().views.active.board.tracks.fixed_route_points().unwrap().to_vec();
    click_route_with_modifiers(&mut h, forbidden, Modifiers::CTRL);
    h.run_steps(300);
    assert_eq!(h.state().views.active.board.tracks.fixed_route_points(), Some(fixed.as_slice()), "the blocked click fixed a preview vertex");
    assert_eq!(h.state().session.revision(), rev, "a blocked preview cannot commit");
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    h.hover_at(at(&h, stopped));
    h.run_steps(300);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    click_route_with_modifiers(&mut h, stopped, Modifiers::CTRL);
    h.key_press(Key::Enter);
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), rev + 1);
    let kept = tracks(&h).into_iter().find(|t| t.id == locked.id).unwrap();
    assert_eq!((kept.start, kept.end), (locked.start, locked.end));
    assert!(kept.locked);
    assert_eq!(vias(&h), original_vias);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// A rightward cursor sweep can retry the return approach without trapping the second rail.
/// Preview and commit share the same two replacements and Undo restores the original board.
#[test]
fn lateral_shove_preview_preserves_the_branch_return_corridor() {
    let (mut h, _) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    h.state_mut().views.active.camera.center_on(p(8_128_000, 3_619_500));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let originals = tracks(&h);
    let revision = h.state().session.revision();
    let start = p(2_159_000, 6_604_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Shove tracks");
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    click_route(&mut h, start);
    let ink = pcbcraft_chrome::Tokens::get().board_front;
    for x in [11_176_000, 12_192_000, 13_208_000, 13_970_000] {
        let target = p(x, -1_016_000);
        h.hover_at(at(&h, target));
        h.run_steps(300);
        assert_eq!(h.state().views.active.board.tracks.displacement_preview(), 2);
        assert!(preview_ends_at(&h, ink, at(&h, target)), "cursor {target:?}");
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    click_route(&mut h, p(13_970_000, -1_016_000));
    h.run_steps(250);
    let expected = h.state().views.active.board.tracks.replacements().to_vec();
    assert_eq!(expected.len(), 2);
    h.key_press(Key::Enter);
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_published_replacements(&originals, &tracks(&h), &expected);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// The return bend stays short during a lateral sweep, and the exact displayed replacements
/// prepare and publish together without changing the branch contacts or Undo boundary.
#[test]
fn shove_return_preview_does_not_form_a_long_hairpin() {
    let (mut h, _) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    let originals = tracks(&h);
    let lower = originals.iter().find(|t| t.start == p(0, 5_800_000) && t.end == p(20_000_000, 5_800_000)).unwrap().id;
    h.state_mut().views.active.camera.center_on(p(8_128_000, 3_619_500));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let start = p(3_556_000, 7_366_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Shove tracks");
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    click_route(&mut h, start);
    let ink = pcbcraft_chrome::Tokens::get().board_front;
    for x in [12_954_000, 13_208_000, 13_462_000] {
        let target = p(x, 254_000);
        h.hover_at(at(&h, target));
        h.run_steps(300);
        assert!(preview_ends_at(&h, ink, at(&h, target)));
        let replacements = h.state().views.active.board.tracks.replacements();
        assert_eq!(replacements.len(), 2);
        let path = &replacements.iter().find(|r| r.track == lower).unwrap().points;
        let retreat: i64 = path.windows(2).map(|p| (p[0].x_nm - p[1].x_nm).max(0)).sum();
        assert!(retreat <= 250_000, "preview backtracks {retreat} nm: {path:?}");
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    click_route(&mut h, p(13_462_000, 254_000));
    h.run_steps(250);
    let expected = h.state().views.active.board.tracks.replacements().to_vec();
    assert_eq!(expected.len(), 2);
    h.key_press(Key::Enter);
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_published_replacements(&originals, &tracks(&h), &expected);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// A clearance-limited preview fixes its visible legal endpoint even with the cursor inside
/// clearance. Publication uses that adjusted endpoint and Undo restores the original board.
#[test]
fn walkaround_preview_and_placement_use_the_visible_boundary() {
    let (mut h, _) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    h.state_mut().views.active.camera.center_on(p(3_048_000, 3_556_000));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let old_ids: BTreeSet<_> = tracks(&h).iter().map(|t| t.id).collect();
    let revision = h.state().session.revision();
    let head = p(6_096_000, 6_858_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, head);
    click(&mut h, "Walk around obstacles");
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    click_route(&mut h, head);
    let ink = pcbcraft_chrome::Tokens::get().board_front;
    for y in [1_016_000, 2_032_000, 3_048_000, 4_064_000] {
        let target = p(-1_524_000, y);
        h.hover_at(at(&h, target));
        h.run_steps(300);
        assert!(preview_ends_at(&h, ink, at(&h, p(-1_550_002, y))), "boundary at y={y}");
        assert!(!preview_ends_at(&h, ink, at(&h, target)));
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    let pressed = at(&h, p(-1_524_000, 4_064_000));
    let released = at(&h, p(-1_524_000, 3_810_000));
    h.event(egui::Event::PointerButton { pos: pressed, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    h.hover_at(released);
    h.run();
    h.event(egui::Event::PointerButton { pos: released, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    let outside = h.get_by_label("PCB canvas").rect().right_top() + egui::vec2(20., 20.);
    h.event(egui::Event::PointerMoved(outside));
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    h.key_press(Key::Enter);
    for step in 0..200 {
        h.hover_at(outside + egui::vec2(step as f32 * 0.1, 0.));
        h.step();
    }
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let added: Vec<_> = tracks(&h).into_iter().filter(|t| !old_ids.contains(&t.id)).collect();
    assert!(added.iter().any(|t| t.end == p(-1_550_002, 3_810_000)), "placed endpoint must match visible clearance boundary: {added:?}");
    assert!(!added.iter().any(|t| t.end == p(-1_524_000, 3_810_000)), "blocked raw cursor cannot become copper");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// Walkaround follows both pointer axes for legal pad-gap targets and keeps a blocked target
/// separate from the visible fallback, without placing it or changing copper membership.
#[test]
fn walkaround_preview_follows_both_axes_in_the_pad_gap() {
    let (mut h, _) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    h.state_mut().views.active.camera.center_on(p(4_000_000, 2_500_000));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let start = p(2_286_000, 6_350_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Walk around obstacles");
    click_route(&mut h, start);
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    let base = pcbcraft_chrome::Tokens::get().board_front;
    for y in [-508_000, -254_000, 254_000] {
        for x in [-127_000, 0, 127_000] {
            let target = p(x, y);
            h.hover_at(at(&h, target));
            h.run_steps(300);
            assert!(preview_ends_at(&h, base, at(&h, target)), "cursor {target:?}: {:?}", h.state().views.active.board.tracks.preparation_status());
            assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
        }
    }
    let blocked = p(254_000, -254_000);
    h.hover_at(at(&h, blocked));
    h.run_steps(300);
    assert!(!preview_ends_at(&h, base, at(&h, blocked)));
    assert_eq!(h.state().session.revision(), revision);
    let end = p(0, -254_000);
    h.hover_at(at(&h, end));
    h.run_steps(300);
    assert!(preview_ends_at(&h, base, at(&h, end)));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    click_route(&mut h, end);
    h.key_press(Key::Enter);
    h.run_steps(200);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// Routing treats a press/move/release as placement even when it exceeds the ordinary click
/// tolerance. Subsequent pointer motion cannot replace that released target before validation.
#[test]
fn route_placement_does_not_require_a_stationary_click() {
    let mut h = routable();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let start = p(0, -5_080_000);
    let pressed = p(2_540_000, -5_080_000);
    let released = p(5_080_000, -5_080_000);
    h.key_press(Key::X);
    h.run();
    click_route(&mut h, start);
    let from = at(&h, pressed);
    let to = at(&h, released);
    h.hover_at(from);
    h.run();
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run();
    h.hover_at(to);
    h.run();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    let outside = h.get_by_label("PCB canvas").rect().right_top() + egui::vec2(20., 20.);
    h.event(egui::Event::PointerMoved(outside));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    h.key_press(Key::Enter);
    for step in 0..100 {
        h.hover_at(outside + egui::vec2(step as f32 * 0.2, 0.));
        h.step();
    }
    h.run_steps(100);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let copper = tracks(&h);
    assert_eq!(copper.len(), 1, "exactly the released vertex should be fixed");
    assert_eq!((copper[0].start, copper[0].end), (start, released));
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// Moving around a pad and continuing from a fixed top bend both retain a placeable
/// clearance guide. A corner click publishes that guide through the ordinary transaction.
#[test]
fn walkaround_pad_bend_and_inside_corner_remain_placeable() {
    for fixed in [false, true] {
        let (mut h, _) = parallel_rail_board();
        let p = |x, y| Point { x_nm: x, y_nm: y };
        h.state_mut().views.active.camera.center_on(p(0, 2_540_000));
        h.run();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let old_ids: BTreeSet<_> = tracks(&h).iter().map(|t| t.id).collect();
        let revision = h.state().session.revision();
        let origin = p(-1_016_000, 6_096_000);
        h.key_press(Key::X);
        h.run();
        right_click_route(&mut h, origin);
        click(&mut h, "Walk around obstacles");
        h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
        click_route(&mut h, origin);
        let ink = pcbcraft_chrome::Tokens::get().board_front;
        let above = p(-660_000, -900_000);
        h.hover_at(at(&h, above));
        h.run_steps(300);
        assert!(preview_ends_at(&h, ink, at(&h, p(-660_000, -1_050_002))));
        if fixed {
            click_route_with_modifiers(&mut h, above, Modifiers::CTRL);
            h.run_steps(300);
            assert_eq!(h.state().views.active.board.tracks.fixed_route_points().and_then(|points| points.last()), Some(&p(-660_000, -1_050_002)));
        }
        for target in [p(-254_000, 1_016_000), p(-300_000, 1_100_000), p(-200_000, 1_270_000)] {
            h.hover_at(at(&h, target));
            h.run_steps(300);
            assert!(
                preview_ends_at(&h, ink, at(&h, target)),
                "fixed{fixed}, target{target:?}: {:?}",
                h.state().views.active.board.tracks.preparation_status()
            );
            assert_eq!(h.state().session.revision(), revision);
        }
        let raw_corner = p(-508_000, 5_334_000);
        let expected = p(-399_998, 4_424_998);
        h.hover_at(at(&h, raw_corner));
        h.run_steps(300);
        assert!(preview_ends_at(&h, ink, at(&h, expected)), "fixed{fixed}: {:?}", h.state().views.active.board.tracks.preparation_status());
        click_route(&mut h, raw_corner);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.key_press(Key::Enter);
        h.run_steps(300);
        assert_eq!(h.state().message, None);
        assert_eq!(h.state().session.revision(), revision + 1, "fixed{fixed}: {:?}", h.state().views.active.board.tracks.preparation_status());
        let added: Vec<_> = tracks(&h).into_iter().filter(|t| !old_ids.contains(&t.id)).collect();
        assert!(added.iter().any(|t| t.end == expected), "fixed{fixed}: {added:?}");
        assert!(!added.iter().any(|t| t.end == raw_corner));
        let target = h.state().session.project().unwrap().board_target().unwrap();
        let board = h.state().session.project().unwrap().board_view(target).unwrap();
        assert!(!board.check_design(board.effective_rules().values).violations.iter().any(|v| matches!(
            v.kind,
            pcbcraft_engine::board::drc::Kind::Clearance | pcbcraft_engine::board::drc::Kind::Short
        )
            && v.items.iter().any(|id| added.iter().any(|t| t.id.to_string() == *id))));
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
    }
}

/// Explicit Shove selection and a clicked U-shaped prefix must still displace the rails
/// as the cursor advances from below them into the gap near the upper rail.
#[test]
fn shove_from_a_bent_prefix_advances_through_both_rails() {
    let (mut h, _) = parallel_rail_board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    for i in 0..12 {
        let x = -12_000_000 + i * 500_000;
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::ADD_TRACKS,
            &json!({"layer":layer,"width_nm":250_000,
            "points":[p(x,9_000_000),p(x,14_000_000)],"expected_revision":revision}),
        );
        assert_eq!(h.state().message, None);
    }
    h.state_mut().views.active.camera.center_on(p(3_048_000, 4_826_000));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let prefix = [p(-3_556_000, 3_810_000), p(-3_556_000, 6_096_000), p(254_000, 9_906_000), p(4_318_000, 9_906_000)];
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, prefix[0]);
    click(&mut h, "Shove tracks");
    for point in prefix {
        click_route(&mut h, point);
        h.run_steps(200);
        assert_eq!(h.state().message, None);
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    let ink = pcbcraft_chrome::Tokens::get().board_front;
    for y in [6_858_000, 6_604_000, 6_350_000, 6_096_000, 5_842_000, 5_588_000, 5_334_000, 5_080_000] {
        let target = p(7_874_000, y);
        h.hover_at(at(&h, target));
        h.run_steps(300);
        assert!(preview_ends_at(&h, ink, at(&h, target)), "y{y}: {:?}", h.state().views.active.board.tracks.preparation_status());
        if y <= 5_334_000 {
            assert_eq!(
                h.state().views.active.board.tracks.displacement_preview(),
                2,
                "y{y}: {:?}",
                h.state().views.active.board.tracks.preparation_status()
            );
        }
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    click_route(&mut h, p(7_874_000, 5_080_000));
    h.key_press(Key::Enter);
    h.run_steps(300);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}

/// Backspace and Enter can share a frame. Finishing at the shortened prefix
/// still needs the displacement for that prefix, including its existing rails.
#[test]
fn shove_backspace_and_finish_revalidate_the_retained_prefix() {
    let (mut h, _) = parallel_rail_board();
    let p = |x_nm, y_nm| Point { x_nm, y_nm };
    h.state_mut().views.active.camera.center_on(p(8_128_000, 3_619_500));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let originals = tracks(&h);
    let revision = h.state().session.revision();
    let start = p(2_159_000, 6_604_000);
    h.key_press(Key::X);
    h.run();
    right_click_route(&mut h, start);
    click(&mut h, "Shove tracks");
    click_route_with_modifiers(&mut h, start, Modifiers::CTRL);
    h.run_steps(40);
    let retained = p(13_970_000, -1_016_000);
    click_route_with_modifiers(&mut h, retained, Modifiers::NONE);
    h.run_steps(300);
    let prefix = h.state().views.active.board.tracks.fixed_route_points().unwrap().to_vec();
    let expected = h.state().views.active.board.tracks.replacements().to_vec();
    assert_eq!(expected.len(), 2, "the retained prefix must displace both rails");
    let next = p(13_970_000, -1_778_000);
    click_route_with_modifiers(&mut h, next, Modifiers::NONE);
    h.run_steps(300);
    let points = h.state().views.active.board.tracks.fixed_route_points().unwrap();
    assert_eq!(&points[..points.len() - 1], &prefix);
    assert_eq!(points.last(), Some(&next));
    h.key_press(Key::Backspace);
    h.key_press(Key::Enter);
    h.run_steps(300);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().session.revision(), revision + 1);
    let laid = tracks(&h);
    assert!(laid.iter().any(|track| track.end == retained));
    assert!(!laid.iter().any(|track| track.end == next));
    assert_published_replacements(&originals, &laid, &expected);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(before, encode_documents(h.state().session.snapshot().unwrap()).unwrap());
}
