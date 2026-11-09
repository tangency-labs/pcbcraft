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

/// Add a route with layer changes and the default via size.
fn route_with_vias(session: &mut Session, layer: Uuid, points: &[Point], changes: &[(usize, Uuid)]) -> Result<u64, CommandError> {
    let changes: Vec<_> = changes.iter().map(|(at, layer)| json!({"at":at,"layer":layer})).collect();
    session.apply(
        ADD_TRACKS,
        &json!({"layer":layer,"width_nm":DEFAULT_WIDTH_NM,"points":points,"layer_changes":changes,
            "via_diameter_nm":DEFAULT_VIA_DIAMETER_NM,"via_drill_nm":DEFAULT_VIA_DRILL_NM,"expected_revision":session.revision()}),
    )
}

/// A route can leave a front pad, drop through a via to the back, cross there, and come back up
/// through a second via into the other pad. Vias join copper on every layer, so the airwire
/// clears. The route is one edit, saves vias beside tracks, reopens identically and undoes whole.
#[test]
fn vias_carry_a_route_between_layers_in_one_edit() {
    let mut s = routed_fixture();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let line = view(&s).ratsnest().unwrap().lines[0].clone();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let (a, b) = (line.from.position, line.to.position);
    let y = a.y_nm + 2_000_000;
    let points = [a, Point { x_nm: a.x_nm, y_nm: y }, Point { x_nm: b.x_nm, y_nm: y }, b];
    let revision = s.revision();
    route_with_vias(&mut s, front, &points, &[(1, back), (2, front)]).unwrap();
    assert_eq!(s.revision(), revision + 1, "segments and vias land in one edit");
    let layers: Vec<_> = {
        let mut tracks = view(&s).tracks().to_vec();
        tracks.sort_by_key(|track| (track.start.y_nm, track.start.x_nm == a.x_nm));
        tracks.iter().map(|track| (track.start, track.layer)).collect()
    };
    assert_eq!(layers.len(), 3);
    assert_eq!(view(&s).tracks().iter().filter(|track| track.layer == back).count(), 1, "only the crossing runs on the back");
    let vias = view(&s).vias().to_vec();
    assert_eq!(vias.len(), 2);
    assert!(vias.iter().all(|via| via.diameter_nm == DEFAULT_VIA_DIAMETER_NM && via.drill_nm == DEFAULT_VIA_DRILL_NM));
    let at: Vec<_> = vias.iter().map(|via| via.at).collect();
    assert!(at.contains(&points[1]) && at.contains(&points[2]), "vias sit at the layer-change vertices");
    let on_net = |lines: &[super::super::ratsnest::Airwire]| lines.iter().filter(|l| l.net == line.net).count();
    assert_eq!(on_net(&view(&s).ratsnest().unwrap().lines), 1, "the via route joins both pads");

    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let text = String::from_utf8(saved["board.json"].clone()).unwrap();
    assert!(text.contains("\"format\": \"pcbcraft-board/12\"") && text.contains("  \"vias\": [\n"), "{text}");
    assert_eq!(text.matches("\"drill_nm\":300000").count(), 2, "one line per via");
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &json!({})).unwrap();

    let id = vias[0].id;
    s.apply(DELETE_TRACKS, &json!({"vias":[id],"expected_revision":s.revision()})).unwrap();
    assert_eq!(view(&s).vias().len(), 1);
    assert_eq!(view(&s).tracks().len(), 3, "deleting a via leaves its tracks");
    assert_eq!(on_net(&view(&s).ratsnest().unwrap().lines), 2, "without the via the layers no longer join");
}

/// A via joins pads on any layer, so one that lands on a pad of another net is a short.
#[test]
fn a_via_onto_another_nets_pad_is_refused() {
    let mut s = routed_fixture();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let footprint = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
    let (a, b) = (footprint.pads[0].center, footprint.pads[1].center);
    let y = a.y_nm + 2_000_000;
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let error = route_with_vias(
        &mut s,
        front,
        &[a, Point { x_nm: a.x_nm, y_nm: y }, Point { x_nm: b.x_nm, y_nm: y }, b, Point { x_nm: b.x_nm + 1_000_000, y_nm: b.y_nm }],
        &[(1, back), (3, front)],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("would connect nets"), "{error}");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Layer changes must sit at increasing vertices after the first, change layer, and carry a
/// valid via size; deleting needs at least one item. Every refusal leaves the board unchanged.
#[test]
fn invalid_layer_changes_and_deletes_change_nothing() {
    let mut s = routed_fixture();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let p = |x| Point { x_nm: x, y_nm: 20_000_000 };
    let points = [p(0), p(2_000_000), p(4_000_000), p(6_000_000)];
    let revision = s.revision();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    for changes in [vec![(0, back)], vec![(4, back)], vec![(3, front)], vec![(2, back), (1, front)], vec![(1, back), (1, front)], vec![(1, front)]] {
        assert!(route_with_vias(&mut s, front, &points, &changes).is_err(), "{changes:?}");
    }
    let base =
        json!({"layer":front,"width_nm":DEFAULT_WIDTH_NM,"points":points,"layer_changes":[{"at":1,"layer":back}],"expected_revision":revision});
    assert!(s.apply(ADD_TRACKS, &base).is_err(), "via sizes are required with layer changes");
    let mut wide_drill = base.clone();
    wide_drill["via_diameter_nm"] = json!(400_000);
    wide_drill["via_drill_nm"] = json!(400_000);
    assert!(s.apply(ADD_TRACKS, &wide_drill).is_err(), "the drill must be smaller than the pad");
    assert!(s.apply(DELETE_TRACKS, &json!({"expected_revision":revision})).is_err());
    assert!(s.apply(DELETE_TRACKS, &json!({"vias":[Uuid::from_u128(3)],"expected_revision":revision})).is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// A route may end on a via, ready for the other layer.
#[test]
fn a_route_can_end_on_a_via() {
    let mut s = routed_fixture();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let end = Point { x_nm: 2_000_000, y_nm: 20_000_000 };
    route_with_vias(&mut s, front, &[Point { x_nm: 0, y_nm: 20_000_000 }, end], &[(1, back)]).unwrap();
    assert_eq!(view(&s).tracks().len(), 1);
    assert_eq!(view(&s).vias().iter().map(|via| via.at).collect::<Vec<_>>(), vec![end]);
}
