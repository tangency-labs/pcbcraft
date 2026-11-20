//! Original command-built fixtures for shove. Every reached shove is committed through
//! `pcb.shove_tracks`, which must land exactly the planned copper in one undo step; full DRC and
//! the exhaustive clearance reference must then report nothing about the changed copper, and
//! no connection may be lost.
use super::{
    shove::{DEFAULT_BUDGET, MAX_DEPTH, MAX_MOVED, Request, Shove, Shover},
    *,
};
use crate::{
    ADD_TRACKS, ADD_VIA,
    board::{
        drc::{Kind, tests::outline},
        geometry::Contour,
        track::{SET_COPPER_LOCKED, Track, Via, tests::copper_layers, tests::routed_fixture, tests::view},
    },
    persistence::encode_documents,
};
use std::collections::{BTreeMap, BTreeSet};

const MM: i64 = 1_000_000;
const WIDTH: i64 = 250_000;

/// A point in board nanometres.
fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// The routed fixture inside a closed Board Outline from (-5, -5) mm to (25, 35) mm. Its pads
/// sit at y = 2 mm, so the area above y = 6 mm is free copper space.
fn board() -> Session {
    let mut s = routed_fixture();
    outline(&mut s);
    s
}

/// The front copper layer.
fn layer(s: &Session) -> Uuid {
    copper_layers(s)[0]
}

/// Add a straight track path of the default width and return its new track identities in order.
fn track(s: &mut Session, points: &[Point]) -> Vec<Uuid> {
    let before: BTreeSet<_> = view(s).tracks().iter().map(|t| t.id).collect();
    let params = json!({"layer":layer(s),"width_nm":WIDTH,"points":points,"expected_revision":s.revision()});
    s.apply(ADD_TRACKS, &params).unwrap();
    points
        .windows(2)
        .map(|leg| view(s).tracks().iter().find(|t| !before.contains(&t.id) && t.start == leg[0] && t.end == leg[1]).unwrap().id)
        .collect()
}

/// Add a standalone 0.6 mm via and return its identity.
fn via(s: &mut Session, at: Point) -> Uuid {
    let params = json!({"at":at,"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()});
    s.apply(ADD_VIA, &params).unwrap();
    view(s).vias().iter().find(|v| v.at == at).unwrap().id
}

/// Parameters for a route shove on the front layer.
fn route(s: &Session, points: &[Point]) -> Value {
    json!({"layer":layer(s),"width_nm":WIDTH,"points":points})
}

/// Plan through the read-only command and decode the plan fields the tests compare.
fn plan(s: &mut Session, params: &Value) -> Value {
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let answer = s.execute(PLAN_SHOVE, params).unwrap();
    assert_eq!(s.revision(), revision, "planning never edits");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    answer["shove_plan"].clone()
}

/// Plan a route shove through the read-only command.
fn plan_route_shove(s: &mut Session, points: &[Point]) -> Value {
    let params = route(s, points);
    plan(s, &params)
}

/// Commit a route shove and check it as [`commit_and_agree`] does.
fn commit_route(s: &mut Session, points: &[Point]) -> BTreeSet<String> {
    let params = route(s, points);
    commit_and_agree(s, &params)
}

/// The outcome kind of a decoded plan.
fn kind(plan: &Value) -> &str {
    plan["outcome"]["kind"].as_str().unwrap()
}

/// A route shove request on the front layer.
fn request(s: &Session, points: &[Point]) -> Request {
    Request { layer: layer(s), shover: Shover::Route { points: points.to_vec(), width_nm: WIDTH } }
}

/// Plan directly over a fresh query, bypassing the command.
fn solve(s: &Session, request: &Request, budget: usize) -> Shove {
    let query = s.clearance_query_with_rules(view(s).target, &json!({})).unwrap();
    s.plan_shove(&query, request, budget, &|| false).unwrap()
}

/// Tracks and vias by identity.
type Copper = (BTreeMap<Uuid, Track>, BTreeMap<Uuid, Via>);

/// The board's tracks and vias by identity.
fn copper(s: &Session) -> Copper {
    (view(s).tracks().iter().map(|t| (t.id, t.clone())).collect(), view(s).vias().iter().map(|v| (v.id, v.clone())).collect())
}

/// Commit through `pcb.shove_tracks` as one undo step, require the board to change exactly as
/// planned, then require full DRC and the exhaustive reference to report no clearance, short,
/// edge or width finding about the changed copper, and no lost connection. Undo restores the
/// saved bytes exactly. Returns the changed copper's identities.
fn commit_and_agree(s: &mut Session, params: &Value) -> BTreeSet<String> {
    let planned = plan(s, params);
    assert_eq!(kind(&planned), "reached", "{planned}");
    let (tracks, vias) = copper(s);
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let unconnected = count(s, Kind::Unconnected);
    let revision = s.revision();
    let mut params = params.clone();
    params["expected_revision"] = json!(s.revision());
    s.apply(SHOVE_TRACKS, &params).unwrap();
    assert_eq!(s.revision(), revision + 1, "one transaction");
    let (after, after_vias) = copper(s);
    let changed: BTreeSet<String> = after
        .values()
        .filter(|t| tracks.get(&t.id) != Some(*t))
        .map(|t| t.id.to_string())
        .chain(after_vias.values().filter(|v| vias.get(&v.id) != Some(*v)).map(|v| v.id.to_string()))
        .collect();
    // The board changed exactly as planned: reshaped and moved identities, added and removed counts.
    for ends in planned["reshaped"].as_array().unwrap().iter().chain(planned["dragged"].as_array().unwrap()) {
        let id: Uuid = serde_json::from_value(ends["track"].clone()).unwrap();
        let t = &after[&id];
        assert_eq!((json!(t.start), json!(t.end)), (ends["start"].clone(), ends["end"].clone()));
    }
    for moved in planned["vias"].as_array().unwrap() {
        let id: Uuid = serde_json::from_value(moved["via"].clone()).unwrap();
        assert_eq!(json!(after_vias[&id].at), moved["at"]);
    }
    for removed in planned["removed"].as_array().unwrap() {
        assert!(!after.contains_key(&serde_json::from_value::<Uuid>(removed.clone()).unwrap()));
    }
    let route_legs = planned["points"].as_array().unwrap().len().saturating_sub(1);
    assert_eq!(after.len(), tracks.len() + route_legs + planned["added"].as_array().unwrap().len() - planned["removed"].as_array().unwrap().len());
    assert!(planned["moved"].as_u64().unwrap() as usize <= MAX_MOVED);
    let report = view(s).check_design(view(s).effective_rules().values);
    for violation in &report.violations {
        if matches!(violation.kind, Kind::Clearance | Kind::Short | Kind::EdgeClearance | Kind::OutsideBoard | Kind::TrackWidth) {
            assert!(!violation.items.iter().any(|item| changed.contains(item)), "{violation:?}");
        }
    }
    assert!(count(s, Kind::Unconnected) <= unconnected, "no connection is lost");
    let q = s.clearance_query_with_rules(view(s).target, &json!({})).unwrap();
    let objects = q.objects();
    for d in q.exhaustive_pairs().decisions.iter().filter(|d| d.clearance_violation || d.conflicting_nets) {
        let a = d.first.map(|i| objects[i].name.clone()).unwrap_or_default();
        assert!(!changed.contains(&a) && !changed.contains(&objects[d.second].name), "{a} / {}", objects[d.second].name);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved, "one Undo restores every shoved item");
    s.apply("edit.redo", &json!({})).unwrap();
    changed
}

/// The number of DRC findings of one kind.
fn count(s: &Session, kind: Kind) -> usize {
    view(s).check_design(view(s).effective_rules().values).counts.get(&kind).copied().unwrap_or(0)
}

/// The smallest copper gap between a planned route and the named source object after commit.
fn gap(s: &Session, points: &[Point], object: &str) -> f64 {
    let q = s.clearance_query_with_rules(view(s).target, &json!({})).unwrap();
    let obstacle = q.objects().iter().find(|o| o.name == object).unwrap().contour.clone();
    points.windows(2).map(|leg| Contour::segment(leg[0], leg[1], WIDTH).unwrap().gap(&obstacle)).fold(f64::INFINITY, f64::min)
}

/// A route laid 0.4 mm from a parallel track pushes it aside to exactly the clearance, keeping
/// the track's ends, identity and axis posture; DRC and the exhaustive reference agree.
#[test]
fn a_route_pushes_a_parallel_track_to_the_clearance_and_agrees_with_drc() {
    let mut s = board();
    let [wall] = track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)])[..] else { panic!() };
    let line = [p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "reached", "{planned}");
    assert_eq!(planned["depth"], 1);
    assert!(planned["probes"].as_u64().unwrap() as usize <= DEFAULT_BUDGET);
    let changed = commit_route(&mut s, &line);
    assert!(changed.contains(&wall.to_string()), "the track keeps its identity");
    let tracks = view(&s).tracks().to_vec();
    let pushed: Vec<&Track> = tracks.iter().filter(|t| t.id == wall || changed.contains(&t.id.to_string()) && t.start.y_nm < 10_100_000).collect();
    assert!(pushed.iter().all(|t| t.start.x_nm == t.end.x_nm || t.start.y_nm == t.end.y_nm), "{pushed:?}");
    let ends: Vec<_> = pushed.iter().flat_map(|t| [t.start, t.end]).collect();
    assert!(ends.contains(&p(2 * MM, 10 * MM)) && ends.contains(&p(18 * MM, 10 * MM)), "the chain's ends stay put");
    let names: Vec<String> = pushed.iter().map(|t| t.id.to_string()).collect();
    let closest = names.iter().map(|n| gap(&s, &line, n)).fold(f64::INFINITY, f64::min);
    assert!((200_000. ..200_010.).contains(&closest), "gap {closest}");
}

/// Pushed copper pushes in turn: two stacked tracks both give way, at depth two.
#[test]
fn pushed_tracks_push_the_next_track() {
    let mut s = board();
    track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)]);
    track(&mut s, &[p(2 * MM, 9_550_000), p(18 * MM, 9_550_000)]);
    let line = [p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "reached", "{planned}");
    assert_eq!(planned["depth"], 2);
    commit_route(&mut s, &line);
}

/// A standalone via moves the least distance straight away from the route; a via with a track
/// ending on it moves and the track follows, keeping its far end.
#[test]
fn vias_move_and_their_tracks_follow() {
    let mut s = board();
    let lone = via(&mut s, p(8 * MM, 10_200_000));
    let joined = via(&mut s, p(12 * MM, 10_200_000));
    let [tail] = track(&mut s, &[p(12 * MM, 10_200_000), p(12 * MM, 6 * MM)])[..] else { panic!() };
    let line = [p(5 * MM, 10_600_000), p(15 * MM, 10_600_000)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "reached", "{planned}");
    let changed = commit_route(&mut s, &line);
    assert!(changed.contains(&lone.to_string()) && changed.contains(&joined.to_string()));
    let at = |id: Uuid| view(&s).vias().iter().find(|v| v.id == id).unwrap().at;
    assert_eq!(at(lone).x_nm, 8 * MM, "straight down, the least distance");
    assert!(at(lone).y_nm < 10_200_000);
    let follower = view(&s).tracks().iter().find(|t| t.id == tail).unwrap().clone();
    assert!([follower.start, follower.end].contains(&p(12 * MM, 6 * MM)), "{follower:?}");
    assert!([follower.start, follower.end].contains(&at(joined)), "{follower:?}");
}

/// Locked tracks, pads and copper joined in mid-span never move. A blocked route reports its
/// longest legal prefix and the blocking copper, the commit is refused with nothing changed,
/// and the reported prefix commits cleanly.
#[test]
fn immovable_copper_blocks_and_the_legal_prefix_commits() {
    let mut s = board();
    let [locked] = track(&mut s, &[p(10 * MM, 8 * MM), p(10 * MM, 14 * MM)])[..] else { panic!() };
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[locked],"locked":true,"expected_revision":s.revision()})).unwrap();
    let line = [p(4 * MM, 11 * MM), p(16 * MM, 11 * MM)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "blocked", "{planned}");
    assert_eq!(planned["blocker"], json!(locked.to_string()));
    let last: Point = serde_json::from_value(planned["outcome"]["last_legal"].clone()).unwrap();
    assert_eq!(last.y_nm, 11 * MM);
    assert!(last.x_nm <= 10 * MM - 125_000 - 200_000 - 125_000 && last.x_nm > 9 * MM, "{last:?}");
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut refused = route(&s, &line);
    refused["expected_revision"] = json!(s.revision());
    let error = s.apply(SHOVE_TRACKS, &refused).unwrap_err();
    assert!(error.to_string().contains(&locked.to_string()), "{error}");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before, "a refused shove changes nothing");
    let prefix: Vec<Point> = serde_json::from_value(planned["points"].clone()).unwrap();
    commit_route(&mut s, &prefix);

    // A pad of another net blocks; so does a track with a T joining it in mid-span.
    let mut s = board();
    let planned = plan_route_shove(&mut s, &[p(6 * MM, 2 * MM), p(14 * MM, 2 * MM)]);
    assert_eq!(kind(&planned), "blocked", "{planned}");
    assert_eq!(planned["blocker"], json!("footprint-8:1"));
    let mut s = board();
    let [bar] = track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)])[..] else { panic!() };
    let [stem] = track(&mut s, &[p(10 * MM, 10 * MM), p(10 * MM, 7 * MM)])[..] else { panic!() };
    let planned = plan_route_shove(&mut s, &[p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)]);
    assert_eq!(kind(&planned), "blocked", "{planned}");
    // The bar is pinned by the T; the stem's end at the bar cannot move either.
    assert!([json!(bar.to_string()), json!(stem.to_string())].contains(&planned["blocker"]), "{planned}");
}

/// The board edge never moves: a track that would be pushed past the edge clearance goes round
/// the route's other side instead, keeping its ends and the edge clearance DRC checks.
#[test]
fn the_board_edge_does_not_move() {
    let mut s = board();
    let [edge] = track(&mut s, &[p(2 * MM, 34_550_000), p(18 * MM, 34_550_000)])[..] else { panic!() };
    let line = [p(5 * MM, 34_150_000), p(15 * MM, 34_150_000)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "reached", "{planned}");
    let changed = commit_route(&mut s, &line);
    assert!(changed.contains(&edge.to_string()));
    let lowest = view(&s).tracks().iter().filter(|t| changed.contains(&t.id.to_string())).flat_map(|t| [t.start.y_nm, t.end.y_nm]).min().unwrap();
    assert!(lowest < 34_150_000, "the track passes under the route, away from the edge");
}

/// Propagation is bounded: a stack deeper than the depth limit stops as incomplete, naming the
/// copper being pushed, and moves nothing. Small budgets are never exceeded.
#[test]
fn propagation_and_budget_are_bounded() {
    let mut s = board();
    for k in 0..(MAX_DEPTH as i64 + 2) {
        // Long enough that every bump, each 0.45 mm wider than the one it gives way to, fits.
        let y = 10 * MM - k * 450_000;
        track(&mut s, &[p(-3 * MM, y), p(23 * MM, y)]);
    }
    let line = [p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let planned = plan_route_shove(&mut s, &line);
    assert_eq!(kind(&planned), "incomplete", "{planned}");
    assert!(planned["blocker"].is_string());
    assert_eq!(planned["reshaped"], json!([]));
    assert_eq!(planned["points"].as_array().unwrap().len(), 1);
    for budget in [1, 2, 5, 17, 40] {
        let result = solve(&s, &request(&s, &line), budget);
        assert!(result.probes <= budget);
        assert_eq!(result.outcome, Outcome::Incomplete);
    }
}

/// The same board and request give the same plan from fresh and reused queries; cancellation
/// returns the head alone; a stale query or revision plans nothing.
#[test]
fn shoves_are_deterministic_cancelable_and_stale_safe() {
    let mut s = board();
    track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)]);
    via(&mut s, p(8 * MM, 10_900_000));
    let line = [p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let query = s.clearance_query_with_rules(view(&s).target, &json!({})).unwrap();
    let first = s.plan_shove(&query, &request(&s, &line), DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(first.outcome, Outcome::Reached);
    assert_eq!(s.plan_shove(&query, &request(&s, &line), DEFAULT_BUDGET, &|| false).unwrap(), first);
    assert_eq!(solve(&s, &request(&s, &line), DEFAULT_BUDGET), first);
    let canceled = s.plan_shove(&query, &request(&s, &line), DEFAULT_BUDGET, &|| true).unwrap();
    assert_eq!((canceled.outcome, canceled.points.len(), canceled.moved), (Outcome::Canceled, 1, 0));
    let mut stale = route(&s, &line);
    stale["expected_revision"] = json!(s.revision() + 7);
    assert_eq!(kind(&plan(&mut s, &stale)), "stale");
    assert!(matches!(s.apply(SHOVE_TRACKS, &stale), Err(CommandError::StaleRevision { .. })));
    s.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    assert_eq!(s.plan_shove(&query, &request(&s, &line), DEFAULT_BUDGET, &|| false).unwrap().outcome, Outcome::Stale);
}

/// A dragged track pushes another track; the drag lands with the push in one undo step.
#[test]
fn a_dragged_track_pushes_its_neighbour() {
    let mut s = board();
    let [dragged] = track(&mut s, &[p(5 * MM, 12 * MM), p(15 * MM, 12 * MM)])[..] else { panic!() };
    let [other] = track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)])[..] else { panic!() };
    let params = json!({"layer":layer(&s),"tracks":[{"track":dragged,"start":p(5 * MM, 10_400_000),"end":p(15 * MM, 10_400_000)}]});
    let planned = plan(&mut s, &params);
    assert_eq!(kind(&planned), "reached", "{planned}");
    let changed = commit_and_agree(&mut s, &params);
    assert!(changed.contains(&dragged.to_string()) && changed.contains(&other.to_string()));
    let moved = view(&s).tracks().iter().find(|t| t.id == dragged).unwrap().clone();
    assert_eq!((moved.start, moved.end), (p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)));
}

/// Refused requests change nothing and charge nothing: both or neither of points and tracks,
/// a width with a drag, a locked dragged track and a foreign layer.
#[test]
fn invalid_shoves_are_refused() {
    let mut s = board();
    let [t] = track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)])[..] else { panic!() };
    let line = json!([p(5 * MM, 12 * MM), p(15 * MM, 12 * MM)]);
    let ends = json!([{"track":t,"start":p(2 * MM, 11 * MM),"end":p(18 * MM, 11 * MM)}]);
    let layer = layer(&s);
    for params in [
        json!({"layer":layer}),
        json!({"layer":layer,"points":line,"tracks":ends}),
        json!({"layer":layer,"tracks":ends,"width_nm":WIDTH}),
        json!({"layer":Uuid::nil(),"points":line}),
        json!({"layer":copper_layers(&s)[1],"tracks":ends}),
    ] {
        assert!(s.execute(PLAN_SHOVE, &params).is_err(), "{params}");
    }
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[t],"locked":true,"expected_revision":s.revision()})).unwrap();
    assert!(s.execute(PLAN_SHOVE, &json!({"layer":layer,"tracks":ends})).is_err());
}

/// The moved-copper bound holds even within the depth bound: seven tracks on each side of the
/// route would each gain a bump, more segments than one shove may change.
#[test]
fn the_moved_copper_bound_stops_a_wide_shove() {
    let mut s = board();
    for k in 0..7 {
        track(&mut s, &[p(-3 * MM, 10 * MM - k * 450_000), p(23 * MM, 10 * MM - k * 450_000)]);
        track(&mut s, &[p(-3 * MM, 10_800_000 + k * 450_000), p(23 * MM, 10_800_000 + k * 450_000)]);
    }
    let crossing = [p(5 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let planned = plan_route_shove(&mut s, &crossing);
    assert_eq!(kind(&planned), "incomplete", "{planned}");
    assert_eq!((planned["moved"].as_u64(), planned["points"].as_array().map(Vec::len)), (Some(0), Some(1)));
    assert!(planned["blocker"].is_string());
}

/// A chain with a 45° leg is pushed round an octagon and keeps H/V/45° legs.
#[test]
fn a_diagonal_chain_keeps_its_posture() {
    let mut s = board();
    let chain = [p(2 * MM, 8 * MM), p(4 * MM, 10 * MM), p(18 * MM, 10 * MM)];
    track(&mut s, &chain);
    let line = [p(6 * MM, 10_400_000), p(15 * MM, 10_400_000)];
    let changed = commit_route(&mut s, &line);
    let pushed: Vec<Track> = view(&s).tracks().iter().filter(|t| changed.contains(&t.id.to_string()) && t.start.y_nm < 10_300_000).cloned().collect();
    assert!(pushed.len() > 2, "{pushed:?}");
    assert!(pushed.iter().all(|t| Posture::Diagonal.allows(t.start, t.end)), "{pushed:?}");
}

/// Discovery, runtime validation and the solver agree on the shove contracts: the route and
/// drag forms exclude each other, a drag lists at most 256 tracks, and only the edit requires
/// `expected_revision`, while the planner keeps it optional.
#[test]
fn shove_contracts_publish_their_forms_drag_limit_and_revision() {
    let described = |id: &str| crate::command(id).unwrap().to_json()["params"].clone();
    let forms = json!([
        {"required":["points"],"not":{"anyOf":[{"required":["tracks"]}]}},
        {"required":["tracks"],"not":{"anyOf":[{"required":["points"]},{"required":["width_nm"]}]}}
    ]);
    for id in [PLAN_SHOVE, SHOVE_TRACKS] {
        let params = described(id);
        assert_eq!(params["oneOf"], forms, "{id}");
        assert_eq!(params["properties"]["tracks"]["maxItems"], json!(shove::MAX_DRAGGED));
        assert_eq!(shove::MAX_DRAGGED, 256);
    }
    assert_eq!(described(PLAN_SHOVE)["required"], json!(["layer"]));
    assert_eq!(described(SHOVE_TRACKS)["required"], json!(["layer", "expected_revision"]));
    assert_eq!(described(crate::RESHAPE_TRACKS)["properties"]["tracks"]["maxItems"], json!(crate::board::track::MAX_TRACKS));

    let mut s = board();
    let [t] = track(&mut s, &[p(2 * MM, 10 * MM), p(18 * MM, 10 * MM)])[..] else { panic!() };
    let layer = layer(&s);
    let line = json!([p(5 * MM, 12 * MM), p(15 * MM, 12 * MM)]);
    let ends = json!([{"track":t,"start":p(2 * MM, 11 * MM),"end":p(18 * MM, 11 * MM)}]);
    let many: Vec<Value> = (0..=shove::MAX_DRAGGED as i64)
        .map(|k| json!({"track":Uuid::from_u128(k as u128 + 1),"start":p(0, k * 1_000),"end":p(1_000, k * 1_000)}))
        .collect();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for (params, expected) in [
        (json!({"layer":layer,"points":line,"tracks":ends,"expected_revision":revision}), "takes exactly one of"),
        (json!({"layer":layer,"tracks":ends,"width_nm":WIDTH,"expected_revision":revision}), "takes exactly one of"),
        (json!({"layer":layer,"expected_revision":revision}), "takes exactly one of"),
        (json!({"layer":layer,"tracks":many,"expected_revision":revision}), "at most 256 tracks"),
    ] {
        for id in [PLAN_SHOVE, SHOVE_TRACKS] {
            let error = s.execute(id, &params).unwrap_err();
            assert!(matches!(error, CommandError::InvalidParams(_)) && error.to_string().contains(expected), "{id} {params}: {error}");
        }
    }
    let error = s.execute(SHOVE_TRACKS, &json!({"layer":layer,"tracks":ends})).unwrap_err();
    assert!(error.to_string().contains("expected_revision"), "{error}");
    assert_eq!(kind(&plan(&mut s, &json!({"layer":layer,"tracks":ends}))), "reached");
    assert_eq!((s.revision(), encode_documents(s.snapshot().unwrap()).unwrap()), (revision, before), "refusals change nothing");
}
