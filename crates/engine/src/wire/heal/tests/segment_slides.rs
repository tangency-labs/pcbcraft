use super::*;
use crate::{MOVE_WIRE_SEGMENT, SLICE_WIRE};

/// Reproduce the photographed stepped route, split where its upper horizontal run meets
/// the vertical segment being dragged. The split is a transit node, not a free endpoint.
fn stepped_route() -> (Session, String, [WireEndpoint; 2]) {
    let mut s = session();
    run(
        &mut s,
        DRAW_WIRE,
        json!({"from":p(16,0),"to":p(12,18),
        "bends":[p(16,6),p(0,6),p(0,10),p(12,10)]}),
    );
    let ends = s.project().unwrap().wires[0].endpoints;
    let wire = s.project().unwrap().wires[0].id();
    run(&mut s, SLICE_WIRE, json!({"wire":wire,"at":p(0,6)}));
    let dragged = s.project().unwrap().wires[1].id();
    (s, dragged, ends)
}

/// Sliding a split route right must consume its doubled-back transit leg, rather than
/// leaving the old corner as a new dangling stub. Preview and commit must agree.
#[test]
fn sliding_a_split_route_removes_the_old_corner_stub() {
    let (mut s, wire, ends) = stepped_route();
    let before = s.snapshot().unwrap();
    let edit = super::super::super::Reshape::Segment { wire: wire.clone(), at: p(0, 8) };
    let preview = before.reshaped(&edit, p(16, 8)).unwrap().healed(&before, s.revision(), true).unwrap();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,8),"to":p(16,8)}));
    let project = s.project().unwrap();
    assert!(!project.junctions.values().any(|&point| point == p(0, 6)), "the old transit corner must not become a free horizontal stub");
    assert_eq!(project.wires.len(), 1);
    assert_eq!(project.wires[0].endpoints, ends);
    assert_eq!(project.wire_path(&project.wires[0]), vec![p(16, 0), p(16, 10), p(12, 10), p(12, 18)]);
    assert_eq!(preview.wires, project.wires);
    assert_eq!(preview.junctions, project.junctions);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
}

/// Walk every edge of a branch-free fixture from its original start to its original end.
/// Extra stubs, disconnected remnants or hidden branches fail instead of being normalized away.
fn chain_path(project: &Project, ends: [WireEndpoint; 2]) -> Vec<Point> {
    let mut at = ends[0];
    let mut seen = BTreeSet::new();
    let mut path = Vec::new();
    while at != ends[1] {
        let edges: Vec<_> = project.wires.iter().filter(|wire| wire.endpoints.contains(&at) && !seen.contains(&wire.ordinal)).collect();
        assert_eq!(edges.len(), 1, "a transit corner must not acquire a new branch");
        let wire = edges[0];
        let mut route = project.wire_path(wire);
        let forward = wire.endpoints[0] == at;
        if !forward {
            route.reverse();
        }
        path.extend(route.into_iter().skip(usize::from(!path.is_empty())));
        seen.insert(wire.ordinal);
        at = wire.endpoints[usize::from(forward)];
    }
    assert_eq!(seen.len(), project.wires.len(), "no old corner may remain as a disconnected tail");
    normalized_path(&path).unwrap()
}

/// Identical visible routes must slide identically whether authored as one wire or split at
/// either/both moved corners. Exercise the slide before, on and beyond both adjacent runs,
/// with reversed endpoint order and both axes/directions.
#[test]
fn sliding_steps_is_independent_of_corner_partitions_and_orientation() {
    for swapped in [false, true] {
        for reflected in [false, true] {
            for reversed in [false, true] {
                for cuts in 0..=2 {
                    for x in [4, 12, 16, 20] {
                        let point = |x: i64, y: i64| {
                            let x = if reflected { -x } else { x };
                            if swapped { p(y, x) } else { p(x, y) }
                        };
                        let mut s = session();
                        let mut from = point(16, 0);
                        let mut to = point(12, 18);
                        let mut bends = vec![point(16, 6), point(0, 6), point(0, 10), point(12, 10)];
                        if reversed {
                            std::mem::swap(&mut from, &mut to);
                            bends.reverse();
                        }
                        run(&mut s, DRAW_WIRE, json!({"from":from,"to":to,"bends":bends}));
                        let ends = s.project().unwrap().wires[0].endpoints;
                        for at in [point(0, 6), point(0, 10)].into_iter().take(cuts) {
                            let project = s.project().unwrap();
                            let wire = project
                                .wires
                                .iter()
                                .find(|wire| {
                                    let path = project.wire_path(wire);
                                    path.contains(&at) && path.first() != Some(&at) && path.last() != Some(&at)
                                })
                                .unwrap()
                                .id();
                            run(&mut s, SLICE_WIRE, json!({"wire":wire,"at":at}));
                        }
                        let project = s.project().unwrap();
                        let wire = project
                            .wires
                            .iter()
                            .find(|wire| project.wire_path(wire).windows(2).any(|leg| on_segment(point(0, 8), leg[0], leg[1])))
                            .unwrap()
                            .id();
                        run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":point(0,8),"to":point(x,8)}));
                        let mut expected = vec![point(16, 0), point(16, 6), point(x, 6), point(x, 10), point(12, 10), point(12, 18)];
                        if reversed {
                            expected.reverse();
                        }
                        assert_eq!(
                            chain_path(s.project().unwrap(), ends),
                            normalized_path(&expected).unwrap(),
                            "swapped={swapped}, reflected={reflected}, reversed={reversed}, cuts={cuts}, x={x}"
                        );
                    }
                }
            }
        }
    }
}

/// A real branch at the old corner makes its horizontal leg necessary. The branch's free
/// endpoint remains in place and connected when the neighbouring segment slides away.
#[test]
fn sliding_preserves_a_real_branch_at_the_folded_corner() {
    let (mut s, wire, ends) = stepped_route();
    let junction = s.project().unwrap().junctions().find(|(_, point)| *point == p(0, 6)).unwrap().0;
    run(&mut s, DRAW_WIRE, json!({"from":WireEndpoint::Junction(junction).inspect(),"to":p(-8,6),"bends":[]}));
    let branch = s.project().unwrap().junctions().find(|(_, point)| *point == p(-8, 6)).unwrap().0;
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,8),"to":p(16,8)}));
    let project = s.project().unwrap();
    assert_eq!(project.endpoint_position(WireEndpoint::Junction(branch)), Some(p(-8, 6)));
    assert!(project.wires.iter().any(|wire| wire.endpoints.contains(&WireEndpoint::Junction(branch))));
    for end in ends {
        assert!(project.wires.iter().any(|wire| wire.endpoints.contains(&end)));
    }
    assert_eq!(project.connectivity().nets.len(), 1);
}

/// A pin at the old corner is an electrical anchor even when just two wires meet there.
/// Cleanup retains its necessary horizontal connection instead of trimming it as a stub.
#[test]
fn sliding_preserves_a_pin_at_the_folded_corner() {
    let (mut s, _, _) = stepped_route();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(0,2).x_nm,"y_nm":p(0,2).y_nm,"rotation":90}));
    let project = s.project().unwrap();
    let pin = project.symbols[0].terminals().into_iter().find(|&pin| project.terminal_position(pin) == Some(p(0, 6))).unwrap();
    let wire = project.wires.iter().find(|wire| project.wire_path(wire).windows(2).any(|leg| on_segment(p(0, 8), leg[0], leg[1]))).unwrap().id();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,8),"to":p(16,8)}));
    let project = s.project().unwrap();
    assert_eq!(project.terminal_position(pin), Some(p(0, 6)));
    assert!(project.wires.iter().any(|wire| wire.endpoints.contains(&WireEndpoint::Terminal(pin))));
    assert!(project.connectivity().net_for(pin).is_some());
}

/// Cleanup-off preserves the authored overlapping route. Explicit Heal can subsequently
/// consume the folded transit corner without adding another move or losing undo fidelity.
#[test]
fn folded_corner_cleanup_respects_the_automatic_healing_preference() {
    let (mut s, wire, ends) = stepped_route();
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,8),"to":p(16,8),"auto_heal":false}));
    let raw = s.snapshot().unwrap();
    assert!(raw.junctions.values().any(|&point| point == p(0, 6)));
    run(&mut s, HEAL_WIRES, json!({"items":{"symbols":[],"wires":[wire]}}));
    assert_eq!(chain_path(s.project().unwrap(), ends), vec![p(16, 0), p(16, 10), p(12, 10), p(12, 18)]);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&raw, &s.snapshot().unwrap());
}

/// An intentionally free endpoint keeps its protection when overlap merging substitutes an
/// older transit identity at the same position. Its remaining connection must not be pruned.
#[test]
fn merging_a_free_endpoint_into_an_old_transit_corner_keeps_its_tail() {
    let (mut s, wire, _) = stepped_route();
    run(&mut s, DRAW_WIRE, json!({"from":p(0,6),"to":p(4,6),"bends":[],"auto_heal":false}));
    run(&mut s, MOVE_WIRE_SEGMENT, json!({"wire":wire,"at":p(0,8),"to":p(16,8)}));
    let project = s.project().unwrap();
    let end = project.junctions().find(|(_, point)| *point == p(0, 6)).unwrap().0;
    assert_eq!(project.wires.iter().filter(|wire| wire.endpoints.contains(&WireEndpoint::Junction(end))).count(), 1);
    assert_eq!(project.connectivity().nets.len(), 1);
}
