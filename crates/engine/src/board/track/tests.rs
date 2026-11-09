use super::*;
use crate::{
    INITIALIZE_BOARD_SETUP,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Three placed 0402 resistors whose first terminals share the accepted net ACCEPTED and whose
/// last two share a second net, on a two-layer board with an accepted baseline. Built through
/// the shared production commands.
fn routed_fixture() -> Session {
    let mut session = super::super::ratsnest::tests::fixture();
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| symbol.id()).collect();
    session
        .apply(
            crate::CONNECT_PINS,
            &json!({"from":{"symbol":symbols[1],"terminal":"2"},"to":{"symbol":symbols[2],"terminal":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    session
        .apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":session.revision()}))
        .unwrap();
    session.apply(super::super::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    session
}

fn view(session: &Session) -> BoardView<'_> {
    let project = session.project().unwrap();
    project.board_view(project.board_target().unwrap()).unwrap()
}

/// The front and back copper layer IDs, chosen by side rather than catalog position.
fn copper_layers(session: &Session) -> Vec<Uuid> {
    use super::super::setup::Side;
    let layers = view(session).setup().unwrap().layers().to_vec();
    let side = |side| layers.iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(side)).unwrap().id;
    vec![side(Side::Front), side(Side::Back)]
}

/// A path that leaves both pads vertically, so it never crosses neighboring pads on the way.
fn detour(from: Point, to: Point) -> Vec<Point> {
    let y = from.y_nm.max(to.y_nm) + 2_000_000;
    vec![from, Point { x_nm: from.x_nm, y_nm: y }, Point { x_nm: to.x_nm, y_nm: y }, to]
}

fn route(session: &mut Session, layer: Uuid, points: &[Point]) -> Result<u64, CommandError> {
    session.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":DEFAULT_WIDTH_NM,"points":points,"expected_revision":session.revision()}))
}

/// Find a pad's center by footprint and number from the resolved view.
fn pad(session: &Session, footprint: &str, number: &str) -> (Point, Option<String>) {
    let pad = view(session).footprints().find(|f| f.id == footprint).unwrap().pads.into_iter().find(|p| p.number == number).unwrap();
    (pad.center, pad.net)
}

/// Routing an accepted connection on front copper joins its pads into one island and removes its
/// airwire. The route saves as board format 12, reopens identically, and undo restores the earlier bytes.
#[test]
fn routing_a_connection_clears_its_airwire_saves_and_undoes() {
    let mut s = routed_fixture();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let ratsnest = view(&s).ratsnest().unwrap();
    assert_eq!(ratsnest.lines.len(), 3, "two ACCEPTED airwires and one on the second net");
    let line = ratsnest.lines[0].clone();
    let front = copper_layers(&s)[0];
    let revision = s.revision();
    route(&mut s, front, &detour(line.from.position, line.to.position)).unwrap();
    assert_eq!(s.revision(), revision + 1, "one route is one edit");
    assert_eq!(view(&s).tracks().len(), 3);
    assert!(view(&s).tracks().windows(2).all(|pair| pair[0].id < pair[1].id), "tracks save in ID order");
    let copper = view(&s).copper();
    assert_eq!(
        copper.pads[&(line.from.footprint.clone(), line.from.pad.number.as_str().to_owned())],
        copper.pads[&(line.to.footprint.clone(), line.to.pad.number.as_str().to_owned())]
    );
    let routed = view(&s).ratsnest().unwrap();
    assert_eq!(routed.lines.len(), 2, "the routed pair no longer needs an airwire");
    let on_net = |lines: &[super::super::ratsnest::Airwire]| lines.iter().filter(|l| l.net == line.net).count();
    assert_eq!((on_net(&ratsnest.lines), on_net(&routed.lines)), (2, 1), "only the routed net loses an airwire");

    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let board = String::from_utf8(saved["board.json"].clone()).unwrap();
    assert!(board.contains("\"format\": \"pcbcraft-board/12\""), "{board}");
    assert_eq!(board.matches("\"width_nm\":250000").count(), 3, "one line per segment");
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.board.tracks, s.project().unwrap().board.tracks);
    assert_eq!(encode_documents(reopened).unwrap(), saved, "saving a reopened route changes nothing");

    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before, "undo restores the untracked format and bytes");
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Copper only connects on a shared layer: a back-side track touching front-only SMD pads
/// leaves the airwire in place. Deleting a front segment brings the airwire back.
#[test]
fn connectivity_follows_layers_and_deletion() {
    let mut s = routed_fixture();
    let line = view(&s).ratsnest().unwrap().lines[0].clone();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    route(&mut s, back, &detour(line.from.position, line.to.position)).unwrap();
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), 3, "back copper does not reach front SMD pads");
    route(&mut s, front, &detour(line.from.position, line.to.position)).unwrap();
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), 2);
    let front_track = view(&s).tracks().iter().find(|t| t.layer == front).unwrap().id;
    let revision = s.revision();
    assert!(s.apply(DELETE_TRACKS, &json!({"tracks":[Uuid::from_u128(7)],"expected_revision":revision})).is_err());
    assert!(s.apply(DELETE_TRACKS, &json!({"tracks":[front_track, front_track],"expected_revision":revision})).is_err());
    assert_eq!(s.revision(), revision);
    s.apply(DELETE_TRACKS, &json!({"tracks":[front_track],"expected_revision":revision})).unwrap();
    assert_eq!(view(&s).tracks().len(), 5, "three back segments and two remaining front segments");
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), 3);
}

/// Copper that would join pads on two different nets is refused without any partial edit.
#[test]
fn a_route_that_shorts_two_nets_is_refused_whole() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let footprint = view(&s)
        .footprints()
        .find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net)
        .expect("a footprint whose pads are on two accepted nets");
    let (a, net_a) = pad(&s, &footprint.id, &footprint.pads[0].number);
    let (b, net_b) = pad(&s, &footprint.id, &footprint.pads[1].number);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let error = route(&mut s, front, &detour(a, b)).unwrap_err().to_string();
    assert!(error.contains(net_a.as_deref().unwrap()) && error.contains(net_b.as_deref().unwrap()), "{error}");
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Malformed paths, non-copper layers, invalid widths and stale revisions leave the board intact.
#[test]
fn invalid_routes_change_nothing() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let edge = view(&s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let a = Point { x_nm: 0, y_nm: 20_000_000 };
    let b = Point { x_nm: 5_000_000, y_nm: 20_000_000 };
    let revision = s.revision();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    for params in [
        json!({"layer":edge,"width_nm":DEFAULT_WIDTH_NM,"points":[a,b]}),
        json!({"layer":Uuid::from_u128(5),"width_nm":DEFAULT_WIDTH_NM,"points":[a,b]}),
        json!({"layer":front,"width_nm":0,"points":[a,b]}),
        json!({"layer":front,"width_nm":MAX_WIDTH_NM + 1,"points":[a,b]}),
        json!({"layer":front,"width_nm":DEFAULT_WIDTH_NM,"points":[a]}),
        json!({"layer":front,"width_nm":DEFAULT_WIDTH_NM,"points":[a,a]}),
        json!({"layer":front,"width_nm":DEFAULT_WIDTH_NM,"points":[a,{"x_nm":crate::schematic::MAX_CENTER_NM + 1,"y_nm":0}]}),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(revision);
        assert!(s.apply(ADD_TRACKS, &params).is_err(), "{params}");
    }
    assert!(s.apply(ADD_TRACKS, &json!({"layer":front,"width_nm":DEFAULT_WIDTH_NM,"points":[a,b],"expected_revision":revision + 9})).is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Saved files must declare tracks with format 12, and format 12 must carry tracks.
#[test]
fn board_format_12_and_tracks_require_each_other() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    route(&mut s, front, &[Point { x_nm: 0, y_nm: 20_000_000 }, Point { x_nm: 5_000_000, y_nm: 20_000_000 }]).unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let text = String::from_utf8(saved["board.json"].clone()).unwrap();
    let mut downgraded = saved.clone();
    downgraded.insert("board.json".into(), text.replace("pcbcraft-board/12", "pcbcraft-board/10").into_bytes());
    assert!(decode_documents(&downgraded).is_err(), "older formats cannot carry tracks");
    let start = text.find("  \"tracks\": [").unwrap();
    let end = start + text[start..].find("  ],\n").unwrap() + "  ],\n".len();
    let mut untracked = saved.clone();
    untracked.insert("board.json".into(), format!("{}{}", &text[..start], &text[end..]).into_bytes());
    assert!(decode_documents(&untracked).is_err(), "format 12 requires tracks");
    let mut bad_layer = saved;
    let front = front.to_string();
    let edge = view(&s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id.to_string();
    let track_line = text[start..end].lines().nth(1).unwrap();
    bad_layer.insert("board.json".into(), text.replace(track_line, &track_line.replace(&front, &edge)).into_bytes());
    assert!(decode_documents(&bad_layer).is_err(), "tracks must be on copper");
}

/// Segment and land geometry used by connectivity, including crossing and touching cases.
#[test]
fn connectivity_geometry_is_exact_at_boundaries() {
    let p = |x, y| Point { x_nm: x, y_nm: y };
    assert_eq!(segment_distance(p(0, 0), p(10, 0), p(5, -5), p(5, 5)), 0.);
    assert_eq!(segment_distance(p(0, 0), p(10, 0), p(0, 3), p(10, 3)), 3.);
    assert_eq!(segment_distance(p(0, 0), p(10, 0), p(13, 4), p(20, 4)), 5.);
    let square = [p(0, 0), p(10, 0), p(10, 10), p(0, 10)];
    assert!(inside(p(5, 5), &square) && inside(p(10, 10), &square) && !inside(p(11, 5), &square));
    let clockwise = [p(0, 0), p(0, 10), p(10, 10), p(10, 0)];
    assert!(inside(p(5, 5), &clockwise));
}

/// A short made by another edit, here moving a footprint onto a track of another net, refuses
/// only routes that touch that island. Unrelated routes still succeed.
#[test]
fn an_existing_short_blocks_only_routes_into_it() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let (id, position, rotation, moved, moved_net) = {
        let f = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
        (f.id.clone(), f.position, f.rotation, f.pads[0].center, f.pads[0].net.clone())
    };
    let target = view(&s).footprints().filter(|f| f.id != id).flat_map(|f| f.pads).find(|p| p.net.is_some() && p.net != moved_net).unwrap().center;
    let stub_end = Point { x_nm: target.x_nm, y_nm: target.y_nm + 3_000_000 };
    route(&mut s, front, &[target, stub_end]).unwrap();
    // Move the first footprint so its first pad lands on the stub's free end.
    let offset = (moved.x_nm - position.x_nm, moved.y_nm - position.y_nm);
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":stub_end.x_nm - offset.0,"y_nm":stub_end.y_nm - offset.1,
            "rotation":rotation,"expected_revision":s.revision()}),
    )
    .unwrap();
    assert!(short(s.project().unwrap(), &view(&s).tracks().iter().map(|t| t.id).collect()).is_some(), "the move created a short");
    route(&mut s, front, &[Point { x_nm: 0, y_nm: 30_000_000 }, Point { x_nm: 5_000_000, y_nm: 30_000_000 }])
        .expect("an unrelated route is not blamed for an existing short");
    let error = route(&mut s, front, &[stub_end, Point { x_nm: stub_end.x_nm + 2_000_000, y_nm: stub_end.y_nm }]).unwrap_err().to_string();
    assert!(error.contains("would connect nets"), "a route into the shorted island is refused: {error}");
}

/// Earlier formats reject a `tracks` field even when it is empty, so files stay canonical.
#[test]
fn an_empty_tracks_field_is_rejected_before_format_12() {
    let s = routed_fixture();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let text = String::from_utf8(saved["board.json"].clone()).unwrap();
    assert!(text.contains("\"format\": \"pcbcraft-board/10\""), "{text}");
    let list = if text.contains("  \"assignments\": [\n") { "  \"assignments\": [\n" } else { "  \"footprints\": [\n" };
    let mut with_empty = saved.clone();
    with_empty.insert("board.json".into(), text.replacen(list, &format!("  \"tracks\": [],\n{list}"), 1).into_bytes());
    assert!(decode_documents(&with_empty).is_err());
    assert!(decode_documents(&saved).is_ok());
}

/// A thin track that crosses a pad off-center, with neither end on it, still overlaps its copper:
/// running from one net's pad across another net's pad 0.2 mm from its center is refused as a
/// short. Edge distance also decides the grazing and clear cases.
#[test]
fn a_track_crossing_a_pad_off_center_touches_it() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let (a, b) = {
        let f = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
        (f.pads[0].center, f.pads[1].center)
    };
    // From inside pad `b`, 0.2 mm off its center line, straight across pad `a` and 1 mm beyond it.
    let offset = 200_000;
    let start = Point { x_nm: b.x_nm, y_nm: b.y_nm + offset };
    let beyond = Point { x_nm: a.x_nm + (a.x_nm - b.x_nm).signum() * 1_000_000, y_nm: a.y_nm + offset };
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let error = s
        .apply(ADD_TRACKS, &json!({"layer":front,"width_nm":100_000,"points":[start, beyond],"expected_revision":s.revision()}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("would connect nets"), "an off-center crossing is a short: {error}");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);

    let square =
        [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 600_000, y_nm: 0 }, Point { x_nm: 600_000, y_nm: 600_000 }, Point { x_nm: 0, y_nm: 600_000 }];
    let crossing = Track {
        id: Uuid::from_u128(1),
        layer: front,
        width_nm: 100_000,
        start: Point { x_nm: -1_000_000, y_nm: 500_000 },
        end: Point { x_nm: 2_000_000, y_nm: 500_000 },
    };
    assert!(touches_land(&crossing, &square), "crossing near an edge");
    let grazing = Track { start: Point { x_nm: -1_000_000, y_nm: 640_000 }, end: Point { x_nm: 2_000_000, y_nm: 640_000 }, ..crossing.clone() };
    assert!(touches_land(&grazing, &square), "copper within half the width of an edge");
    let clear = Track { start: Point { x_nm: -1_000_000, y_nm: 700_000 }, end: Point { x_nm: 2_000_000, y_nm: 700_000 }, ..crossing };
    assert!(!touches_land(&clear, &square), "copper clear of the land");
}

/// A track that crosses a pad with both ends outside its land joins that pad's copper island, and
/// leaves other pads out of it. A crossing like that over pads on two different nets is refused.
#[test]
fn a_track_crossing_a_pad_with_both_ends_outside_joins_its_island() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let (footprint, a, b) = {
        let f = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
        (f.id.clone(), (f.pads[0].number.clone(), f.pads[0].center), (f.pads[1].number.clone(), f.pads[1].center))
    };
    // Vertical, 0.2 mm off the center of pad `a`, starting and ending 1 mm outside its 0.6 mm land.
    let x = a.1.x_nm + (a.1.x_nm - b.1.x_nm).signum() * 200_000;
    let crossing = [Point { x_nm: x, y_nm: a.1.y_nm - 1_000_000 }, Point { x_nm: x, y_nm: a.1.y_nm + 1_000_000 }];
    s.apply(ADD_TRACKS, &json!({"layer":front,"width_nm":100_000,"points":crossing,"expected_revision":s.revision()})).unwrap();
    let track = view(&s).tracks()[0].id;
    let copper = view(&s).copper();
    assert_eq!(copper.pads[&(footprint.clone(), a.0.clone())], copper.tracks[&track], "the crossed pad joins the track's island");
    assert_ne!(copper.pads[&(footprint.clone(), b.0.clone())], copper.tracks[&track], "the neighboring pad stays separate");

    // Horizontal, 0.2 mm off both pad centers, starting and ending 1 mm outside the pair.
    let (left, right) = if a.1.x_nm < b.1.x_nm { (a.1, b.1) } else { (b.1, a.1) };
    let y = a.1.y_nm + 200_000;
    let across = [Point { x_nm: left.x_nm - 1_000_000, y_nm: y }, Point { x_nm: right.x_nm + 1_000_000, y_nm: y }];
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let error =
        s.apply(ADD_TRACKS, &json!({"layer":front,"width_nm":100_000,"points":across,"expected_revision":s.revision()})).unwrap_err().to_string();
    assert!(error.contains("would connect nets"), "crossing two nets' pads is a short: {error}");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}
