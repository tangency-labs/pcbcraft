//! Original command-built fixtures: every plan is committed through `pcb.add_tracks` and must
//! agree with full DRC and the exhaustive clearance reference about the new copper.
use super::*;
use crate::{
    ADD_TRACKS, ADD_VIA, INITIALIZE_BOARD_SETUP,
    board::{
        clearance::Results,
        drc::{
            Kind,
            tests::{outline, outline_square},
        },
        geometry::Contour,
        track::{SET_COPPER_LOCKED, tests::copper_layers, tests::routed_fixture, tests::view},
    },
    persistence::encode_documents,
};
use std::collections::BTreeSet;

const WIDTH: i64 = 250_000;

/// Suspension preserves the exact synchronous plan, with bounded probe progress and no edits.
#[test]
fn captured_planning_yields_and_matches_the_shared_solver() {
    let mut s = board();
    add(&mut s, ADD_VIA, json!({"at":p(4_000_000,10_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let request = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 11_000_000));
    let expected = plan(&s, &request);
    for batch in [1, 7, 256] {
        let context = s.route_planning(view(&s).target).unwrap();
        let mut job = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
        assert!(!job.step(0));
        assert!(job.take_result().unwrap().is_none());
        let mut completed = false;
        for _ in 0..100_000 {
            let before = job.progress().completed;
            completed = job.step(batch);
            assert!(job.progress().completed <= before + batch);
            if completed {
                break;
            }
        }
        assert!(completed);
        assert_eq!(job.take_result().unwrap().unwrap(), expected);
        assert!(job.take_result().unwrap().is_none());
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    let context = s.route_planning(view(&s).target).unwrap();
    let mut abandoned = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
    assert!(!abandoned.step(1));
    drop(abandoned);
    let mut next = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
    while !next.step(256) {}
    assert_eq!(next.take_result().unwrap().unwrap(), expected);
    s.apply("project.rename", &json!({"name":"Changed while planning"})).unwrap();
    assert!(!next.source().is_current(&s));
    assert!(s.prepare_route_plan(&context, request, DEFAULT_BUDGET).is_err());
}

/// Warm capture does not start cold projection; its continuation
/// retains exact search progress and still carries revocable source authority.
#[test]
fn cached_planning_skips_cold_queries_and_resumes_exactly() {
    let mut s = board();
    add(&mut s, ADD_VIA, json!({"at":p(4_000_000,10_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let request = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 11_000_000));
    let context = s.route_planning(view(&s).target).unwrap();
    assert!(context.try_prepare_cached(request.clone(), DEFAULT_BUDGET).unwrap().is_none());
    let mut cold = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
    while !cold.step(8) {}
    let expected = cold.take_result().unwrap().unwrap();
    let mut warm = context.try_prepare_cached(request.clone(), DEFAULT_BUDGET).unwrap().unwrap();
    assert!(!warm.step(1), "the detour must yield before finishing");
    let progress = warm.progress().completed;
    while !warm.step(8) {}
    assert!(warm.progress().completed >= progress);
    assert_eq!(warm.take_result().unwrap().unwrap(), expected);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("project.rename", &json!({"name":"Changed while planning"})).unwrap();
    assert!(!warm.source().is_current(&s));
    assert!(s.prepare_route_plan(&context, request, DEFAULT_BUDGET).is_err());
}

fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

fn query(s: &Session) -> ClearanceQuery {
    s.clearance_query_with_rules(view(s).target, &json!({})).unwrap()
}

fn request(s: &Session, head: Point, target: Point) -> Request {
    Request {
        mode: Default::default(),
        merge_same_net: false,
        fixed: Vec::new(),
        head,
        target,
        layer: copper_layers(s)[0],
        origin: None,
        width_nm: WIDTH,
        posture: Posture::Diagonal,
        flipped: false,
        previous: Vec::new(),
    }
}

fn plan(s: &Session, request: &Request) -> Plan {
    s.plan_route(&query(s), request, DEFAULT_BUDGET, &|| false).unwrap()
}

/// The routed fixture inside a closed Board Outline from (-5, -5) mm to (25, 35) mm.
fn board() -> Session {
    let mut s = routed_fixture();
    outline(&mut s);
    s
}

fn pad(s: &Session, footprint: &str, number: &str) -> Point {
    view(s).footprints().find(|f| f.id == footprint).unwrap().pads.into_iter().find(|p| p.number == number).unwrap().center
}

fn add(s: &mut Session, command: &str, params: Value) {
    let mut params = params;
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Every leg follows the requested posture.
fn assert_posture(plan: &Plan, posture: Posture) {
    assert!(plan.points.windows(2).all(|leg| posture.allows(leg[0], leg[1])), "{:?}", plan.points);
}

/// Commit the plan's points with the shared command, then require full DRC and the exhaustive
/// pair reference to report no clearance, short, edge or width finding about the new copper.
/// Returns the new tracks' addresses.
fn commit_and_agree(s: &mut Session, plan: &Plan, layer: Uuid, width_nm: i64) -> BTreeSet<String> {
    let before: BTreeSet<_> = view(s).tracks().iter().map(|t| t.id).collect();
    add(s, ADD_TRACKS, json!({"layer":layer,"width_nm":width_nm,"points":plan.points}));
    let added: BTreeSet<String> = view(s).tracks().iter().filter(|t| !before.contains(&t.id)).map(|t| t.id.to_string()).collect();
    assert_eq!(added.len(), plan.points.len() - 1);
    let report = view(s).check_design(view(s).effective_rules().values);
    for violation in &report.violations {
        if matches!(violation.kind, Kind::Clearance | Kind::Short | Kind::EdgeClearance | Kind::OutsideBoard | Kind::TrackWidth) {
            assert!(!violation.items.iter().any(|item| added.contains(item)), "{violation:?}");
        }
    }
    let q = query(s);
    let names = |r: &Results| -> Vec<(String, String)> {
        let objects = q.objects();
        r.decisions
            .iter()
            .filter(|d| d.clearance_violation || d.conflicting_nets)
            .map(|d| (d.first.map(|i| objects[i].name.clone()).unwrap_or_default(), objects[d.second].name.clone()))
            .collect()
    };
    for (a, b) in names(&q.exhaustive_pairs()) {
        assert!(!added.contains(&a) && !added.contains(&b), "{a} / {b}");
    }
    added
}

/// The smallest copper gap between the plan's track and one source object.
fn gap_to(s: &Session, plan: &Plan, object: &str) -> f64 {
    let q = query(s);
    let obstacle = q.objects().iter().find(|o| o.name == object).unwrap().contour.clone();
    plan.points.windows(2).map(|leg| Contour::segment(leg[0], leg[1], WIDTH).unwrap().gap(&obstacle)).fold(f64::INFINITY, f64::min)
}

/// A route between two ACCEPTED pads goes round the unconnected pad between them, hugging it
/// at exactly the clearance rule plus at most a few nanometres of rounding, in 45° legs.
#[test]
fn a_route_hugs_a_pad_at_the_clearance_rule_and_agrees_with_drc() {
    let mut s = board();
    let (head, target) = (pad(&s, "footprint-6", "2"), pad(&s, "footprint-8", "2"));
    let plan = plan(&s, &request(&s, head, target));
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert_eq!((plan.points.first(), plan.points.last()), (Some(&head), Some(&target)));
    assert!(plan.probes <= DEFAULT_BUDGET);
    assert_posture(&plan, Posture::Diagonal);
    let gap = gap_to(&s, &plan, "footprint-6:1");
    assert!((200_000. ..200_010.).contains(&gap), "gap {gap}");
    let ratsnest = view(&s).ratsnest().unwrap().lines.len();
    let layer = copper_layers(&s)[0];
    commit_and_agree(&mut s, &plan, layer, WIDTH);
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), ratsnest - 1, "the route makes its connection");
}

/// An unconnected physical terminal must not acquire another pad's net just because
/// a speculative route overlaps it. The same obstacle rule applies in either direction.
#[test]
fn an_unconnected_pad_cannot_route_into_a_named_pad() {
    let mut s = board();
    let footprint = view(&s).footprints().find(|f| f.pads.iter().any(|p| p.net.is_some()) && f.pads.iter().any(|p| p.net.is_none())).unwrap();
    let named = footprint.pads.iter().find(|p| p.net.is_some()).unwrap().center;
    let unconnected = footprint.pads.iter().find(|p| p.net.is_none()).unwrap().center;
    for (head, target) in [(unconnected, named), (named, unconnected)] {
        let request = request(&s, head, target);
        let result = plan(&s, &request);
        assert_ne!(result.outcome, Outcome::Reached, "a route must not silently assign the other terminal: {result:?}");
        // Even a caller-supplied previous crossing cannot grant that exemption in confirmation.
        let reused = plan(&s, &Request { previous: vec![head, target], ..request });
        assert_ne!(reused.outcome, Outcome::Reached);
    }
    let layer = copper_layers(&s)[0];
    let end = p(named.x_nm, named.y_nm + 5_000_000);
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":WIDTH,"points":[named,end]}));
    let obstacle = view(&s).tracks().last().unwrap().id.to_string();
    let result = plan(&s, &request(&s, unconnected, p(named.x_nm + 2_000_000, named.y_nm + 4_000_000)));
    assert_eq!(result.outcome, Outcome::Reached, "{result:?}");
    assert!(gap_to(&s, &result, &obstacle) >= 200_000., "the unnamed route must go around the named track");
}

/// A free-space route goes round a standalone via in either posture and agrees with DRC.
#[test]
fn a_route_walks_around_a_via_in_both_postures() {
    for posture in [Posture::Diagonal, Posture::Orthogonal] {
        let mut s = board();
        add(&mut s, ADD_VIA, json!({"at":p(4_000_000, 10_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
        let mut request = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 11_000_000));
        request.posture = posture;
        let plan = plan(&s, &request);
        assert_eq!(plan.outcome, Outcome::Reached, "{posture:?} {plan:?}");
        assert_posture(&plan, posture);
        let via = view(&s).vias()[0].id.to_string();
        assert!(gap_to(&s, &plan, &via) >= 200_000.);
        commit_and_agree(&mut s, &plan, request.layer, WIDTH);
    }
}

/// A wall with a narrow gap admits a route that fits it and blocks a wider one, which stops at
/// its last legal point and names the wall.
#[test]
fn a_narrow_channel_admits_a_fitting_route_and_blocks_a_wider_one() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    add(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    let outline_layer = view(&s).layers().into_iter().find(|l| l.purpose == crate::board::setup::LayerPurpose::Outline).unwrap().id;
    add(
        &mut s,
        crate::ADD_DRAWING,
        json!({"layer":outline_layer,"closed":true,"points":[p(0, 0),p(20_000_000, 0),p(20_000_000, 10_000_000),p(0, 10_000_000)]}),
    );
    let layer = copper_layers(&s)[0];
    // Two walls from the board edges leave 0.62 mm of free copper gap at x = 10 mm.
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":200_000,"points":[p(10_000_000, 0),p(10_000_000, 4_600_000)]}));
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":200_000,"points":[p(10_000_000, 5_420_000),p(10_000_000, 10_000_000)]}));
    let mut fits = request(&s, p(2_000_000, 2_000_000), p(18_000_000, 8_000_000));
    fits.width_nm = 200_000;
    let through = s.plan_route(&query(&s), &fits, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(through.outcome, Outcome::Reached, "{through:?}");
    assert_posture(&through, Posture::Diagonal);
    let mut wide = fits.clone();
    wide.width_nm = 300_000;
    let blocked = s.plan_route(&query(&s), &wide, DEFAULT_BUDGET, &|| false).unwrap();
    let Outcome::Blocked { last_legal } = blocked.outcome else { panic!("{blocked:?}") };
    assert_eq!(blocked.points.last(), Some(&last_legal));
    assert!(last_legal.x_nm < 10_000_000, "{blocked:?}");
    assert!(blocked.blocker.is_some());
    let mut committed = Session::default();
    committed.open_snapshot(s.snapshot().unwrap());
    commit_and_agree(&mut committed, &through, layer, 200_000);
    if blocked.points.len() >= 2 {
        commit_and_agree(&mut s, &blocked, layer, 300_000);
    }
}

/// When the cheaper side of an obstacle leads into a dead end, the walk goes back and round
/// its other side instead of reporting the route blocked.
#[test]
fn a_dead_end_on_the_cheaper_side_is_walked_round_the_other_way() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    add(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    let outline_layer = view(&s).layers().into_iter().find(|l| l.purpose == crate::board::setup::LayerPurpose::Outline).unwrap().id;
    add(
        &mut s,
        crate::ADD_DRAWING,
        json!({"layer":outline_layer,"closed":true,"points":[p(0, 0),p(20_000_000, 0),p(20_000_000, 10_000_000),p(0, 10_000_000)]}),
    );
    let layer = copper_layers(&s)[0];
    // A large via whose track runs straight to the top edge. Going over the via is cheaper to a
    // target just above the head's line, but the track closes that way at the board edge.
    add(&mut s, ADD_VIA, json!({"at":p(10_000_000, 5_000_000),"via_diameter_nm":1_700_000,"via_drill_nm":800_000}));
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":200_000,"points":[p(10_000_000, 5_000_000),p(10_000_000, 0)]}));
    let plan = plan(&s, &request(&s, p(2_000_000, 5_000_000), p(18_000_000, 4_600_000)));
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.points.iter().any(|p| p.y_nm > 5_600_000), "goes under the via: {plan:?}");
    assert_posture(&plan, Posture::Diagonal);
    commit_and_agree(&mut s, &plan, layer, WIDTH);
}

/// A target enclosed by other copper is a dead end: the route is blocked at a legal point and
/// the legal prefix commits cleanly.
#[test]
fn a_dead_end_is_blocked_at_the_last_legal_point() {
    let mut s = board();
    let layer = copper_layers(&s)[0];
    let ring =
        [p(10_000_000, 20_000_000), p(14_000_000, 20_000_000), p(14_000_000, 24_000_000), p(10_000_000, 24_000_000), p(10_000_000, 20_000_000)];
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":200_000,"points":ring}));
    let plan = plan(&s, &request(&s, p(2_000_000, 22_000_000), p(12_000_000, 22_000_000)));
    let Outcome::Blocked { last_legal } = plan.outcome else { panic!("{plan:?}") };
    assert_eq!(plan.points.last(), Some(&last_legal));
    assert!(plan.blocker.is_some());
    let inside = (10_000_000..=14_000_000).contains(&last_legal.x_nm) && (20_000_000..=24_000_000).contains(&last_legal.y_nm);
    assert!(!inside, "{plan:?}");
    commit_and_agree(&mut s, &plan, layer, WIDTH);
}

/// The board edge stops a route exactly where its edge clearance is met, naming no copper.
#[test]
fn the_board_edge_stops_a_route_at_its_edge_clearance() {
    let mut s = board();
    let plan = plan(&s, &request(&s, p(5_000_000, 30_000_000), p(5_000_000, 40_000_000)));
    let edge = 35_000_000 - 300_000 - WIDTH / 2;
    assert_eq!(plan.outcome, Outcome::Blocked { last_legal: p(5_000_000, edge) }, "{plan:?}");
    assert_eq!(plan.blocker, None);
    let layer = copper_layers(&s)[0];
    commit_and_agree(&mut s, &plan, layer, WIDTH);
}

/// A closed contour inside the board is a cut-out. Its edge stops a route at the edge
/// clearance, as the outer board edge does, so no route passes through it; and a route that
/// would start inside the cut-out is blocked where it starts, because that is off the board.
#[test]
fn a_board_cut_out_stops_routes_and_cannot_start_one() {
    let mut s = board();
    outline_square(&mut s, 5, 15, 10);
    let across = plan(&s, &request(&s, p(2_000_000, 20_000_000), p(18_000_000, 20_000_000)));
    let Outcome::Blocked { last_legal } = across.outcome else { panic!("{across:?}") };
    let edge = 5_000_000 - 300_000 - WIDTH / 2;
    assert_eq!(last_legal.y_nm, 20_000_000, "{across:?}");
    assert!((edge - 10..=edge).contains(&last_legal.x_nm), "{across:?}");
    assert_eq!(across.blocker, None);
    let layer = copper_layers(&s)[0];
    let head = p(10_000_000, 20_000_000);
    let inside = plan(&s, &request(&s, head, p(2_000_000, 20_000_000)));
    assert_eq!(inside.outcome, Outcome::Blocked { last_legal: head }, "{inside:?}");
    assert_eq!(inside.points, vec![head]);
    commit_and_agree(&mut s, &across, layer, WIDTH);
}

/// Overlapping closed outlines are one board without cut-outs: a route in the area two of them
/// share is on the board and reaches its target, as it did before cut-outs existed.
#[test]
fn a_route_in_overlapping_outlines_is_on_the_board() {
    let mut s = board();
    outline_square(&mut s, 20, 10, 15);
    let layer = copper_layers(&s)[0];
    let shared = plan(&s, &request(&s, p(21_000_000, 15_000_000), p(24_000_000, 15_000_000)));
    assert_eq!(shared.outcome, Outcome::Reached, "{shared:?}");
    commit_and_agree(&mut s, &shared, layer, WIDTH);
}

/// A disjoint cut-out still blocks routing containment even when two other closed Board Outlines
/// overlap elsewhere on the board: routes cannot start inside or cross the cut-out, while routes
/// in the overlapping board area remain legal.
#[test]
fn a_cut_out_away_from_overlapping_outlines_still_blocks_routes() {
    let mut s = routed_fixture();
    outline_square(&mut s, -5, -5, 40);
    outline_square(&mut s, 5, 15, 10);
    outline_square(&mut s, 25, -5, 20);

    let head = p(8_000_000, 20_000_000);
    let inside = plan(&s, &request(&s, head, p(12_000_000, 20_000_000)));
    assert_eq!(inside.outcome, Outcome::Blocked { last_legal: head }, "{inside:?}");
    assert_eq!(inside.points, vec![head]);

    let across = plan(&s, &request(&s, p(2_000_000, 20_000_000), p(18_000_000, 20_000_000)));
    let Outcome::Blocked { last_legal } = across.outcome else { panic!("{across:?}") };
    let edge = 5_000_000 - 300_000 - WIDTH / 2;
    assert_eq!(last_legal.y_nm, 20_000_000, "{across:?}");
    assert!((edge - 10..=edge).contains(&last_legal.x_nm), "{across:?}");

    let shared = plan(&s, &request(&s, p(28_000_000, 10_000_000), p(32_000_000, 10_000_000)));
    assert_eq!(shared.outcome, Outcome::Reached, "{shared:?}");
}

/// Locked copper is an obstacle like any other: planning edits nothing, the route goes round,
/// and the committed route leaves the locked track's geometry and lock untouched.
#[test]
fn locked_copper_stays_unchanged() {
    let mut s = board();
    let layer = copper_layers(&s)[0];
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":WIDTH,"points":[p(4_000_000, 16_000_000),p(4_000_000, 22_000_000)]}));
    let locked = view(&s).tracks()[0].id;
    let board = view(&s).target.board;
    add(&mut s, SET_COPPER_LOCKED, json!({"board":board,"tracks":[locked],"locked":true}));
    let before = view(&s).tracks()[0].clone();
    let (bytes, revision) = (encode_documents(s.snapshot().unwrap()).unwrap(), s.revision());
    let plan = plan(&s, &request(&s, p(1_000_000, 19_000_000), p(7_000_000, 19_000_000)));
    assert_eq!((encode_documents(s.snapshot().unwrap()).unwrap(), s.revision()), (bytes, revision), "planning edits nothing");
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    commit_and_agree(&mut s, &plan, layer, WIDTH);
    let after = view(&s).tracks().iter().find(|t| t.id == locked).unwrap().clone();
    assert_eq!(after, before);
    assert!(after.locked);
}

/// The same request over the same board gives the same plan, from a reused or fresh query and
/// from a reopened copy of the project.
#[test]
fn plans_are_deterministic() {
    let s = board();
    let r = request(&s, pad(&s, "footprint-8", "1"), pad(&s, "footprint-11", "1"));
    let first = plan(&s, &r);
    assert_eq!(first.outcome, Outcome::Reached, "{first:?}");
    let q = query(&s);
    assert_eq!(s.plan_route(&q, &r, DEFAULT_BUDGET, &|| false).unwrap(), first);
    assert_eq!(s.plan_route(&q, &r, DEFAULT_BUDGET, &|| false).unwrap(), first);
    let mut reopened = Session::default();
    reopened.open_snapshot(s.snapshot().unwrap());
    assert_eq!(plan(&reopened, &r), first);
}

/// No budget is ever exceeded; a budget too small to finish reports an incomplete plan with
/// nothing to add, and a larger budget reaches the target.
#[test]
fn the_probe_budget_is_never_exceeded() {
    let s = board();
    let r = request(&s, pad(&s, "footprint-6", "2"), pad(&s, "footprint-8", "2"));
    let q = query(&s);
    let mut incomplete = 0;
    for budget in [1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144] {
        let plan = s.plan_route(&q, &r, budget, &|| false).unwrap();
        assert!(plan.probes <= budget, "{budget}: {plan:?}");
        if plan.outcome == Outcome::Incomplete {
            incomplete += 1;
            assert_eq!(plan.points, vec![r.head]);
        }
    }
    assert!(incomplete > 0);
    assert_eq!(s.plan_route(&q, &r, DEFAULT_BUDGET, &|| false).unwrap().outcome, Outcome::Reached);
    assert!(s.plan_route(&q, &r, 0, &|| false).is_err());
    assert!(s.plan_route(&q, &r, MAX_BUDGET + 1, &|| false).is_err());
}

/// A query captured before an edit, Undo or reopen gives a stale outcome without probing, and
/// a raised cancellation flag stops before the first probe.
#[test]
fn stale_queries_and_cancellation_plan_nothing() {
    let mut s = board();
    let r = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 10_000_000));
    let q = query(&s);
    let canceled = s.plan_route(&q, &r, DEFAULT_BUDGET, &|| true).unwrap();
    assert_eq!((canceled.outcome, canceled.probes, canceled.points.clone()), (Outcome::Canceled, 0, vec![r.head]));
    add(&mut s, ADD_VIA, json!({"at":p(4_000_000, 10_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    let stale = s.plan_route(&q, &r, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!((stale.outcome, stale.probes, stale.points.len()), (Outcome::Stale, 0, 0));
    let current = query(&s);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.plan_route(&current, &r, DEFAULT_BUDGET, &|| false).unwrap().outcome, Outcome::Stale);
}

/// A still-legal previous result is kept as is; when the target moves, the route stays on the
/// previous side unless the other side is markedly shorter.
#[test]
fn previous_results_keep_the_route_stable() {
    let mut s = board();
    add(&mut s, ADD_VIA, json!({"at":p(4_000_000, 10_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    let above = |plan: &Plan| plan.points.iter().any(|p| p.y_nm > 10_500_000);
    let below = |plan: &Plan| plan.points.iter().any(|p| p.y_nm < 9_500_000);
    let low = plan(&s, &request(&s, p(1_000_000, 10_000_000), p(7_000_000, 9_900_000)));
    let high = plan(&s, &request(&s, p(1_000_000, 10_000_000), p(7_000_000, 10_100_000)));
    assert_eq!((low.outcome, high.outcome), (Outcome::Reached, Outcome::Reached));
    assert!(below(&low) && !above(&low), "{low:?}");
    assert!(above(&high) && !below(&high), "{high:?}");
    let mut moved = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 10_100_000));
    moved.previous = low.points.clone();
    let kept = plan(&s, &moved);
    assert_eq!(kept.outcome, Outcome::Reached);
    assert!(below(&kept) && !above(&kept), "{kept:?}");
    let mut same = request(&s, p(1_000_000, 10_000_000), p(7_000_000, 9_900_000));
    same.previous = low.points.clone();
    let reused = plan(&s, &same);
    assert_eq!(reused.points, low.points);
    assert_eq!(reused.probes, low.points.len(), "an unchanged plan only re-confirms its head and legs");
}

/// The command plans with the board's preferred width, edits nothing, refuses a foreign layer
/// and reports a stale outcome for an old revision; its points commit through `pcb.add_tracks`.
#[test]
fn the_command_plans_without_editing_and_feeds_add_tracks() {
    let mut s = board();
    let (head, target) = (pad(&s, "footprint-6", "2"), pad(&s, "footprint-8", "2"));
    let layer = copper_layers(&s)[0];
    let (bytes, revision) = (encode_documents(s.snapshot().unwrap()).unwrap(), s.revision());
    let params = json!({"from":head,"to":target,"layer":layer});
    let response = s.execute(PLAN_ROUTE, &params).unwrap();
    assert_eq!((&encode_documents(s.snapshot().unwrap()).unwrap(), s.revision()), (&bytes, revision));
    let planned = &response["route_plan"];
    assert_eq!((planned["outcome"]["kind"].as_str(), planned["width_nm"].as_i64()), (Some("reached"), Some(WIDTH)));
    assert_eq!(s.apply(PLAN_ROUTE, &params).unwrap(), revision);
    assert!(s.execute(PLAN_ROUTE, &json!({"from":head,"to":target,"layer":uuid::Uuid::from_u128(7)})).is_err());
    assert!(s.execute(PLAN_ROUTE, &json!({"from":head,"to":target,"layer":layer,"posture":"free"})).is_err());
    for expected in [revision - 1, revision + 1] {
        let stale = json!({"from":head,"to":target,"layer":layer,"expected_revision":expected});
        let response = s.execute(PLAN_ROUTE, &stale).unwrap();
        assert_eq!(response["route_plan"]["outcome"]["kind"], "stale");
        assert_eq!(s.apply(PLAN_ROUTE, &stale).unwrap(), revision);
    }
    let error = s.execute(PLAN_ROUTE, &json!({"from":head,"to":target,"layer":layer,"expected_revision":"old"})).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::InvalidParams);
    assert!(error.to_string().contains("`expected_revision`"), "{error}");
    assert_eq!((&encode_documents(s.snapshot().unwrap()).unwrap(), s.revision()), (&bytes, revision));
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":planned["width_nm"],"points":planned["points"]}));
    assert_eq!(s.revision(), revision + 1, "one undo step");
}

/// Board minimums apply to both the command's explicit operand and direct captured requests;
/// omitted widths still select the authored preference and neither refusal edits the project.
#[test]
fn board_minimum_width_is_enforced_for_command_and_pure_planner() {
    let mut s = board();
    let board = view(&s).target.board;
    add(
        &mut s,
        crate::board::rules::SET_RULES,
        json!({"board":board,"rules":crate::board::rules::Values { min_track_width_nm:500000, preferred_track_width_nm:600000,..Default::default() }}),
    );
    let mut r = request(&s, p(1_000_000, 15_000_000), p(7_000_000, 15_000_000));
    r.width_nm = 100_000;
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let params = json!({"from":r.head,"to":r.target,"layer":r.layer,"width_nm":r.width_nm});
    assert!(matches!(s.execute(PLAN_ROUTE, &params), Err(CommandError::InvalidParams(_))));
    assert!(matches!(walkaround(&query(&s), &r, DEFAULT_BUDGET, &|| false), Err(CommandError::InvalidParams(_))));
    let preferred = s.execute(PLAN_ROUTE, &json!({"from":r.head,"to":r.target,"layer":r.layer})).unwrap();
    assert_eq!(preferred["route_plan"]["width_nm"], 600000);
    assert_eq!(preferred["route_plan"]["outcome"]["kind"], "reached");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
}

/// Empty and head-only outputs round-trip as continuity hints, while malformed coordinates,
/// excess vertices and singleton add_tracks paths retain their original validation failures.
#[test]
fn previous_accepts_empty_and_singleton_results_without_weakening_track_paths() {
    let mut s = board();
    let r = request(&s, p(1_000_000, 15_000_000), p(7_000_000, 15_000_000));
    let base = json!({"from":r.head,"to":r.target,"layer":r.layer});
    let incomplete = s.execute(PLAN_ROUTE, &json!({"from":r.head,"to":r.target,"layer":r.layer,"budget":1})).unwrap();
    assert_eq!(incomplete["route_plan"]["outcome"]["kind"], "incomplete");
    for previous in [json!([]), incomplete["route_plan"]["points"].clone()] {
        let mut params = base.clone();
        params["previous"] = previous;
        assert_eq!(s.execute(PLAN_ROUTE, &params).unwrap()["route_plan"]["outcome"]["kind"], "reached");
    }
    for previous in [json!([{"x_nm":i64::MAX,"y_nm":0}]), json!(vec![r.head; 257]), json!([{"x_nm":0}])] {
        let mut params = base.clone();
        params["previous"] = previous;
        assert!(s.execute(PLAN_ROUTE, &params).is_err());
    }
    assert!(s.apply(ADD_TRACKS, &json!({"layer":r.layer,"width_nm":WIDTH,"points":[r.head],"expected_revision":s.revision()})).is_err());
}

/// Two route legs individually touch different accepted nets away from their common elbow.
/// Previous-plan reuse and a fresh search must validate their connected membership together.
#[test]
fn free_space_routes_cannot_bridge_nets_on_different_legs() {
    let mut s = board();
    let layer = copper_layers(&s)[0];
    let a = pad(&s, "footprint-6", "2");
    let b = pad(&s, "footprint-8", "1");
    // Extend accepted copper down to well-separated points in free board space.
    let contact_a = p(a.x_nm, 12_000_000);
    let contact_b = p(0, 18_000_000);
    add(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":WIDTH,"points":[a,contact_a]}));
    add(
        &mut s,
        ADD_TRACKS,
        json!({"layer":layer,"width_nm":WIDTH,"points":[b,p(b.x_nm,6_000_000),p(-2_000_000,6_000_000),p(-2_000_000,18_000_000),contact_b]}),
    );
    let head = p(contact_a.x_nm + 3_000_000, contact_a.y_nm);
    let elbow = p(contact_b.x_nm, contact_a.y_nm);
    let target = p(contact_b.x_nm, contact_b.y_nm + 2_000_000);
    let previous = vec![head, elbow, target];
    let q = query(&s);
    for leg in previous.windows(2) {
        let result = q.probe(&Contour::segment(leg[0], leg[1], WIDTH).unwrap(), &[layer], None).unwrap();
        assert!(!result.decisions.iter().any(|d| d.conflicting_nets), "each independent leg misses the short");
    }
    for continuity in [Vec::new(), previous.clone()] {
        let mut r = request(&s, head, target);
        r.posture = Posture::Orthogonal;
        r.previous = continuity;
        let planned = plan(&s, &r);
        assert_ne!(planned.points, previous, "must not reuse the shorting path");
        if planned.points.len() >= 2 {
            let mut copy = Session::default();
            copy.open_snapshot(s.snapshot().unwrap());
            commit_and_agree(&mut copy, &planned, layer, WIDTH);
        }
    }
}

/// A sequence of separate obstacles exhausts the walk bound even with ample probe budget.
/// Exhaustion must discard partial geometry rather than certify it as a blocked prefix.
#[test]
fn obstacle_search_exhaustion_returns_head_only_incomplete() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    add(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    // The diameters grow from either end so both bounded search directions must
    // encounter more hulls than one walk permits.
    for i in 1..=60 {
        add(&mut s, ADD_VIA, json!({"at":p(i*10_000_000, 0),"via_diameter_nm":600000+i.min(61-i)*200000,"via_drill_nm":300000}));
    }
    let r = request(&s, p(0, 0), p(615_000_000, 0));
    let planned = s.plan_route(&query(&s), &r, MAX_BUDGET, &|| false).unwrap();
    assert_eq!(planned.outcome, Outcome::Incomplete, "{planned:?}");
    assert_eq!(planned.points, vec![r.head]);
    assert_eq!(planned.blocker, None);
    assert!(planned.probes < MAX_BUDGET, "search limit rather than probe budget");
}

/// A contact on a later leg gives the whole route its class requirements, including earlier
/// legs near unrelated copper. Discarded previous-plan contacts cannot remain in search state.
#[test]
fn later_class_contact_revalidates_earlier_clearance_and_width() {
    for (minimum, clearance) in [(100_000, 1_000_000), (400_000, 200_000)] {
        let mut s = board();
        let layer = copper_layers(&s)[0];
        let b = pad(&s, "footprint-8", "1");
        let target = view(&s).net_class_target_for_copper("footprint-8:1").unwrap().unwrap();
        add(
            &mut s,
            crate::net_classes::CREATE,
            json!({"name":"Late contact","rules":{
            "clearance_nm":clearance,"min_track_width_nm":minimum,"preferred_track_width_nm":500000}}),
        );
        let class = s.project().unwrap().net_classes().definitions.iter().find(|c| c.name == "Late contact").unwrap().id;
        add(&mut s, crate::nets::SET_CLASSES, json!({"target":target,"classes":[class]}));
        // Approach the later route vertically from outside its elbow, then return to it.
        let contact = p(0, 18_000_000);
        add(
            &mut s,
            ADD_TRACKS,
            json!({"layer":layer,"width_nm":500000,"points":[b,p(b.x_nm,6_000_000),p(-2_000_000,6_000_000),p(-2_000_000,18_000_000),contact]}),
        );
        add(&mut s, ADD_VIA, json!({"at":p(4_000_000,12_700_000),"via_diameter_nm":600000,"via_drill_nm":300000}));
        let mut r = request(&s, p(20_000_000, 12_000_000), p(0, 20_000_000));
        r.posture = Posture::Orthogonal;
        r.previous = vec![r.head, p(0, r.head.y_nm), r.target];
        let old = r.previous.clone();
        let planned = plan(&s, &r);
        assert_ne!(planned.points, old, "{minimum} / {clearance}: {planned:?}");
        if planned.points.len() >= 2 {
            commit_and_agree(&mut s, &planned, layer, WIDTH);
        }
    }
}

/// Direct Rust clients get the same bounded previous hints as command clients, before side
/// arithmetic, cancellation or a probe. Empty and bounded singleton hints remain valid.
#[test]
fn direct_planner_validates_previous_hints_before_using_them() {
    let s = board();
    let q = query(&s);
    let mut r = request(&s, p(-1, 0), p(-1, 1));
    let cancellations = std::cell::Cell::new(0);
    let cancel = || {
        cancellations.set(cancellations.get() + 1);
        false
    };
    for previous in [vec![p(i64::MAX, 0)], vec![p(i64::MIN, 0)], vec![r.head; 257]] {
        r.previous = previous;
        assert!(matches!(walkaround(&q, &r, DEFAULT_BUDGET, &cancel), Err(CommandError::InvalidParams(_))));
        assert_eq!(cancellations.get(), 0);
    }
    for previous in [Vec::new(), vec![r.head]] {
        r.previous = previous;
        assert_eq!(walkaround(&q, &r, DEFAULT_BUDGET, &|| false).unwrap().outcome, Outcome::Reached);
    }
}

/// A new unknown-net obstacle invalidates a previous route rather than becoming an implicitly
/// authorized island. Head-contact islands retain their existing departure exemption.
#[test]
fn previous_route_goes_around_new_unconnected_copper() {
    for via in [false, true] {
        let mut s = board();
        let mut r = request(&s, p(1_000_000, 15_000_000), p(7_000_000, 15_000_000));
        let first = plan(&s, &r);
        assert_eq!(first.outcome, Outcome::Reached);
        assert_eq!(first.points, vec![r.head, r.target]);
        if via {
            add(&mut s, ADD_VIA, json!({"at":p(4_000_000,15_000_000),"via_diameter_nm":600000,"via_drill_nm":300000}));
        } else {
            add(&mut s, ADD_TRACKS, json!({"layer":r.layer,"width_nm":WIDTH,"points":[p(4_000_000,14_000_000),p(4_000_000,16_000_000)]}));
        }
        let obstacle = query(&s)
            .objects()
            .iter()
            .find(|o| o.contour.gap(&Contour::capsule(p(4_000_000, 15_000_000), p(4_000_000, 15_000_000), 0.)) <= 0.)
            .unwrap()
            .name
            .clone();
        r.previous = first.points;
        let replanned = plan(&s, &r);
        assert_eq!(replanned.outcome, Outcome::Reached, "{replanned:?}");
        assert_ne!(replanned.points, r.previous, "must not authorize new copper by crossing it");
        assert!(gap_to(&s, &replanned, &obstacle) >= 200000.);
        commit_and_agree(&mut s, &replanned, r.layer, r.width_nm);
    }
}

/// A command-created 0805 pair and connected rails matching the pad-gap walkaround report.
fn pad_gap_board() -> Session {
    pad_gap_board_with_pickup(false)
}

/// Include an optional isolated starting pad before accepting the board/package baseline.
fn pad_gap_board_with_pickup(extra_pickup: bool) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let mut symbols = Vec::new();
    for x in [0, 20_000_000] {
        add(&mut s, crate::PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x,"y_nm":0}));
        let symbol = s.project().unwrap().symbols().last().unwrap().id();
        add(&mut s, crate::PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"resistor-0805","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0}));
        symbols.push(symbol);
    }
    add(&mut s, crate::CONNECT_PINS, json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"}}));
    if extra_pickup {
        add(&mut s, crate::PLACE_SYMBOL, json!({"definition":"resistor","x_nm":30_000_000,"y_nm":0}));
        let symbol = s.project().unwrap().symbols().last().unwrap().id();
        add(
            &mut s,
            crate::PLACE_FOOTPRINT,
            json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":2_811_000,"y_nm":6_350_000}),
        );
    }
    add(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true}));
    add(&mut s, crate::board::update::INITIALIZE_BASELINE, json!({}));
    let layer = copper_layers(&s)[0];
    for (points, width) in [
        (vec![p(-975_000, 0), p(-975_000, 5_000_000), p(19_025_000, 5_000_000), p(19_025_000, 0)], 500_000),
        (vec![p(0, 5_800_000), p(20_000_000, 5_800_000)], 250_000),
        (vec![p(17_000_000, 5_000_000), p(17_000_000, 8_000_000)], 250_000),
    ] {
        add(&mut s, ADD_TRACKS, json!({"layer":layer,"points":points,"width_nm":width}));
    }
    s
}

/// A legal cursor in the gap must be reached on both axes, without adopting either pad's net.
#[test]
fn walkaround_reaches_legal_cursor_positions_in_a_concave_pad_gap() {
    let s = pad_gap_board();
    let head = p(2_286_000, 6_350_000);
    for x in [-127_000, 0, 127_000] {
        for y in [-508_000, -254_000, 0, 254_000, 508_000] {
            let target = p(x, y);
            let req = request(&s, head, target);
            let mut witness = Session::default();
            witness.open_snapshot(s.snapshot().unwrap());
            witness.apply(ADD_TRACKS,&json!({"board":view(&witness).target.board,"layer":req.layer,"width_nm":WIDTH,"points":[head,p(-1_800_000,head.y_nm),p(-1_800_000,-1_200_000),p(x,-1_200_000),target],"replacements":[],"expected_revision":witness.revision()})).unwrap();
            let result = plan(&s, &req);
            assert_eq!(result.outcome, Outcome::Reached, "legal target {target:?}: {result:?}");
            assert_eq!(result.points.last(), Some(&target));
            let mut candidate = Session::default();
            candidate.open_snapshot(s.snapshot().unwrap());
            commit_and_agree(&mut candidate, &result, req.layer, WIDTH);
        }
    }
}

/// A target-side retry keeps the original unconnected pad as its contact authority;
/// it cannot adopt the nearby named copper or lose the starting pad's own-island exemption.
#[test]
fn walkaround_retry_keeps_the_original_unconnected_pickup() {
    let mut s = pad_gap_board_with_pickup(true);
    let head = p(2_286_000, 6_350_000);
    let captured = query(&s);
    assert!(captured.objects().iter().any(|o| o.at == head && o.intended_nets.is_empty()));
    let req = request(&s, head, p(0, -254_000));
    let result = plan(&s, &req);
    assert_eq!(result.outcome, Outcome::Reached, "{result:?}");
    assert_eq!(result.points.first(), Some(&head));
    assert_eq!(result.points.last(), Some(&req.target));
    commit_and_agree(&mut s, &result, req.layer, WIDTH);
    for x in [-254_000, 254_000] {
        let mut candidate = Session::default();
        candidate.open_snapshot(pad_gap_board().snapshot().unwrap());
        let blocked = plan(&candidate, &request(&candidate, p(2_286_000, 6_350_000), p(x, -254_000)));
        assert!(!matches!(blocked.outcome, Outcome::Reached), "a blocked pad-clearance target was reached");
    }
}

/// A blocked cursor beside a trace should approach its nearby clearance boundary instead
/// of leaving the preview at an obstacle encountered much earlier along the route.
#[test]
fn blocked_cursor_approach_tracks_the_clearance_boundary() {
    let s = pad_gap_board();
    let head = p(6_096_000, 6_858_000);
    for y in [1_016_000, 2_032_000, 3_048_000, 4_064_000] {
        let target = p(-1_524_000, y);
        let req = request(&s, head, target);
        let boundary = p(-1_550_002, y);
        let mut witness = Session::default();
        witness.open_snapshot(s.snapshot().unwrap());
        witness
            .apply(
                ADD_TRACKS,
                &json!({"board":view(&witness).target.board,"layer":req.layer,"width_nm":WIDTH,
            "points":[head,p(-1_600_000,head.y_nm),p(-1_600_000,y),boundary],"replacements":[],"expected_revision":witness.revision()}),
            )
            .unwrap();
        let polls = std::cell::Cell::new(0);
        let planned = s
            .plan_route(&query(&s), &req, DEFAULT_BUDGET, &|| {
                polls.set(polls.get() + 1);
                false
            })
            .unwrap();
        let Outcome::Blocked { last_legal } = planned.outcome else { panic!("{planned:?}") };
        assert_eq!(planned.adjusted_target, Some(last_legal));
        assert_eq!(planned.points.last(), Some(&last_legal));
        assert!(
            (last_legal.x_nm - target.x_nm).abs() <= 50_000 && (last_legal.y_nm - y).abs() <= 2,
            "blocked cursor{target:?} should remain close to clearance, got{planned:?}"
        );
        let mut candidate = Session::default();
        candidate.open_snapshot(s.snapshot().unwrap());
        commit_and_agree(&mut candidate, &planned, req.layer, WIDTH);
        if y == 4_064_000 {
            let before = encode_documents(s.snapshot().unwrap()).unwrap();
            let mut command_session = Session::default();
            command_session.open_snapshot(s.snapshot().unwrap());
            let response = command_session
                .execute(PLAN_ROUTE, &json!({"board":view(&s).target.board,"from":head,"to":target,"layer":req.layer,"width_nm":WIDTH}))
                .unwrap();
            assert_eq!(response["route_plan"]["adjusted_target"], json!(last_legal));
            assert_eq!(response["route_plan"]["outcome"]["kind"], "blocked");
            let context = s.route_planning(view(&s).target).unwrap();
            let mut job = s.prepare_route_plan(&context, req.clone(), DEFAULT_BUDGET).unwrap();
            let mut done = false;
            for _ in 0..100_000 {
                let probes = job.progress().completed;
                if job.step(1) {
                    done = true;
                    break;
                }
                assert!(job.progress().completed <= probes + 1);
            }
            assert!(done);
            assert_eq!(job.take_result().unwrap().unwrap(), planned);
            let count = std::cell::Cell::new(0);
            let canceled = s
                .plan_route(&query(&s), &req, DEFAULT_BUDGET, &|| {
                    count.set(count.get() + 1);
                    count.get() >= polls.get() - 1
                })
                .unwrap();
            assert_eq!(canceled.outcome, Outcome::Canceled);
            assert_eq!(canceled.points, vec![head]);
            assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
        }
    }
}

/// A single round obstacle isolates optional cursor refinement from the primary walk.
#[test]
fn refinement_exhaustion_keeps_a_certified_prefix() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    add(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    add(&mut session, ADD_VIA, json!({"at":p(5_000_000,0),"via_diameter_nm":1_000_000,"via_drill_nm":300_000}));
    let request = request(&session, p(0, 0), p(5_000_000, 200_000));
    let query = query(&session);
    let refined = session.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap();
    let prefix = session.plan_route(&query, &request, 45, &|| false).unwrap();
    let Outcome::Blocked { last_legal } = prefix.outcome else { panic!("{prefix:?}") };
    assert_eq!(prefix.probes, 45);
    assert_eq!(prefix.points, [request.head, last_legal]);
    assert_ne!(last_legal, request.head);
    assert_eq!(prefix.adjusted_target, Some(last_legal));
    assert_ne!(prefix.adjusted_target, refined.adjusted_target);
    let no_prefix = session.plan_route(&query, &request, 1, &|| false).unwrap();
    assert_eq!(no_prefix.outcome, Outcome::Incomplete);
    assert_eq!(no_prefix.points, [request.head]);
    assert!(no_prefix.adjusted_target.is_none());
    commit_and_agree(&mut session, &prefix, request.layer, WIDTH);
}

/// Both pointer-only movement and continuation from a fixed top bend reach legal pad-gap
/// targets and certify the visible clearance endpoint at an obstructed inside corner.
#[test]
fn pad_bend_continuations_keep_a_placeable_clearance_endpoint() {
    let s = pad_gap_board();
    let origin = p(-1_016_000, 6_096_000);
    let above = plan(&s, &request(&s, origin, p(-660_000, -900_000)));
    assert_eq!(above.adjusted_target, Some(p(-660_000, -1_050_002)));
    for fixed in [false, true] {
        let head = if fixed { *above.points.last().unwrap() } else { origin };
        let mut previous = if fixed { Vec::new() } else { above.points.clone() };
        for (target, expected) in [
            (p(-254_000, 1_016_000), None),
            (p(-508_000, 5_334_000), Some(p(-399_998, 4_424_998))),
            (p(-254_000, 4_572_000), Some(p(-254_000, 4_424_998))),
            (p(-300_000, 4_300_000), None),
        ] {
            let mut req = request(&s, head, target);
            req.origin = Some((origin, req.layer));
            req.previous = previous;
            let mut result = plan(&s, &req);
            assert_eq!(result.adjusted_target, expected, "fixed{fixed} cursor{target:?}: {result:?}");
            assert_eq!(result.outcome, expected.map_or(Outcome::Reached, |last_legal| Outcome::Blocked { last_legal }));
            assert_eq!(result.points.last(), Some(&expected.unwrap_or(target)));
            previous = result.points.clone();
            if fixed {
                let mut complete = above.points.clone();
                complete.extend(result.points.into_iter().skip(1));
                result.points = complete;
            }
            let mut candidate = Session::default();
            candidate.open_snapshot(s.snapshot().unwrap());
            let original = encode_documents(candidate.snapshot().unwrap()).unwrap();
            candidate
                .apply(
                    ADD_TRACKS,
                    &json!({"board":view(&s).target.board,"layer":req.layer,"width_nm":WIDTH,
                "points":result.points,"replacements":[],"expected_revision":candidate.revision()}),
                )
                .unwrap();
            candidate.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(original, encode_documents(candidate.snapshot().unwrap()).unwrap());
            commit_and_agree(&mut candidate, &result, req.layer, WIDTH);
        }
    }
}

/// Sub-grid cursor movement beside a pad/trace junction must not collapse a route.
/// Independent strict-command witnesses establish that each tested legal target is reachable,
/// including a target exactly at the trace's clearance boundary.
#[test]
fn pad_pocket_subgrid_targets_agree_with_legal_command_witnesses() {
    let s = pad_gap_board();
    let q = query(&s);
    for head in [p(-1_016_000, 6_096_000), p(-1_070_000, 6_125_000), p(-1_143_000, 6_223_000)] {
        for x in [-400_000, -300_000, -254_000, -200_000, 0] {
            for y in [762_000, 1_016_000, 1_100_000, 1_270_000] {
                let target = p(x, y);
                let req = request(&s, head, target);
                let probe = q.probe_membership(&Contour::capsule(target, target, WIDTH as f64 / 2.), &[req.layer], &[], None).unwrap();
                let legal = probe.decisions.iter().all(|d| !d.conflicting_nets && d.gap_nm >= d.clearance_nm as f64);
                let result = plan(&s, &req);
                assert!(result.probes <= DEFAULT_BUDGET);
                if !legal {
                    assert_ne!(result.outcome, Outcome::Reached, "{result:?}");
                    continue;
                }
                let mut witness = Session::default();
                witness.open_snapshot(s.snapshot().unwrap());
                let mut points = vec![head, p(-2_000_000, head.y_nm), p(-2_000_000, -2_000_000), p(0, -2_000_000), p(0, y), target];
                points.dedup();
                witness
                    .apply(
                        ADD_TRACKS,
                        &json!({"board":view(&s).target.board,"layer":req.layer,"width_nm":WIDTH,
                    "points":points,"replacements":[],"expected_revision":witness.revision()}),
                    )
                    .unwrap();
                assert_eq!(result.outcome, Outcome::Reached, "head{head:?} target{target:?}: {result:?}");
                assert_eq!(result.points.last(), Some(&target));
                let mut candidate = Session::default();
                candidate.open_snapshot(s.snapshot().unwrap());
                candidate
                    .apply(
                        ADD_TRACKS,
                        &json!({"board":view(&s).target.board,"layer":req.layer,"width_nm":WIDTH,
                    "points":result.points,"replacements":[],"expected_revision":candidate.revision()}),
                    )
                    .unwrap();
                candidate.apply("edit.undo", &json!({})).unwrap();
                commit_and_agree(&mut candidate, &result, req.layer, WIDTH);
            }
        }
    }
}

/// Fixed vias can precede a segment in the MCP payload without moving the
/// original pickup from front copper to the new route layer.
#[test]
fn shove_pickup_uses_first_fixed_segment_after_a_via() {
    let mut s = board();
    let [front, back, ..] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let start = p(1_000_000, 0);
    let head = p(5_000_000, 0);
    let target = p(8_000_000, 0);
    add(&mut s, ADD_TRACKS, json!({"layer":front,"width_nm":WIDTH,"points":[p(0,0),start]}));
    let track = view(&s).tracks()[0].id;
    let board = view(&s).target.board;
    add(&mut s, SET_COPPER_LOCKED, json!({"board":board,"tracks":[track],"locked":true}));
    let segment = FixedCopper::Segment { start, end: head, layer: front, width_nm: WIDTH };
    let via = FixedCopper::Via { at: head, diameter_nm: 600_000 };
    let plan = |fixed| s.route_plan_query(&json!({"mode":"shove","from":head,"to":target,"layer":back,"width_nm":WIDTH,"fixed":fixed})).unwrap();
    let result = plan([via.clone(), segment.clone()]);
    assert_eq!(result, plan([segment, via]));
    assert_eq!(result["route_plan"]["outcome"]["kind"], "reached");
}
