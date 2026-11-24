//! Shove fixtures are authored with shared commands and checked by the final compound validator.
use super::*;
use crate::{
    ADD_TRACKS, INITIALIZE_BOARD_SETUP, SET_COPPER_LOCKED,
    board::track::tests::{copper_layers, view},
    persistence::{decode_documents, encode_documents},
};

fn p(x: i64, y: i64) -> Point {
    Point { x_nm: x * 100_000, y_nm: y * 100_000 }
}
fn board() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s
}
fn add(s: &mut Session, points: Vec<Point>, width: i64) -> Vec<Uuid> {
    let old: BTreeSet<_> = view(s).tracks().iter().map(|t| t.id).collect();
    s.apply(ADD_TRACKS, &json!({"layer":copper_layers(s)[0],"width_nm":width,"points":points,"expected_revision":s.revision()})).unwrap();
    view(s).tracks().iter().filter(|t| !old.contains(&t.id)).map(|t| t.id).collect()
}
fn request(s: &Session) -> Request {
    Request {
        head: p(20, 20),
        target: p(80, 1),
        origin: None,
        layer: copper_layers(s)[0],
        width_nm: 250_000,
        posture: Posture::Diagonal,
        flipped: false,
        previous: Vec::new(),
        mode: Mode::Shove,
        merge_same_net: false,
        fixed: Vec::new(),
    }
}
fn query(s: &Session) -> ClearanceQuery {
    s.clearance_query_with_rules(view(s).target, &json!({})).unwrap()
}

/// Plan publication revalidates independently, preserves the complete contact graph, and gives
/// exactly one undo entry for incoming and displaced copper together.
fn commit(s: &mut Session, request: &Request, plan: &Plan) {
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let rev = s.revision();
    let mut points = Vec::new();
    for piece in &request.fixed {
        if let FixedCopper::Segment { start, end, .. } = piece {
            if points.is_empty() {
                points.push(*start);
            }
            points.push(*end);
        }
    }
    if points.is_empty() {
        points = plan.points.clone();
    } else {
        points.extend(plan.points.iter().copied().skip(1));
    }
    s.apply(
        ADD_TRACKS,
        &json!({"board":view(s).target.board,"layer":request.layer,"width_nm":request.width_nm,
        "points":points,"via_moves":plan.via_moves,"replacements":plan.replacements,"merge_same_net":request.merge_same_net,"expected_revision":rev}),
    )
    .unwrap();
    assert_eq!(s.revision(), rev + 1);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// A parallel incoming route pushes one neighbor; the solver retains its endpoints and width.
#[test]
fn single_neighbor_is_displaced_without_live_mutation() {
    let mut s = board();
    let id = add(&mut s, vec![p(0, 0), p(100, 0)], 500_000)[0];
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let request = request(&s);
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    assert_eq!(plan.replacements.len(), 1, "{plan:?}");
    assert_eq!(plan.replacements[0].track, id);
    assert_eq!(plan.replacements[0].points.first(), Some(&p(0, 0)));
    assert_eq!(plan.replacements[0].points.last(), Some(&p(100, 0)));
    commit(&mut s, &request, &plan);
}

/// Displacement propagates to a second different-width neighbor with deterministic source order.
#[test]
fn cascades_and_branches_keep_their_connections() {
    for branch in [false, true] {
        let mut s = board();
        add(&mut s, vec![p(0, 0), p(100, 0)], 250_000);
        if branch {
            add(&mut s, vec![p(95, 0), p(95, -40)], 500_000);
        } else {
            add(&mut s, vec![p(0, -7), p(100, -7)], 500_000);
        }
        let request = request(&s);
        let query = query(&s);
        let plan = s.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan, s.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap());
        assert!(!plan.replacements.is_empty(), "{branch}: {plan:?}");
        if !branch {
            assert_eq!(plan.replacements.len(), 2, "{plan:?}");
        }
        commit(&mut s, &request, &plan);
    }
}

/// Locked direct and indirect neighbors remain byte-identical; the initial path may detour around them.
#[test]
fn direct_and_indirect_locks_stay_fixed() {
    for indirect in [false, true] {
        let mut s = board();
        let first = add(&mut s, vec![p(0, 0), p(100, 0)], 250_000)[0];
        let locked = if indirect { add(&mut s, vec![p(0, -7), p(100, -7)], 500_000)[0] } else { first };
        s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[locked],"locked":true,"expected_revision":s.revision()})).unwrap();
        let original = view(&s).tracks().iter().find(|t| t.id == locked).unwrap().clone();
        let request = request(&s);
        let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert!(plan.replacements.iter().all(|r| r.track != locked));
        if plan.outcome == Outcome::Reached {
            commit(&mut s, &request, &plan);
        } else {
            assert!(plan.replacements.is_empty());
        }
        assert_eq!(view(&s).tracks().iter().find(|t| t.id == locked), Some(&original));
    }
}

/// Cancellation, budget exhaustion, stale sources and dropping a suspended job leave bytes
/// and revision unchanged. Native/browser batches return the exact synchronous plan.
#[test]
fn bounded_jobs_cancel_and_match_the_synchronous_plan() {
    let mut s = board();
    add(&mut s, vec![p(0, 0), p(100, 0)], 250_000);
    let request = request(&s);
    let query = query(&s);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(s.plan_route(&query, &request, DEFAULT_BUDGET, &|| true).unwrap().outcome, Outcome::Canceled);
    let exhausted = s.plan_route(&query, &request, 1, &|| false).unwrap();
    assert_eq!(exhausted.outcome, Outcome::Incomplete);
    assert!(exhausted.replacements.is_empty());
    let expected = s.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap();
    let context = s.route_planning(view(&s).target).unwrap();
    let revision = s.revision();
    let mut suspended = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
    for _ in 0..256 {
        assert!(!suspended.step(1), "must still be pending after the first probe");
        if suspended.progress().completed > 0 {
            break;
        }
    }
    assert!(suspended.progress().completed > 0);
    drop(suspended);
    assert_eq!(s.revision(), revision);
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    let mut job = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
    assert!(!job.step(0));
    for i in 0..100_000 {
        let probes = job.progress().completed;
        if job.step(1) {
            break;
        }
        assert!(job.progress().completed >= probes);
        assert!(job.progress().completed <= probes + 1);
        assert!(i < 99_999);
    }
    assert_eq!(job.take_result().unwrap().unwrap(), expected);
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    s.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    assert_eq!(s.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap().outcome, Outcome::Stale);
}

/// Parallel mixed-width rails sharing an interior branch, with either stable-ID ordering.
fn parallel_rails(reverse: bool) -> (Session, Uuid, Uuid) {
    let mut s = board();
    let ids = add(&mut s, vec![p(0, 0), p(1, 0), p(2, 0)], 250_000);
    let (a, b) = if reverse { (ids[1], ids[0]) } else { (ids[0], ids[1]) };
    s.apply(
        crate::RESHAPE_TRACKS,
        &json!({"tracks":[
        {"track":a,"start":Point{x_nm:-975_000,y_nm:5_000_000},"end":Point{x_nm:19_025_000,y_nm:5_000_000}},
        {"track":b,"start":Point{x_nm:0,y_nm:5_800_000},"end":Point{x_nm:20_000_000,y_nm:5_800_000}}],"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::SET_TRACK_WIDTH, &json!({"tracks":[a],"width_nm":500_000,"expected_revision":s.revision()})).unwrap();
    add(&mut s, vec![Point { x_nm: 17_000_000, y_nm: 5_000_000 }, Point { x_nm: 17_000_000, y_nm: 8_000_000 }], 250_000);
    assert_eq!(view(&s).copper().expect("arc-free fixture copper").tracks[&a], view(&s).copper().expect("arc-free fixture copper").tracks[&b]);
    (s, a, b)
}

/// Parallel tracks joined through a branch still need spacing when displaced. Their shared
/// island must not authorize collapse of originally separated copper, in either work-queue order.
#[test]
fn connected_parallel_tracks_cannot_be_shoved_under_each_other() {
    use crate::board::geometry::Contour;
    for reverse in [false, true] {
        let (mut s, a, b) = parallel_rails(reverse);
        let original = view(&s).tracks().to_vec();
        let prefix = [
            Point { x_nm: 6_858_000, y_nm: 7_366_000 },
            Point { x_nm: 10_566_657, y_nm: 7_366_000 },
            Point { x_nm: 11_445_634, y_nm: 6_487_023 },
            Point { x_nm: 13_324_977, y_nm: 6_487_023 },
            Point { x_nm: 14_224_000, y_nm: 5_588_000 },
            Point { x_nm: 14_224_000, y_nm: 4_572_000 },
        ];
        let request = Request {
            head: prefix[5],
            origin: Some((prefix[0], copper_layers(&s)[0])),
            fixed: prefix
                .windows(2)
                .map(|p| FixedCopper::Segment { start: p[0], end: p[1], layer: copper_layers(&s)[0], width_nm: 250_000 })
                .collect(),
            target: Point { x_nm: 11_430_000, y_nm: 1_778_000 },
            ..request(&s)
        };
        let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
        if plan.outcome != Outcome::Reached {
            assert!(plan.replacements.is_empty());
            continue;
        }
        let pieces = |id| {
            let track = original.iter().find(|t| t.id == id).unwrap();
            let points = plan.replacements.iter().find(|r| r.track == id).map_or_else(|| vec![track.start, track.end], |r| r.points.clone());
            points.windows(2).map(|p| Contour::segment(p[0], p[1], track.width_nm).unwrap()).collect::<Vec<_>>()
        };
        let (left, right) = (pieces(a), pieces(b));
        let gap = left.iter().flat_map(|a| right.iter().map(move |b| a.gap(b))).fold(f64::INFINITY, f64::min);
        assert!(gap >= 200_000., "shared-island displacement collapsed spacing to {gap} nm: {plan:?}");
        commit(&mut s, &request, &plan);
    }
    // The compact approach in the same branched fixture has space for both
    // returns. Use its separately accepted witness so this test cannot pass
    // by refusing every candidate.
    let (mut s, a, b) = parallel_rails(false);
    let head = Point { x_nm: 2_159_000, y_nm: 6_604_000 };
    let target = Point { x_nm: 13_970_000, y_nm: -1_016_000 };
    validate_clean_branch_return(&s, a, b, vec![head, Point { x_nm: 6_350_000, y_nm: 6_604_000 }, target]);
    let request = Request { head, target, ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, request.mode.default_probe_budget(), &|| false).unwrap();
    assert_eq!(plan.replacements.len(), 2, "{plan:?}");
    let shapes = |id, width| {
        plan.replacements
            .iter()
            .find(|r| r.track == id)
            .unwrap()
            .points
            .windows(2)
            .map(|p| Contour::segment(p[0], p[1], width).unwrap())
            .collect::<Vec<_>>()
    };
    for left in shapes(a, 500_000) {
        for right in shapes(b, 250_000) {
            assert!(left.gap(&right) >= 200_000., "{plan:?}");
        }
    }
    commit(&mut s, &request, &plan);
}

/// A push from either side keeps the displaced rails ahead, including after fixing a vertex.
/// Interior anchors on the other rail must be reserved before the first displacement.
#[test]
fn parallel_rails_push_forward_from_free_space_and_an_advanced_prefix() {
    for reverse in [false, true] {
        for case in 0..4 {
            let (mut s, a, b) = parallel_rails(reverse);
            add(&mut s, vec![Point { x_nm: -975_000, y_nm: 0 }, Point { x_nm: -975_000, y_nm: 5_000_000 }], 500_000);
            let start = if case == 0 { Point { x_nm: 1_000_000, y_nm: 6_700_000 } } else { Point { x_nm: 975_000, y_nm: 0 } };
            let target = if case == 0 {
                Point { x_nm: 6_000_000, y_nm: 5_200_000 }
            } else if case == 3 {
                Point { x_nm: 8_000_000, y_nm: 6_500_000 }
            } else {
                Point { x_nm: 4_000_000, y_nm: 6_500_000 }
            };
            let mut prefix = vec![start];
            if case >= 2 {
                prefix.push(Point { x_nm: 975_000, y_nm: 3_475_000 });
            }
            if case == 3 {
                prefix.push(Point { x_nm: 4_000_000, y_nm: 6_500_000 });
            }
            let request = Request {
                head: *prefix.last().unwrap(),
                target,
                origin: Some((start, copper_layers(&s)[0])),
                fixed: prefix
                    .windows(2)
                    .map(|p| FixedCopper::Segment { start: p[0], end: p[1], layer: copper_layers(&s)[0], width_nm: 250_000 })
                    .collect(),
                ..request(&s)
            };
            let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "case {case}, reverse {reverse}: {plan:?}");
            for (id, y) in [(a, 5_000_000), (b, 5_800_000)] {
                let replacement = plan.replacements.iter().find(|r| r.track == id).expect("both rails must move");
                assert!(
                    replacement.points.iter().all(|p| if case == 0 { p.y_nm <= y } else { p.y_nm >= y }),
                    "track wrapped behind push, case{case}: {plan:?}"
                );
                assert!(replacement.points.iter().any(|p| if case == 0 { p.y_nm < y } else { p.y_nm > y }));
            }
            commit(&mut s, &request, &plan);
        }
    }
}

/// Displaced same-net copper starts the next shove at clearance, before the user's route
/// reaches that next track. The opt-in policy may deliberately exempt the same-net neighbor.
#[test]
fn same_net_clearance_alone_propagates_the_next_shove() {
    for reverse in [false, true] {
        for merging in [false, true] {
            let (mut s, a, b) = parallel_rails(reverse);
            let request = Request {
                head: Point { x_nm: 6_000_000, y_nm: 7_300_000 },
                target: Point { x_nm: 11_000_000, y_nm: 6_000_000 },
                merge_same_net: merging,
                ..request(&s)
            };
            let original = query(&s);
            let upper = original.objects().iter().find(|o| o.name == a.to_string()).unwrap();
            let plan = s.plan_route(&original, &request, DEFAULT_BUDGET, &|| false).unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
            for leg in plan.points.windows(2) {
                let incoming = Contour::segment(leg[0], leg[1], request.width_nm).unwrap();
                assert!(incoming.gap(&upper.contour) >= 200_000., "incoming leg {leg:?} directly hits the upper rail");
            }
            assert!(plan.replacements.iter().any(|r| r.track == b));
            if !merging {
                assert!(plan.replacements.iter().any(|r| r.track == a), "same-net clearance did not propagate: {plan:?}");
            }
            commit(&mut s, &request, &plan);
        }
    }
}

/// The mixed-width branch fixture with fixed side contacts and an upper locked rail/via.
fn locked_channel(reverse: bool) -> (Session, Uuid, Uuid) {
    let (mut s, upper, lower) = parallel_rails(reverse);
    add(&mut s, vec![Point { x_nm: -975_000, y_nm: 0 }, Point { x_nm: -975_000, y_nm: 5_000_000 }], 500_000);
    add(&mut s, vec![Point { x_nm: 19_025_000, y_nm: 5_000_000 }, Point { x_nm: 19_025_000, y_nm: 0 }], 500_000);
    let lock = add(&mut s, vec![Point { x_nm: 4_000_000, y_nm: -4_000_000 }, Point { x_nm: 16_000_000, y_nm: -4_000_000 }], 500_000)[0];
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[lock],"locked":true,"expected_revision":s.revision()})).unwrap();
    s.apply(
        crate::ADD_VIA,
        &json!({"board":view(&s).target.board,"at":Point{x_nm:12_000_000,y_nm:-4_000_000},"via_diameter_nm":1_000_000,"via_drill_nm":500_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    (s, upper, lower)
}

/// Advancing a vertical draft through the rails must not abandon a valid shove near the
/// locked rail above. The remaining channel fits both displaced widths and their clearances.
#[test]
fn advancing_to_the_top_keeps_the_two_rail_shove() {
    for reverse in [false, true] {
        let (s, upper, lower) = locked_channel(reverse);
        // An independently specified legal path demonstrates that the locked channel fits;
        // a search refusal here is not an intentional clearance or anchor limit.
        let mut witness = Session::default();
        witness.open_snapshot(s.snapshot().unwrap());
        witness.apply(ADD_TRACKS,&json!({"board":view(&witness).target.board,"layer":copper_layers(&witness)[0],"width_nm":250_000,
            "points":[Point{x_nm:8_128_000,y_nm:9_398_000},Point{x_nm:8_128_000,y_nm:-2_159_000}],
            "replacements":[
                {"track":upper,"points":[Point{x_nm:-975_000,y_nm:5_000_000},p(48,50),p(48,-32),p(105,-32),p(105,40),p(170,40),p(170,50),Point{x_nm:19_025_000,y_nm:5_000_000}]},
                {"track":lower,"points":[p(0,58),p(54,58),Point{x_nm:5_400_000,y_nm:-2_620_000},Point{x_nm:9_900_000,y_nm:-2_620_000},p(99,58),p(200,58)]}
            ],"expected_revision":witness.revision()})).expect("a legal two-rail shove fits below the locked rail");
        let source = encode_documents(s.snapshot().unwrap()).unwrap();
        for y in [1_016_000, -1_524_000, -2_159_000] {
            let request = Request { head: Point { x_nm: 8_128_000, y_nm: 9_398_000 }, target: Point { x_nm: 8_128_000, y_nm: y }, ..request(&s) };
            let polls = std::cell::Cell::new(0usize);
            let plan = s
                .plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| {
                    polls.set(polls.get() + 1);
                    false
                })
                .unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "y={y}: {plan:?}");
            assert_eq!(plan.replacements.len(), 2, "y={y}: abandoned shove: {plan:?}");
            if y == -2_159_000 {
                let context = s.route_planning(view(&s).target).unwrap();
                let mut job = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
                let mut done = false;
                for _ in 0..100_000 {
                    if job.step(1) {
                        done = true;
                        break;
                    }
                }
                assert!(done, "bounded alternative continuation did not finish");
                assert_eq!(job.take_result().unwrap().unwrap(), plan);
                for cut in [polls.get() / 2, polls.get() - 1] {
                    let count = std::cell::Cell::new(0usize);
                    let canceled = s
                        .plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| {
                            count.set(count.get() + 1);
                            count.get() >= cut
                        })
                        .unwrap();
                    assert_eq!(canceled.outcome, Outcome::Canceled);
                    assert!(canceled.replacements.is_empty());
                    assert_eq!(source, encode_documents(s.snapshot().unwrap()).unwrap());
                }
                let exhausted = s.plan_route(&query(&s), &request, 64, &|| false).unwrap();
                assert!(exhausted.probes <= 64);
                assert_eq!(exhausted.outcome, Outcome::Incomplete);
                assert!(exhausted.replacements.is_empty());
            }
            let mut candidate = Session::default();
            candidate.open_snapshot(decode_documents(&source).unwrap());
            commit(&mut candidate, &request, &plan);
        }
        // Near the locked rail, a successful route must still displace both neighbors.
        // A failed cascade must not change to walking around the original movable copper.
        for x in [8_128_000, 9_017_000] {
            let request = Request { head: Point { x_nm: 8_128_000, y_nm: 9_398_000 }, target: Point { x_nm: x, y_nm: -2_667_000 }, ..request(&s) };
            let blocked = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
            if blocked.outcome == Outcome::Reached {
                assert_eq!(blocked.replacements.len(), 2, "abandoned shove: {blocked:?}");
                let mut candidate = Session::default();
                candidate.open_snapshot(decode_documents(&source).unwrap());
                commit(&mut candidate, &request, &blocked);
            } else {
                assert!(matches!(blocked.outcome, Outcome::Blocked { .. } | Outcome::Incomplete));
                assert_eq!(blocked.points, vec![request.head]);
                assert!(blocked.replacements.is_empty());
            }
            assert_eq!(source, encode_documents(s.snapshot().unwrap()).unwrap());
        }
    }
}

/// Independently validate a clean return around the branch against the compound transaction.
fn validate_clean_branch_return(s: &Session, upper: Uuid, lower: Uuid, points: Vec<Point>) {
    let mut witness = Session::default();
    witness.open_snapshot(s.snapshot().unwrap());
    witness.apply(ADD_TRACKS,&json!({"board":view(&witness).target.board,"layer":copper_layers(&witness)[0],"width_nm":250_000,
        "points":points,
        "replacements":[
            {"track":upper,"points":[Point{x_nm:-975_000,y_nm:5_000_000},Point{x_nm:-13_176,y_nm:5_000_000},Point{x_nm:7_027_828,y_nm:-2_041_004},Point{x_nm:14_394_572,y_nm:-2_041_004},Point{x_nm:17_000_000,y_nm:564_424},Point{x_nm:17_000_000,y_nm:5_000_000},Point{x_nm:19_025_000,y_nm:5_000_000}]},
            {"track":lower,"points":[Point{x_nm:0,y_nm:5_800_000},Point{x_nm:7_266_002,y_nm:-1_466_002},Point{x_nm:14_156_398,y_nm:-1_466_002},Point{x_nm:16_424_998,y_nm:802_598},Point{x_nm:16_424_998,y_nm:5_238_174},Point{x_nm:16_986_824,y_nm:5_800_000},Point{x_nm:17_000_000,y_nm:5_800_000},Point{x_nm:20_000_000,y_nm:5_800_000}]}
        ],"expected_revision":witness.revision()})).expect("the lateral branch corridor has an independently validated witness");
}

/// A lateral push must keep a return corridor to the fixed branch instead of trapping
/// the second rail behind the first speculative replacement.
#[test]
fn lateral_shove_keeps_a_valid_return_to_the_branch() {
    for reverse in [false, true] {
        let (s, upper, lower) = locked_channel(reverse);
        validate_clean_branch_return(
            &s,
            upper,
            lower,
            vec![
                Point { x_nm: 2_159_000, y_nm: 6_604_000 },
                Point { x_nm: 6_350_000, y_nm: 6_604_000 },
                Point { x_nm: 13_970_000, y_nm: -1_016_000 },
            ],
        );
        for x in [11_176_000, 12_192_000, 13_208_000, 13_970_000] {
            let request = Request { head: Point { x_nm: 2_159_000, y_nm: 6_604_000 }, target: Point { x_nm: x, y_nm: -1_016_000 }, ..request(&s) };
            let polls = std::cell::Cell::new(0);
            let plan = s
                .plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| {
                    polls.set(polls.get() + 1);
                    false
                })
                .unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "x={x}: {plan:?}");
            assert_eq!(plan.replacements.len(), 2);
            assert_eq!(plan.shove.as_ref().unwrap().attempts, 2);
            if x == 13_970_000 && !reverse {
                let before = encode_documents(s.snapshot().unwrap()).unwrap();
                let context = s.route_planning(view(&s).target).unwrap();
                let mut job = s.prepare_route_plan(&context, request.clone(), DEFAULT_BUDGET).unwrap();
                let mut done = false;
                for _ in 0..100_000 {
                    let progress = job.progress().completed;
                    if job.step(1) {
                        done = true;
                        break;
                    }
                    assert!(job.progress().completed <= progress + 1);
                }
                assert!(done);
                assert_eq!(job.take_result().unwrap().unwrap(), plan);
                let limited = s.plan_route(&query(&s), &request, plan.probes - 1, &|| false).unwrap();
                assert!(limited.probes < plan.probes);
                match limited.outcome {
                    Outcome::Reached => {
                        // A smaller allowance can skip optional refinements and still
                        // certify the same two-rail cascade within the available work.
                        assert_eq!(limited.replacements.len(), 2);
                        let mut candidate = Session::default();
                        candidate.open_snapshot(s.snapshot().unwrap());
                        commit(&mut candidate, &request, &limited);
                    }
                    Outcome::Incomplete => {
                        assert_eq!(limited.points, vec![request.head]);
                        assert!(limited.replacements.is_empty());
                    }
                    other => panic!("unexpected limited outcome: {other:?}"),
                }
                let count = std::cell::Cell::new(0);
                let canceled = s
                    .plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| {
                        count.set(count.get() + 1);
                        count.get() >= polls.get() - 1
                    })
                    .unwrap();
                assert_eq!(canceled.outcome, Outcome::Canceled);
                assert!(canceled.replacements.is_empty());
                assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
            }
            let mut candidate = Session::default();
            candidate.open_snapshot(s.snapshot().unwrap());
            commit(&mut candidate, &request, &plan);
        }
    }
}

/// Returning to a fixed branch should not introduce a long hairpin beside the incoming route.
#[test]
fn displaced_return_does_not_backtrack_along_the_incoming_route() {
    for reverse in [false, true] {
        let (s, upper, lower) = locked_channel(reverse);
        let head = Point { x_nm: 3_556_000, y_nm: 7_366_000 };
        let target = Point { x_nm: 13_462_000, y_nm: 254_000 };
        validate_clean_branch_return(&s, upper, lower, vec![head, Point { x_nm: 6_350_000, y_nm: head.y_nm }, target]);
        let request = Request { head, target, ..request(&s) };
        let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
        let displaced = &plan.replacements.iter().find(|r| r.track == lower).unwrap().points;
        let retreat: i64 = displaced.windows(2).map(|p| (p[0].x_nm - p[1].x_nm).max(0)).sum();
        assert!(retreat <= 250_000, "unnecessary return backtracks{retreat}nm: {plan:?}");
        let mut candidate = Session::default();
        candidate.open_snapshot(s.snapshot().unwrap());
        commit(&mut candidate, &request, &plan);
    }
}

/// Unrelated tracks behind the push must not consume the forward-guide quota. Both
/// source orders, posture senses and cursor progressions retain a legal two-rail shove.
#[test]
fn bent_approach_below_the_rails_still_shoves() {
    for reverse in [false, true] {
        for flipped in [false, true] {
            for unrelated in [0, 12] {
                let (mut s, upper, lower) = locked_channel(reverse);
                let mut unchanged = Vec::new();
                for i in 0..unrelated {
                    let x = -12_000_000 + i * 500_000;
                    let id = add(&mut s, vec![Point { x_nm: x, y_nm: 9_000_000 }, Point { x_nm: x, y_nm: 14_000_000 }], 250_000)[0];
                    unchanged.push(view(&s).tracks().iter().find(|t| t.id == id).unwrap().clone());
                }
                let point = |x, y| Point { x_nm: x, y_nm: y };
                let prefix = [point(-3_556_000, 3_810_000), point(-3_556_000, 6_096_000), point(254_000, 9_906_000), point(4_318_000, 9_906_000)];
                let mut req = request(&s);
                req.head = *prefix.last().unwrap();
                req.origin = Some((prefix[0], req.layer));
                req.flipped = flipped;
                req.fixed = prefix
                    .windows(2)
                    .map(|pair| FixedCopper::Segment { start: pair[0], end: pair[1], layer: req.layer, width_nm: req.width_nm })
                    .collect();
                let before = encode_documents(s.snapshot().unwrap()).unwrap();
                let q = query(&s);
                let mut last = None;
                for y in [6_858_000, 6_604_000, 6_350_000, 6_096_000, 5_842_000, 5_588_000, 5_334_000, 5_080_000] {
                    req.target = point(7_874_000, y);
                    let result = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| false).unwrap();
                    assert_eq!(result.outcome, Outcome::Reached, "reverse{reverse} flipped{flipped} unrelated{unrelated} y{y}: {result:?}");
                    assert!(result.probes <= DEFAULT_BUDGET);
                    if y <= 5_334_000 {
                        assert!(result.replacements.iter().any(|r| r.track == lower), "{result:?}");
                        assert!(result.replacements.iter().any(|r| r.track == upper), "{result:?}");
                    }
                    req.previous = result.points.clone();
                    last = Some(result);
                }
                assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
                let result = last.unwrap();
                commit(&mut s, &req, &result);
                for original in unchanged {
                    assert_eq!(view(&s).tracks().iter().find(|t| t.id == original.id), Some(&original));
                }
            }
        }
    }
}

/// A short push between long rails must localize their bends rather than sweep
/// distant hooked copper into the cascade. Rotation and source insertion order
/// change without changing the available channel or its clearance requirements.
#[test]
fn local_rail_displacements_leave_distant_hooks_untouched() {
    for turns in 0..4 {
        for reverse in [false, true] {
            let mut s = board();
            let point = |x: i64, y: i64| {
                let (mut x, mut y) = (x, y);
                for _ in 0..turns {
                    (x, y) = (-y, x);
                }
                Point { x_nm: x + 30_000_000, y_nm: y - 20_000_000 }
            };
            let mut paths = vec![
                (vec![point(-975_000, 5_000_000), point(19_025_000, 5_000_000)], 500_000),
                (vec![point(0, 5_800_000), point(20_000_000, 5_800_000)], 250_000),
                (vec![point(17_000_000, 5_000_000), point(17_000_000, 8_000_000)], 250_000),
                (
                    vec![
                        point(-400_000, 1_000_000),
                        point(-400_000, 4_400_000),
                        point(7_200_000, 4_400_000),
                        point(8_800_000, 2_800_000),
                        point(14_000_000, 2_800_000),
                    ],
                    250_000,
                ),
            ];
            // Additional legal source fragments supply fixed guide caps near the
            // far side of the channel, outside the requested local displacement.
            for i in 0..12 {
                let x = -3_000_000 - i * 700_000;
                paths.push((vec![point(x, 1_000_000), point(x, 4_000_000)], 250_000));
            }
            if reverse {
                paths.reverse();
            }
            for (points, width) in paths {
                add(&mut s, points, width);
            }
            let unchanged: Vec<_> = view(&s)
                .tracks()
                .iter()
                .filter(|t| {
                    t.start == point(-400_000, 1_000_000)
                        || t.start == point(-400_000, 4_400_000)
                        || t.start == point(7_200_000, 4_400_000)
                        || t.start == point(8_800_000, 2_800_000)
                })
                .cloned()
                .collect();
            let mut req = request(&s);
            let origin = point(4_826_000, 10_414_000);
            req.head = point(5_588_000, 10_414_000);
            req.origin = Some((origin, req.layer));
            req.flipped = turns % 2 == 0;
            req.fixed = vec![FixedCopper::Segment { start: origin, end: req.head, layer: req.layer, width_nm: req.width_nm }];
            let q = query(&s);
            let mut last = None;
            for y in [6_350_000, 5_842_000, 5_334_000, 4_826_000, 4_572_000] {
                req.target = point(11_176_000, y);
                let plan = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| false).unwrap();
                assert_eq!(plan.outcome, Outcome::Reached, "rotation{turns} reverse{reverse} y{y}: {plan:?}");
                assert!(plan.probes <= DEFAULT_BUDGET);
                assert!(unchanged.iter().all(|t| plan.replacements.iter().all(|r| r.track != t.id)), "{plan:?}");
                req.previous = plan.points.clone();
                last = Some(plan);
            }
            commit(&mut s, &req, &last.unwrap());
            for track in unchanged {
                assert_eq!(view(&s).tracks().iter().find(|t| t.id == track.id), Some(&track));
            }
        }
    }
}

/// A chamfered trace yields its obstructed ordinary bends while an external
/// branch remains fixed. Every original record survives the atomic replacement.
#[test]
fn shove_moves_ordinary_bends_and_preserves_chain_contacts() {
    for flipped in [false, true] {
        for reverse in [false, true] {
            let mut s = board();
            let point = |x, y| Point { x_nm: x, y_nm: y };
            let mut shape = vec![
                point(0, 5_800_000),
                point(8_000_000, 5_800_000),
                point(8_254_000, 5_546_000),
                point(12_000_000, 5_546_000),
                point(12_254_000, 5_800_000),
                point(20_000_000, 5_800_000),
            ];
            if reverse {
                shape.reverse();
            }
            let originals = add(&mut s, shape, 250_000);
            let branch = add(&mut s, vec![point(17_000_000, 5_800_000), point(17_000_000, 9_000_000)], 500_000)[0];
            let old_branch = view(&s).tracks().iter().find(|t| t.id == branch).unwrap().clone();
            let req = Request { head: point(8_254_000, 10_000_000), target: point(8_254_000, 4_000_000), flipped, ..request(&s) };
            let q = query(&s);
            let before = encode_documents(s.snapshot().unwrap()).unwrap();
            let plan = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| false).unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "reverse{reverse} flipped{flipped}: {plan:?}");
            assert!(plan.replacements.len() >= 2);
            assert!(plan.replacements.iter().all(|r| originals.contains(&r.track)));
            assert!(!plan.replacements.iter().flat_map(|r| &r.points).any(|p| *p == point(8_254_000, 5_546_000)), "blocked bend was retained");
            assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
            let context = s.route_planning(view(&s).target).unwrap();
            let mut job = s.prepare_route_plan(&context, req.clone(), DEFAULT_BUDGET).unwrap();
            let mut done = false;
            for _ in 0..100_000 {
                if job.step(1) {
                    done = true;
                    break;
                }
            }
            assert!(done);
            assert_eq!(job.take_result().unwrap().unwrap(), plan);
            let canceled = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| true).unwrap();
            assert_eq!(canceled.outcome, Outcome::Canceled);
            assert!(canceled.replacements.is_empty());
            let exhausted = s.plan_route(&q, &req, 1, &|| false).unwrap();
            assert_eq!(exhausted.outcome, Outcome::Incomplete);
            assert!(exhausted.replacements.is_empty());
            commit(&mut s, &req, &plan);
            for id in &originals {
                assert!(view(&s).tracks().iter().any(|t| t.id == *id));
            }
            assert_eq!(view(&s).tracks().iter().find(|t| t.id == branch), Some(&old_branch));
        }
    }
}

/// A mixed-width cascade must move ordinary chamfers in a third trace rather
/// than pinning its bends. Both routing postures and original path orientations
/// publish through the same independently validated atomic command.
#[test]
fn rail_cascade_moves_a_neighboring_chamfered_trace() {
    for reverse in [false, true] {
        for flipped in [false, true] {
            let (mut s, upper, lower) = locked_channel(reverse);
            let point = |x, y| Point { x_nm: x, y_nm: y };
            let mut hook = vec![
                point(-375_000, 1_000_000),
                point(-375_000, 4_400_000),
                point(7_200_000, 4_400_000),
                point(8_800_000, 2_800_000),
                point(14_224_000, 2_800_000),
                point(14_478_000, 3_054_000),
                point(16_000_000, 3_054_000),
            ];
            if reverse {
                hook.reverse();
            }
            let originals = add(&mut s, hook, 250_000);
            let origin = point(4_826_000, 10_414_000);
            let req = Request {
                head: point(5_588_000, 10_414_000),
                target: point(11_176_000, 1_778_000),
                origin: Some((origin, copper_layers(&s)[0])),
                fixed: vec![FixedCopper::Segment {
                    start: origin,
                    end: point(5_588_000, 10_414_000),
                    layer: copper_layers(&s)[0],
                    width_nm: 250_000,
                }],
                flipped,
                ..request(&s)
            };
            let plan = s.plan_route(&query(&s), &req, DEFAULT_BUDGET, &|| false).unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "reverse{reverse} flipped{flipped}: {plan:?}");
            assert!(plan.replacements.iter().any(|r| r.track == upper));
            assert!(plan.replacements.iter().any(|r| r.track == lower));
            assert!(plan.replacements.iter().filter(|r| originals.contains(&r.track)).count() >= 2);
            assert!(plan.probes <= DEFAULT_BUDGET);
            commit(&mut s, &req, &plan);
            for id in originals {
                assert!(view(&s).tracks().iter().any(|t| t.id == id));
            }
        }
    }
}

/// Invalid free pickups are checked against the original body at both ordinary
/// bends and segment interiors. A later clear pickup remains immediately usable.
#[test]
fn free_pickups_inside_original_clearance_are_refused_before_chain_preparation() {
    let mut s = board();
    let p = |x, y| Point { x_nm: x, y_nm: y };
    add(&mut s, vec![p(4_000_000, -7_620_000), p(4_000_000, -2_540_000), p(10_160_000, -2_540_000)], 500_000);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let q = query(&s);
    for y in [-2_540_000, -5_000_000] {
        let req = Request { head: p(3_500_000, y), target: p(1_270_000, -5_080_000), ..request(&s) };
        let plan = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| false).unwrap();
        assert!(plan.start_blocked);
        assert!(matches!(plan.outcome, Outcome::Blocked { .. }));
        assert_eq!(plan.points, [req.head]);
        assert!(plan.replacements.is_empty());
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    }
    let req = Request { head: p(1_270_000, -5_080_000), target: p(2_540_000, -5_080_000), ..request(&s) };
    let plan = s.plan_route(&q, &req, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached);
    commit(&mut s, &req, &plan);
}

/// Two off-angle centerlines can meet far from every projected endpoint and midpoint.
#[test]
fn shove_keeps_a_proper_crossing_as_a_protected_contact() {
    let mut s = board();
    let rail = add(&mut s, vec![p(0, 0), p(100, 0)], 250_000)[0];
    let crossing = add(&mut s, vec![p(20, -20), p(60, 60)], 250_000)[0];
    let request = Request { head: p(60, 20), target: p(80, 1), ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    let path = &plan.replacements.iter().find(|r| r.track == rail).expect("the intersecting rail should move").points;
    assert_ne!(path, &[p(0, 0), p(100, 0)]);
    let junction = p(30, 0);
    assert!(
        path.windows(2).any(|leg| leg[0].y_nm == 0
            && leg[1].y_nm == 0
            && (leg[0].x_nm.min(leg[1].x_nm)..=leg[0].x_nm.max(leg[1].x_nm)).contains(&junction.x_nm)),
        "missing original crossing: {path:?}"
    );
    let original = view(&s).tracks().iter().find(|t| t.id == crossing).unwrap().clone();
    commit(&mut s, &request, &plan);
    assert_eq!(view(&s).tracks().iter().find(|t| t.id == crossing), Some(&original));
}

/// A dense trace outside the active copper layer cannot invalidate a clear route.
#[test]
fn capped_back_trace_does_not_stop_a_clear_front_route() {
    let mut s = board();
    let back = copper_layers(&s)[1];
    let add_back = |s: &mut Session, points: Vec<Point>| {
        s.apply(ADD_TRACKS, &json!({"layer":back,"points":points,"width_nm":200_000,"expected_revision":s.revision()})).unwrap();
    };
    add_back(&mut s, vec![p(0, 0), p(1300, 0)]);
    let mut zigzag = vec![p(0, 30)];
    for i in 0..127 {
        zigzag.extend([p(10 * i + 5, 0), p(10 * i + 10, 30)]);
    }
    add_back(&mut s, zigzag);
    let request = request(&s);
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.replacements.is_empty());
}

/// A prefix must remain inside the outline with clearance for every segment and via land.
#[test]
fn shove_checks_fixed_segments_and_vias_against_the_outline() {
    let mut s = board();
    crate::board::drc::tests::outline(&mut s);
    let query = query(&s);
    let layer = copper_layers(&s)[0];
    let head = p(20, 50);
    let target = p(40, 50);
    for (piece, valid) in [
        (FixedCopper::Segment { start: p(0, 50), end: head, layer, width_nm: 250_000 }, true),
        (FixedCopper::Segment { start: p(-60, 50), end: head, layer, width_nm: 250_000 }, false),
        (FixedCopper::Via { at: p(-41, 50), diameter_nm: 1_000_000 }, true),
        (FixedCopper::Via { at: p(-45, 50), diameter_nm: 1_000_000 }, false),
        (FixedCopper::Via { at: p(-55, 50), diameter_nm: 1_000_000 }, false),
    ] {
        let request = Request { head, target, origin: Some((p(0, 50), layer)), fixed: vec![piece.clone()], ..request(&s) };
        let plan = s.plan_route(&query, &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome == Outcome::Reached, valid, "{piece:?}: {plan:?}");
        assert!(plan.replacements.is_empty());
    }
}

/// A fixed prefix and the proposed leg share the normal placement vertex limit.
#[test]
fn shove_uses_the_combined_prefix_and_continuation_point_limit() {
    let mut s = board();
    let layer = copper_layers(&s)[0];
    let points: Vec<_> = (0..crate::board::track::MAX_PATH_POINTS).map(|i| p(i as i64, 0)).collect();
    let make_request = |s: &Session, count| Request {
        head: points[count - 1],
        target: p(300, 0),
        origin: Some((points[0], layer)),
        fixed: points[..count].windows(2).map(|p| FixedCopper::Segment { start: p[0], end: p[1], layer, width_nm: 250_000 }).collect(),
        ..request(s)
    };
    let cap = points.len();
    let overflow = make_request(&s, cap);
    let plan = s.plan_route(&query(&s), &overflow, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Incomplete, "{plan:?}");
    let fits = make_request(&s, cap - 1);
    let plan = s.plan_route(&query(&s), &fits, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    commit(&mut s, &fits, &plan);
}

/// A wider continuation does not capture contacts the narrower pickup never touched.
#[test]
fn shove_captures_pickup_contacts_at_the_fixed_width() {
    let mut s = board();
    let at = |x_nm, y_nm| Point { x_nm, y_nm };
    add(&mut s, vec![at(-3_000_000, 450_000), at(3_000_000, 450_000)], 250_000);
    let layer = copper_layers(&s)[0];
    let origin = p(0, 0);
    let head = p(0, -20);
    let request = Request {
        head,
        origin: Some((origin, layer)),
        width_nm: 800_000,
        fixed: vec![FixedCopper::Segment { start: origin, end: head, layer, width_nm: 150_000 }],
        ..request(&s)
    };
    let query = query(&s);
    assert!(pickup(&query, &request).unwrap().islands.is_empty());
    assert_eq!(pickup(&query, &Request { fixed: Vec::new(), ..request }).unwrap().islands.len(), 1);
}

#[test]
fn shove_approaches_the_board_edge_and_recovers_when_the_cursor_returns() {
    let mut s = board();
    crate::board::drc::tests::outline(&mut s);
    let head = Point { x_nm: -4_550_000, y_nm: 13_223_000 };
    let target = Point { x_nm: -4_756_000, y_nm: 10_807_000 };
    let edge = Point { x_nm: -4_575_000, y_nm: 10_988_000 };
    let mut request = Request { head, target, ..request(&s) };
    for flipped in [false, true] {
        request.flipped = flipped;
        let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome, Outcome::Blocked { last_legal: edge }, "{plan:?}");
        assert_eq!(plan.adjusted_target, Some(edge));
        assert_eq!(plan.points.last(), Some(&edge));
        assert!(!plan.start_blocked);
        request.previous = plan.points;
    }
    let recovered = Request { target: Point { x_nm: -4_400_000, y_nm: target.y_nm }, ..request.clone() };
    let plan = s.plan_route(&query(&s), &recovered, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached);
    assert_eq!(plan.adjusted_target, None);

    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    s.apply(ADD_TRACKS, &json!({"layer":request.layer,"width_nm":request.width_nm,"points":plan.points,"expected_revision":s.revision()})).unwrap();
}

/// A short overlapping return on an existing trace is an anchored contact. Shove elsewhere
/// on the trace may move the adjacent rail while preserving the original return and its branch.
#[test]
fn a_distant_overlapping_return_does_not_block_a_rail_shove() {
    let mut s = board();
    let point = |x_nm, y_nm| Point { x_nm, y_nm };
    let (rail_end, bend) = (point(-11_430_000, 18_034_000), point(-11_430_000, 10_539_600));
    let overlap_end = point(-7_366_000, 13_208_000);
    let branch = point(-7_366_000, 12_446_000);
    add(
        &mut s,
        vec![rail_end, bend, point(-10_092_398, 9_201_998), point(-7_941_602, 9_201_998), point(-7_366_000, 9_777_600), overlap_end],
        250_000,
    );
    let overlap = add(&mut s, vec![overlap_end, branch], 250_000)[0];
    add(&mut s, vec![branch, point(-3_810_000, 8_890_000)], 250_000);
    for x in [-9_906_000, -8_890_000, -8_128_000] {
        add(&mut s, vec![point(x, 9_652_000), point(x, 20_320_000)], 250_000);
    }
    let rail = view(&s).tracks().iter().find(|t| [t.start, t.end] == [rail_end, bend]).unwrap().id;
    let original_overlap = view(&s).tracks().iter().find(|t| t.id == overlap).unwrap().clone();
    let request = Request { head: point(-12_192_000, 14_224_000), target: point(-11_430_000, 14_224_000), ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.replacements.iter().any(|path| path.track == rail), "{plan:?}");
    commit(&mut s, &request, &plan);
    assert_eq!(view(&s).tracks().iter().find(|t| t.id == overlap), Some(&original_overlap));
}

/// A long trace that crosses its own return has an interior junction. Moving a remote
/// interval must preserve the junction, while the horizontal source is allowed to reach.
#[test]
fn a_self_crossing_trace_can_shove_away_from_its_junction() {
    let mut s = board();
    let ends = [p(-150, 180), p(-150, -250)];
    let rail = vec![p(0, 0), p(-20, 0), p(-160, 140), p(-160, 170), ends[0], ends[1], p(-60, -340)];
    add(&mut s, rail, 250_000);
    let original = view(&s).tracks().iter().find(|t| [t.start, t.end] == ends).unwrap().id;
    let request = Request { head: p(-130, -80), target: p(-180, -100), ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.replacements.iter().any(|path| path.track == original), "{plan:?}");
    commit(&mut s, &request, &plan);
}

/// Once the other elbow reaches a new cursor target, turns needed by earlier
/// targets should not keep accumulating in the unfixed preview.
#[test]
fn shove_tries_both_elbows_before_carrying_previous_turns() {
    let mut session = board();
    let point = |x_nm, y_nm| Point { x_nm, y_nm };
    add(
        &mut session,
        vec![point(4_052_000, 11_938_000), point(8_636_000, 11_938_000), point(9_144_000, 12_446_000), point(12_046_000, 12_446_000)],
        250_000,
    );
    let head = point(4_826_000, 12_700_000);
    let previous =
        vec![head, point(5_501_000, 12_025_000), point(9_520_000, 12_025_000), point(9_630_000, 11_915_000), point(10_000_000, 11_915_000)];
    for (target, previous) in [(point(10_650_000, 11_915_000), previous), (point(8_890_000, 11_557_000), Vec::new())] {
        let request = Request { head, target, previous, flipped: true, ..request(&session) };
        let plan = session.plan_route(&query(&session), &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
        assert!([false, true].into_iter().any(|flipped| plan.points == super::super::posture::elbow(head, target, request.posture, flipped)));
        assert!(!plan.replacements.is_empty());
        commit(&mut session, &request, &plan);
        session.apply("edit.undo", &Value::Null).unwrap();
    }
}

/// A cursor advance can reopen an earlier choice of approach. Retrying the last
/// legal guide retains its turns when both direct elbow senses trap the cascade.
#[test]
fn continuing_a_guide_retries_the_incoming_path() {
    for direction in [-1_i64, 1] {
        let point = |x_nm, y_nm| Point { x_nm: direction * x_nm, y_nm };
        let mut s = board();
        add(&mut s, vec![point(6_604_000, 8_636_000), point(-12_446_000, 8_636_000)], 250_000);
        add(&mut s, vec![point(761_422, 8_185_998), point(9_709_998, 8_185_998)], 250_000);
        add(&mut s, vec![point(-10_092_398, 9_201_998), point(-7_941_602, 9_201_998)], 250_000);
        add(&mut s, vec![point(-7_366_000, 12_446_000), point(-4_064_000, 9_144_000)], 250_000);
        let head = point(10_922_000, 10_668_000);
        let target = point(-8_128_000, 8_128_000);
        let previous = vec![
            head,
            point(10_859_329, 10_730_671),
            point(-953_329, 10_730_671),
            point(-3_810_000, 7_874_000),
            point(-5_080_000, 7_874_000),
            point(-5_588_000, 8_382_000),
        ];
        let request = Request { head, target, previous, ..request(&s) };
        let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
        assert_eq!(plan.outcome, Outcome::Reached, "direction {direction}: {plan:?}");
        assert_eq!(plan.points.first(), Some(&head));
        assert_eq!(plan.points.last(), Some(&target));
        assert!(!plan.replacements.is_empty(), "{plan:?}");
        assert!(plan.probes <= DEFAULT_BUDGET);
        commit(&mut s, &request, &plan);
    }
}

/// A cached guide can fold as the cursor moves. Reusing its endpoint must
/// certify the incoming copper against itself before applying any displacement.
#[test]
fn shove_rebuilds_a_guide_that_folds_back_into_its_own_clearance() {
    let mut s = board();
    let head = p(0, 0);
    let target = p(12, 4);
    let previous = vec![head, p(10, 0), p(10, 2), p(2, 2), p(2, 4), target];
    let request = Request { head, target, previous, ..request(&s) };
    let result = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(result.outcome, Outcome::Reached, "{result:?}");
    assert_eq!(result.points, super::super::posture::elbow(head, target, request.posture, request.flipped));
    commit(&mut s, &request, &result);
}

/// An incoming extension cannot run back into an earlier fixed leg of the same gesture.
#[test]
fn shove_refuses_to_overlap_an_earlier_fixed_vertex() {
    let s = board();
    let layer = copper_layers(&s)[0];
    let (origin, corner, head) = (p(0, 0), p(50, 0), p(50, 30));
    let fixed = [origin, corner, head].windows(2).map(|leg| FixedCopper::Segment { start: leg[0], end: leg[1], layer, width_nm: 250_000 }).collect();
    let request = Request { head, target: p(20, 0), origin: Some((origin, layer)), fixed, ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert_ne!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert!(plan.replacements.is_empty());
}

/// Reserving all three returns together lets the gesture pass staggered bends
/// where individually certified displacements can trap the next run.
#[test]
fn shove_coordinates_returns_across_staggered_runs() {
    for sign in [-1, 1] {
        let mut session = board();
        let point = |(x, y): (i64, i64)| Point { x_nm: 1_000 * x, y_nm: sign * 1_000 * y };
        let rails: &[&[(i64, i64)]] = &[
            &[(-14_986, 18_034), (-14_986, -24_638)],
            &[(-4_064, -31_242), (-11_176, -24_130), (-11_176, -10_414)],
            &[(3_048, -36_830), (-6_350, -27_432), (-6_350, -18_796), (-7_874, -17_272), (-7_874, -14_224)],
        ];
        let tracks: BTreeSet<_> = rails.iter().flat_map(|rail| add(&mut session, rail.iter().copied().map(point).collect(), 250_000)).collect();
        let head = point((-25_400, -14_986));
        let mut previous =
            [(-25_400, -14_986), (-20_574, -14_986), (-16_764, -18_796), (-7_620, -18_796), (-7_366, -19_050)].into_iter().map(point).collect();
        for x in [-7_112, -5_080, -2_032] {
            let target = point((x, -19_050));
            let request = Request { head, target, previous, ..request(&session) };
            let budget = request.mode.default_probe_budget();
            let context = session.route_planning(view(&session).target).unwrap();
            let mut job = session.prepare_route_plan(&context, request.clone(), budget).unwrap();
            let mut before = 0;
            let mut done = false;
            for _ in 0..100_000 {
                done = job.step(1);
                let after = job.progress().completed;
                assert!((before..=before + 1).contains(&after), "a window rewound or overcharged the shared budget");
                before = after;
                if done {
                    break;
                }
            }
            assert!(done && before <= budget);
            let plan = job.take_result().unwrap().unwrap();
            assert_eq!(plan.points.last(), Some(&target), "{plan:?}");
            assert_eq!(plan.replacements.iter().map(|part| part.track).collect::<BTreeSet<_>>(), tracks);
            commit(&mut session, &request, &plan);
            session.apply("edit.undo", &Value::Null).unwrap();
            previous = plan.points;
        }
    }
}

/// An incoming route can carry a shove across a wide row of unlocked copper.
#[test]
fn shove_displaces_more_than_ninety_six_rails_in_one_edit() {
    let mut s = board();
    let first = add(&mut s, vec![p(-1500, 0), p(1500, 0)], 250_000)[0];
    for rail in 1..112 {
        add(&mut s, vec![p(-1500, rail * 8), p(1500, rail * 8)], 250_000);
    }
    let request = Request { head: p(0, -10), target: p(0, 111 * 8 + 1), ..request(&s) };
    let plan = s.plan_route(&query(&s), &request, request.mode.default_probe_budget(), &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "large cascade stopped after {} probes", plan.probes);
    assert_eq!(plan.replacements.len(), 112);
    let displaced = plan.replacements.iter().find(|path| path.track == first).unwrap();
    assert!(displaced.points.iter().any(|point| point.y_nm > request.target.y_nm));
    assert_eq!(displaced.points.first(), Some(&p(-1500, 0)));
    assert_eq!(displaced.points.last(), Some(&p(1500, 0)));
    commit(&mut s, &request, &plan);
}

/// A through via and its endpoint tracks on both sides move together in one edit.
#[test]
fn a_route_moves_the_via_and_its_attached_segments() {
    let mut s = board();
    let at = p(50, 0);
    let layers = copper_layers(&s);
    s.apply(crate::ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
    let via = view(&s).vias()[0].id;
    let lone_at = p(35, 0);
    s.apply(crate::ADD_VIA, &json!({"at":lone_at,"expected_revision":s.revision()})).unwrap();
    let lone = view(&s).vias().iter().find(|v| v.at == lone_at).unwrap().id;
    for layer in &layers {
        s.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":250000,"points":[p(50,-30),at],"expected_revision":s.revision()})).unwrap();
    }
    let mut request = request(&s);
    request.head = p(20, 3);
    request.target = p(80, 3);
    request.mode = Mode::ShoveVias;
    let plan = s.plan_route(&query(&s), &request, MAX_BUDGET, &|| false).unwrap();
    assert_eq!(plan.points, vec![request.head, request.target]);
    assert_eq!(plan.via_moves.len(), 2, "{plan:?}");
    let fixed = Request {
        head: request.target,
        origin: Some((request.head, request.layer)),
        fixed: vec![FixedCopper::Segment { start: request.head, end: request.target, layer: request.layer, width_nm: request.width_nm }],
        ..request.clone()
    };
    let at_vertex = s.plan_route(&query(&s), &fixed, MAX_BUDGET, &|| false).unwrap();
    assert_eq!(at_vertex.outcome, Outcome::Reached, "{at_vertex:?}");
    assert_eq!(at_vertex.points, vec![fixed.head]);
    assert_eq!(at_vertex.via_moves, plan.via_moves);
    assert_eq!(at_vertex.replacements, plan.replacements);
    let moved = plan.via_moves.iter().find(|m| m.via == via).unwrap().at;
    let free = plan.via_moves.iter().find(|m| m.via == lone).unwrap().at;
    assert_eq!(free.x_nm, lone_at.x_nm);
    assert_eq!(free.y_nm, moved.y_nm);
    assert!(moved.y_nm < at.y_nm);
    assert_eq!(moved.x_nm, at.x_nm);
    assert_eq!(plan.replacements.len(), 2);
    commit(&mut s, &request, &plan);
    assert_eq!(view(&s).vias().iter().find(|v| v.id == via).unwrap().at, moved);
    assert_eq!(view(&s).vias().iter().find(|v| v.id == lone).unwrap().at, free);
    for layer in layers {
        assert!(view(&s).tracks().iter().any(|t| t.layer == layer && t.start == p(50, -30) && t.end == moved));
    }
    assert!(!query(&s).exhaustive_pairs().decisions.iter().any(|d| d.clearance_violation || d.conflicting_nets));
}

/// Shove on D stretches a connected neighbor and pushes an unrelated track atomically.
#[test]
fn connected_drag_pushes_a_neighbor() {
    let mut s = board();
    let a = add(&mut s, vec![p(20, 20), p(80, 20)], 250000)[0];
    let b = add(&mut s, vec![p(20, 40), p(20, 20)], 250000)[0];
    let obstacle = add(&mut s, vec![p(0, 0), p(100, 0)], 500000)[0];
    let tracks = json!([{"track":a,"start":p(20,1),"end":p(80,1)},{"track":b,"start":p(20,40),"end":p(20,1)}]);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply(crate::RESHAPE_TRACKS, &json!({"tracks":tracks,"mode":"shove","expected_revision":s.revision()})).unwrap();
    assert!(view(&s).tracks().iter().any(|t| t.id == a && t.start == p(20, 1) && t.end == p(80, 1)));
    assert!(view(&s).tracks().iter().any(|t| t.id == b && t.end == p(20, 1)));
    assert!(view(&s).tracks().iter().any(|t| t.id == obstacle && t.end != p(100, 0)));
    assert!(!query(&s).exhaustive_pairs().decisions.iter().any(|d| d.clearance_violation || d.conflicting_nets));
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// A direct collision is visible to highlight mode. Walkaround moves the proposed track
/// around the fixed via and its followers, keeping the reshaped branch attached.
#[test]
fn fixed_drag_modes_keep_the_via_and_its_followers_fixed() {
    let mut s = board();
    let a = add(&mut s, vec![p(20, 20), p(80, 20)], 250_000)[0];
    let b = add(&mut s, vec![p(20, 40), p(20, 20)], 300_000)[0];
    let via = p(50, 0);
    s.apply(crate::ADD_VIA, &json!({"at":via,"expected_revision":s.revision()})).unwrap();
    let layers = copper_layers(&s);
    for layer in layers {
        s.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":250_000,"points":[p(50,-28),via],"expected_revision":s.revision()})).unwrap();
    }
    let fixed: Vec<_> = view(&s).tracks().iter().filter(|track| track.id != a && track.id != b).cloned().collect();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let params = |mode| {
        json!({"tracks":[{"track":a,"start":p(20,0),"end":p(80,0)},{"track":b,"start":p(20,40),"end":p(20,0)}],
               "mode":mode,"expected_revision":revision})
    };
    let error = s.apply(crate::RESHAPE_TRACKS, &params("highlight")).unwrap_err().to_string();
    assert!(error.contains("clearance"), "{error}");
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);

    s.apply(crate::RESHAPE_TRACKS, &params("walkaround")).unwrap();
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(view(&s).vias()[0].at, via);
    assert!(fixed.iter().all(|original| view(&s).tracks().contains(original)));
    assert!(view(&s).tracks().iter().any(|track| track.id == b && track.end == p(20, 0)));
    assert!(view(&s).tracks().len() > fixed.len() + 2);
    assert!(!query(&s).exhaustive_pairs().decisions.iter().any(|hit| hit.clearance_violation || hit.conflicting_nets));
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// Highlight plans expose a blocked preferred path while walkaround detours and can be placed.
#[test]
fn highlight_route_keeps_the_preferred_path_on_collision() {
    let mut s = board();
    let via = p(50, 0);
    s.apply(crate::ADD_VIA, &json!({"at":via,"expected_revision":s.revision()})).unwrap();
    let request = Request { head: p(20, 0), target: p(80, 0), mode: Mode::Highlight, ..request(&s) };
    let snapshot = query(&s);
    let plan = s.plan_route(&snapshot, &request, DEFAULT_BUDGET, &|| false).unwrap();
    assert!(matches!(plan.outcome, Outcome::Blocked { .. }));
    assert_eq!(plan.points, [request.head, request.target]);
    assert!(plan.blocker.is_some() && plan.adjusted_target.is_none() && plan.replacements.is_empty() && plan.via_moves.is_empty());
    let route = s.plan_route(&snapshot, &Request { mode: Mode::Walkaround, ..request }, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(route.outcome, Outcome::Reached);
    assert!(route.points.len() > 2);
}

/// The mouse-drag fixture can walk around the rail while retaining its perpendicular follower.
#[test]
fn walkaround_drag_clears_a_parallel_rail() {
    let mut s = board();
    let p = |x_nm, y_nm| Point { x_nm, y_nm };
    let start = p(2_540_000, -5_080_000);
    let fixed = p(2_540_000, -2_540_000);
    let a = add(&mut s, vec![start, p(7_620_000, -5_080_000)], 250_000)[0];
    let b = add(&mut s, vec![fixed, start], 250_000)[0];
    add(&mut s, vec![p(4_200_000, -7_720_000), p(6_300_000, -7_720_000)], 250_000);
    let joint = p(2_540_000, -7_620_000);
    s.apply(
        crate::RESHAPE_TRACKS,
        &json!({"tracks":[{"track":a,"start":joint,"end":p(7_620_000,-7_620_000)},{"track":b,"start":fixed,"end":joint}],
               "mode":"walkaround","expected_revision":s.revision()}),
    )
    .unwrap();
}

/// A manual compound commit cannot omit or move a locked follower, including one on inner copper.
#[test]
fn a_locked_inner_layer_endpoint_pins_its_via() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":s.revision()})).unwrap();
    let inner = view(&s)
        .layers()
        .into_iter()
        .find(|l| l.purpose == crate::board::setup::LayerPurpose::Copper && !copper_layers(&s).contains(&l.id))
        .unwrap()
        .id;
    let at = p(50, 0);
    s.apply(crate::ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
    let via = view(&s).vias()[0].id;
    s.apply(ADD_TRACKS, &json!({"layer":inner,"points":[p(50,-30),at],"expected_revision":s.revision()})).unwrap();
    let follower = view(&s).tracks()[0].id;
    let mut request = request(&s);
    request.head = p(20, 3);
    request.target = p(80, 3);
    request.mode = Mode::ShoveVias;
    let plan = s.plan_route(&query(&s), &request, MAX_BUDGET, &|| false).unwrap();
    assert_eq!(plan.via_moves.len(), 1);
    let params = |s: &Session| json!({"layer":request.layer,"points":plan.points,"via_moves":plan.via_moves,"replacements":plan.replacements,"expected_revision":s.revision()});
    let mut incomplete = params(&s);
    incomplete["replacements"] = json!([]);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(s.apply(ADD_TRACKS, &incomplete).is_err());
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let original = params(&s);
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[follower],"locked":true,"expected_revision":s.revision()})).unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert!(s.apply(ADD_TRACKS, &original).is_err());
    assert!(s.apply(ADD_TRACKS, &params(&s)).is_err());
    assert!(s.plan_route(&query(&s), &request, MAX_BUDGET, &|| false).unwrap().via_moves.is_empty());
    assert_eq!(view(&s).vias().iter().find(|v| v.id == via).unwrap().at, at);
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Shove-drag must preserve the original contact even when another path would keep the island connected.
#[test]
fn a_drag_cannot_pull_off_its_locked_neighbor() {
    let mut s = board();
    let moved = add(&mut s, vec![p(0, 0), p(40, 0)], 250000)[0];
    let locked = add(&mut s, vec![p(40, 0), p(80, 0)], 250000)[0];
    add(&mut s, vec![p(0, 0), p(0, -20), p(80, -20), p(80, 0)], 250000);
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[locked],"locked":true,"expected_revision":s.revision()})).unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let params = json!({"tracks":[{"track":moved,"start":p(0,0),"end":p(40,10)}],"mode":"shove","expected_revision":revision});
    let mut abandoned = s.prepare_copper_edit(crate::RESHAPE_TRACKS, &params).unwrap();
    assert!(!abandoned.step(1));
    drop(abandoned);
    let error = s.apply(crate::RESHAPE_TRACKS, &params).unwrap_err().to_string();
    assert!(error.contains("lose a protected copper contact"), "{error}");
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// A via follower may pivot while its unchanged endpoint stays inside an oval land, off the pad axis.
#[test]
fn via_shove_keeps_an_off_axis_oval_pad_contact() {
    use crate::board::{definition::PadShape, drc::contour_tests::fixture};

    let mut s = fixture(PadShape::Oval, 2_000_000, 1_000_000, p(100, 0), 200_000);
    let at = p(0, -50);
    let pad_end = Point { x_nm: 0, y_nm: -250_000 };
    s.apply(crate::ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
    let track = add(&mut s, vec![pad_end, at], 250_000)[0];
    let request = Request { head: p(-30, -47), target: p(30, -47), mode: Mode::ShoveVias, ..request(&s) };
    let snapshot = query(&s);
    let oval = snapshot.objects().iter().find(|item| item.at == p(0, 0)).unwrap();
    assert!(oval.contour.track_centerline().is_some(), "the oval must have a capsule core");
    let plan = s.plan_route(&snapshot, &request, MAX_BUDGET, &|| false).unwrap();
    assert_eq!(plan.via_moves.len(), 1, "{plan:?}");
    assert_eq!(plan.replacements, vec![Replacement { track, points: vec![pad_end, plan.via_moves[0].at] }]);
    commit(&mut s, &request, &plan);
}

/// A straight D-drag keeps its original angle even when neither routing elbow would hit the via.
#[test]
fn diagonal_drag_moves_a_via_on_the_actual_segment() {
    let mut s = board();
    let track = add(&mut s, vec![p(10, 40), p(90, 80)], 250_000)[0];
    let at = p(50, 20);
    s.apply(crate::ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
    let tracks = json!([{"track":track,"start":p(10,0),"end":p(90,40)}]);
    s.apply(crate::RESHAPE_TRACKS, &json!({"tracks":tracks,"mode":"shove_vias","expected_revision":s.revision()})).unwrap();
    let board = view(&s);
    assert_ne!(board.vias()[0].at, at);
    assert_eq!([board.tracks()[0].start, board.tracks()[0].end], [p(10, 0), p(90, 40)]);
    assert!(!query(&s).exhaustive_pairs().decisions.iter().any(|d| d.clearance_violation || d.conflicting_nets));
}

/// A via outside the conservative search region cannot exhaust the track solver's last probe.
#[test]
fn a_distant_via_leaves_the_track_shove_probe_budget_intact() {
    let mut s = board();
    s.apply(crate::ADD_VIA, &json!({"at":p(400, 400),"expected_revision":s.revision()})).unwrap();
    add(&mut s, vec![p(0, 0), p(100, 0)], 500_000);
    let request = request(&s);
    let snapshot = query(&s);
    let budget = s.plan_route(&snapshot, &request, MAX_BUDGET, &|| false).unwrap().probes;
    let track_only = s.plan_route(&snapshot, &request, budget, &|| false).unwrap();
    assert_eq!(track_only.outcome, Outcome::Reached);
    assert!(!track_only.replacements.is_empty());
    let plan = s.plan_route(&snapshot, &Request { mode: Mode::ShoveVias, ..request }, budget, &|| false).unwrap();
    assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
    assert_eq!(plan.points, track_only.points);
    assert_eq!(plan.replacements, track_only.replacements);
    assert_eq!(plan.probes, track_only.probes);
    assert!(plan.via_moves.is_empty());
}

/// An unrelated lock can force walkaround even after a proposed via exit clears the preferred path.
#[test]
fn a_rejected_incoming_path_discards_its_via_move() {
    let mut s = board();
    let at = p(40, 0);
    s.apply(crate::ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
    let locked = add(&mut s, vec![p(58, -10), p(58, 10)], 250_000)[0];
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[locked],"locked":true,"expected_revision":s.revision()})).unwrap();
    let request = Request { head: p(0, 3), target: p(90, 3), mode: Mode::ShoveVias, ..request(&s) };
    let snapshot = query(&s);
    let tentative = joints::Motion::preview_for_test(&snapshot, &request);
    assert_eq!(tentative.len(), 1, "the rejected guide must propose a via move");
    assert_eq!(tentative[0].via, view(&s).vias()[0].id);
    assert_ne!(tentative[0].at, at);
    let plan = s.plan_route(&snapshot, &request, MAX_BUDGET, &|| false).unwrap();
    assert_ne!(plan.points, vec![request.head, request.target]);
    assert!(plan.via_moves.is_empty(), "{plan:?}");
    commit(&mut s, &request, &plan);
    assert_eq!(view(&s).vias()[0].at, at);
}

/// Highlight keeps a crossing preview visible and marks its fixed-stroke collision as blocked.
#[test]
fn highlight_refuses_crossing_fixed_track() {
    let session = board();
    let mut route = Request { head: p(100, 100), target: p(50, -50), mode: Mode::Highlight, posture: Posture::Orthogonal, ..request(&session) };
    route.fixed = [p(0, 0), p(100, 0), route.head]
        .windows(2)
        .map(|pair| FixedCopper::Segment { start: pair[0], end: pair[1], layer: route.layer, width_nm: route.width_nm })
        .collect();
    let query = query(&session);
    let clear = session.plan_route(&query, &Request { target: p(50, 50), ..route.clone() }, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(clear.outcome, Outcome::Reached);
    let crossing = session.plan_route(&query, &route, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(crossing.points, [p(100, 100), p(50, 100), p(50, -50)]);
    assert_eq!(crossing.outcome, Outcome::Blocked { last_legal: route.head });
}

/// A width change at the head must retain earlier draft copper in the crossing check.
#[test]
fn highlight_refuses_crossing_fixed_track_at_a_different_width() {
    let session = board();
    let mut route = Request {
        head: p(100, 100),
        target: p(50, -50),
        width_nm: 500_000,
        mode: Mode::Highlight,
        posture: Posture::Orthogonal,
        ..request(&session)
    };
    route.fixed = [p(0, 0), p(100, 0), route.head]
        .windows(2)
        .map(|pair| FixedCopper::Segment { start: pair[0], end: pair[1], layer: route.layer, width_nm: 250_000 })
        .collect();
    let query = query(&session);
    let clear = session.plan_route(&query, &Request { target: p(50, 50), ..route.clone() }, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(clear.outcome, Outcome::Reached);
    let crossing = session.plan_route(&query, &route, DEFAULT_BUDGET, &|| false).unwrap();
    assert_eq!(crossing.points, [p(100, 100), p(50, 100), p(50, -50)]);
    assert_eq!(crossing.outcome, Outcome::Blocked { last_legal: route.head });
}

/// A parallel return must keep the sum of the two leg radii and their clearance.
#[test]
fn highlight_mixed_width_return_uses_both_radii() {
    let session = board();
    let query = query(&session);
    let spacing = 375_000 + query.authority().clearance_nm();
    for (gap, reaches_target) in [(spacing - 1, false), (spacing, true)] {
        let mut route = Request {
            head: Point { x_nm: 10_000_000, y_nm: gap },
            target: Point { x_nm: 5_000_000, y_nm: gap },
            width_nm: 500_000,
            mode: Mode::Highlight,
            posture: Posture::Orthogonal,
            ..request(&session)
        };
        route.fixed = [p(0, 0), p(100, 0), route.head]
            .windows(2)
            .map(|pair| FixedCopper::Segment { start: pair[0], end: pair[1], layer: route.layer, width_nm: 250_000 })
            .collect();
        let plan = session.plan_route(&query, &route, DEFAULT_BUDGET, &|| false).unwrap();
        let expected = if reaches_target { Outcome::Reached } else { Outcome::Blocked { last_legal: route.head } };
        assert_eq!(plan.outcome, expected, "gap {gap}");
    }
}
