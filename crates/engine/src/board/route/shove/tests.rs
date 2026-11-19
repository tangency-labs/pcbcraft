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
        "points":points,"replacements":plan.replacements,"merge_same_net":request.merge_same_net,"expected_revision":rev}),
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
            let incoming = Contour::segment(request.target, request.target, request.width_nm).unwrap();
            let upper = original.objects().iter().find(|o| o.name == a.to_string()).unwrap();
            assert!(incoming.gap(&upper.contour) >= 200_000., "incoming route itself must not hit the upper rail");
            let plan = s.plan_route(&original, &request, DEFAULT_BUDGET, &|| false).unwrap();
            assert_eq!(plan.outcome, Outcome::Reached, "{plan:?}");
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
                {"track":upper,"points":[Point{x_nm:-975_000,y_nm:5_000_000},Point{x_nm:-480_576,y_nm:5_000_000},Point{x_nm:7_703_428,y_nm:-3_184_004},Point{x_nm:8_829_172,y_nm:-3_184_004},Point{x_nm:17_000_000,y_nm:4_986_824},Point{x_nm:17_000_000,y_nm:5_000_000},Point{x_nm:19_025_000,y_nm:5_000_000}]},
                {"track":lower,"points":[Point{x_nm:0,y_nm:5_800_000},Point{x_nm:0,y_nm:5_332_600},Point{x_nm:7_941_602,y_nm:-2_609_002},Point{x_nm:8_577_822,y_nm:-2_609_002},Point{x_nm:16_986_824,y_nm:5_800_000},Point{x_nm:17_000_000,y_nm:5_800_000},Point{x_nm:20_000_000,y_nm:5_800_000}]}
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
                let exhausted = s.plan_route(&query(&s), &request, plan.probes - 1, &|| false).unwrap();
                assert_eq!(exhausted.outcome, Outcome::Incomplete);
                assert_eq!(exhausted.shove.as_ref().unwrap().attempts, 2);
                assert!(exhausted.probes < plan.probes);
                assert_eq!(exhausted.points, vec![request.head]);
                assert!(exhausted.replacements.is_empty());
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
