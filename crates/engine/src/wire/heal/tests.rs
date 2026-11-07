use super::*;
use crate::{ATTACH_JUNCTION, DRAW_WIRE, HEAL_WIRES, MOVE_ITEMS, MOVE_SYMBOL, PLACE_SYMBOL, Session, persistence};
use serde_json::{Value, json};

mod detach;
mod redraws;
mod segment_slides;

/// Build a model point in the fixture's schematic grid units.
fn p(x: i64, y: i64) -> Point {
    Point { x_nm: x * 1_270_000, y_nm: y * 1_270_000 }
}
/// Execute a fixture edit against the session's current revision.
fn run(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}
/// Open an empty project for command-driven healing fixtures.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Healing"})).unwrap();
    s
}
/// Draw a straight free-ended fixture wire through the production command.
fn draw(s: &mut Session, from: Point, to: Point) {
    run(s, DRAW_WIRE, json!({"from":from,"to":to,"bends":[]}));
}

/// A moved part at the exact dangling-wire coordinate replaces the free identity with its pin;
/// a new two-pin part spanning a wire is inserted without shorting its two terminals.
#[test]
fn part_drops_connect_endpoints_and_insert_into_straight_wires() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(20,10).x_nm,"y_nm":p(20,10).y_nm}));
    let symbol = s.project().unwrap().symbols()[0].clone();
    let before = s.snapshot().unwrap();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol.id(),"x_nm":p(14,0).x_nm,"y_nm":0}));
    assert_eq!(s.project().unwrap().wires()[0].endpoints[1], WireEndpoint::Terminal(symbol.terminals()[0]));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));

    let mut s = session();
    draw(&mut s, p(0, 0), p(30, 0));
    let before = s.snapshot().unwrap();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(15,0).x_nm,"y_nm":0}));
    let project = s.project().unwrap();
    let pins = project.symbols()[0].terminals();
    assert_eq!(project.wires.len(), 2);
    assert_eq!(project.connectivity().nets.len(), 2);
    assert_eq!(project.wire_path(&project.wires[0]), vec![p(0, 0), p(11, 0)]);
    assert_eq!(project.wire_path(&project.wires[1]), vec![p(19, 0), p(30, 0)]);
    assert_ne!(project.connectivity().net_for(pins[0]), project.connectivity().net_for(pins[1]));
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().wires.len(), 2);
}

/// Whole-route moves attach their free ends to pins just as handle drops do, through the same
/// engine path used by GUI group previews and MCP execution.
#[test]
fn whole_wire_drop_onto_pin_connects() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(10, 0));
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,8).x_nm,"y_nm":p(14,8).y_nm}));
    let wire = s.project().unwrap().wires[0].id();
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    run(&mut s, MOVE_ITEMS, json!({"items":{"symbols":[],"wires":[wire]},"dx_nm":0,"dy_nm":p(0,8).y_nm}));
    assert_eq!(s.project().unwrap().wires[0].endpoints[1], WireEndpoint::Terminal(pin));
}

/// Explicit segment targets split at the branch; an ordinary perpendicular crossing adds no
/// connection. Preview, commit, undo and serialization retain the same graph.
#[test]
fn drawing_or_dropping_into_a_segment_creates_a_branch() {
    for attach in [false, true] {
        let mut s = session();
        draw(&mut s, p(0, 0), p(20, 0));
        let target = s.project().unwrap().wires[0].id();
        if attach {
            draw(&mut s, p(10, -10), p(10, -5));
        }
        let before = s.snapshot().unwrap();
        if attach {
            let source = before.wires[1].endpoints[1].inspect();
            run(&mut s, ATTACH_JUNCTION, json!({"junction":source["junction"],"to":{"wire":target,"at":p(10,0)}}));
        } else {
            run(&mut s, DRAW_WIRE, json!({"from":p(10,-10),"to":{"wire":target,"at":p(10,0)},"bends":[]}));
        }
        let project = s.project().unwrap();
        assert_eq!(project.wires.len(), 3);
        assert_eq!(project.connectivity().nets.len(), 1);
        let node = project.junctions().find(|(_, point)| *point == p(10, 0)).unwrap().0;
        assert_eq!(project.wires.iter().filter(|w| w.endpoints.contains(&WireEndpoint::Junction(node))).count(), 3);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    draw(&mut s, p(10, -10), p(10, 10));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), 2);
}

/// Partial positive overlaps become one run with no duplicate strokes or redundant intermediate
/// junctions. Repeated healing is a byte-stable no-op and adds no history entry.
#[test]
fn overlaps_and_doubled_back_legs_heal_locally() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    let before = s.snapshot().unwrap();
    draw(&mut s, p(10, 0), p(30, 0));
    let project = s.project().unwrap();
    assert_eq!(project.wires.len(), 1);
    assert_eq!(project.junctions().count(), 2);
    assert_eq!(project.wire_path(&project.wires[0]), vec![p(0, 0), p(30, 0)]);
    let wire = project.wires[0].id();
    let revision = s.revision();
    run(&mut s, HEAL_WIRES, json!({"items":{"symbols":[],"wires":[wire]}}));
    assert_eq!(s.revision(), revision);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let mut s = session();
    run(&mut s, DRAW_WIRE, json!({"from":p(0,0),"to":p(5,5),"bends":[p(10,0),p(5,0)]}));
    assert_eq!(s.project().unwrap().wire_path(&s.project().unwrap().wires[0]), vec![p(0, 0), p(5, 0), p(5, 5)]);
}

/// Finishing a drawn route on an existing wire must remove the U-shaped return to that
/// same run, without removing its free tail or changing the connection's identities.
#[test]
fn drawing_a_return_to_a_connected_run_straightens_the_branch() {
    let mut s = session();
    draw(&mut s, p(2, 12), p(14, 12));
    let target = s.project().unwrap().wires[0].id();
    let before = s.snapshot().unwrap();
    run(
        &mut s,
        DRAW_WIRE,
        json!({
            "from":p(-10,0),"to":{"wire":target,"at":p(8,12)},
            "bends":[p(6,0),p(6,13),p(8,13)]
        }),
    );
    let project = s.project().unwrap();
    let routes: Vec<_> = project.wires.iter().map(|w| project.wire_path(w)).collect();
    assert_eq!(routes, vec![vec![p(2, 12), p(6, 12)], vec![p(6, 12), p(14, 12)], vec![p(-10, 0), p(6, 0), p(6, 12)]]);
    assert_eq!(project.connectivity().nets.len(), 1);
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Return cleanup works in either route direction and orientation, but a geometrically
/// identical free-end crossing stays separate until an explicit connection is made.
#[test]
fn return_cleanup_requires_a_shared_junction_in_every_orientation() {
    for rotated in [false, true] {
        for reversed in [false, true] {
            for connected in [false, true] {
                let point = |x: i64, y: i64| if rotated { p(-y, x) } else { p(x, y) };
                let mut s = session();
                draw(&mut s, point(2, 12), point(14, 12));
                let target = s.project().unwrap().wires[0].id();
                let mut from = json!(point(-10, 0));
                let mut to = if connected { json!({"wire":target,"at":point(8,12)}) } else { json!(point(8, 12)) };
                let mut bends = vec![point(6, 0), point(6, 13), point(8, 13)];
                let mut expected = if connected {
                    vec![point(-10, 0), point(6, 0), point(6, 12)]
                } else {
                    vec![point(-10, 0), point(6, 0), point(6, 13), point(8, 13), point(8, 12)]
                };
                if reversed {
                    std::mem::swap(&mut from, &mut to);
                    bends.reverse();
                    expected.reverse();
                }
                run(&mut s, DRAW_WIRE, json!({"from":from,"to":to,"bends":bends}));
                let project = s.project().unwrap();
                assert_eq!(project.wire_path(project.wires.last().unwrap()), expected);
                assert_eq!(project.connectivity().nets.len(), if connected { 1 } else { 2 });
            }
        }
    }
}

/// A shortcut must never move a component pin or pull a junction past another branch on the
/// attached run. Keeping the authored return is preferable to changing those connections.
#[test]
fn return_cleanup_keeps_pins_and_intervening_branches_fixed() {
    for pin in [false, true] {
        let mut s = session();
        let target = if pin {
            run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(12,12).x_nm,"y_nm":p(12,12).y_nm}));
            let terminal = s.project().unwrap().symbols[0].terminals()[0].inspect();
            run(&mut s, DRAW_WIRE, json!({"from":p(2,12),"to":terminal,"bends":[]}));
            terminal
        } else {
            draw(&mut s, p(2, 12), p(14, 12));
            let wire = s.project().unwrap().wires[0].id();
            run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(7,12)},"to":p(7,8),"bends":[]}));
            let right = s.project().unwrap().wires[1].id();
            json!({"wire":right,"at":p(8,12)})
        };
        let before = s.snapshot().unwrap();
        run(&mut s, DRAW_WIRE, json!({"from":p(-10,0),"to":target,"bends":[p(6,0),p(6,13),p(8,13)]}));
        let project = s.project().unwrap();
        assert_eq!(project.wire_path(project.wires.last().unwrap()), vec![p(-10, 0), p(6, 0), p(6, 13), p(8, 13), p(8, 12)]);
        assert_eq!(project.symbols, before.symbols);
        for (id, point) in before.junctions() {
            assert_eq!(project.junctions.get(&id), Some(&point));
        }
        assert_eq!(project.connectivity().nets.len(), 1);
    }
}

/// An older saved return stays byte-identical on load. Explicit Heal from either incident
/// wire repairs it once, preserves every ID/edge and undoes without hidden normalization.
#[test]
fn manual_heal_repairs_saved_returns_without_changing_ids() {
    let mut s = session();
    draw(&mut s, p(2, 12), p(14, 12));
    let project = s.project().unwrap();
    let trunk = project.wires[0].id();
    // Build the pre-fix topology through the edit builder, deliberately omitting normalization.
    let mut edit = Edit::new(project, s.revision());
    let end = edit.resolve(Anchor::Split { wire: project.wires[0].ordinal, at: p(8, 12) }).unwrap();
    let from = edit.resolve(Anchor::Free(p(-10, 0))).unwrap();
    let ordinal = edit.id().unwrap();
    let wire = Wire { ordinal, endpoints: [from, end], bends: Some(vec![p(6, 0), p(6, 13), p(8, 13)]) };
    let branch = wire.id();
    Arc::make_mut(&mut edit.project.wires).push(wire);
    edit.project.connectivity = Arc::default();
    let saved = persistence::encode_documents(Arc::new(edit.project)).unwrap();
    for selected in [trunk, branch] {
        s.open_snapshot(persistence::decode_documents(&saved).unwrap());
        let before = s.snapshot().unwrap();
        assert_eq!(persistence::encode_documents(before.clone()).unwrap(), saved);
        let params = json!({"items":{"symbols":[],"wires":[selected]}});
        run(&mut s, HEAL_WIRES, params.clone());
        let healed = s.snapshot().unwrap();
        assert_eq!(healed.wire_path(healed.wires.last().unwrap()), vec![p(-10, 0), p(6, 0), p(6, 12)]);
        assert_eq!(
            healed.wires.iter().map(|w| (w.ordinal, w.endpoints)).collect::<Vec<_>>(),
            before.wires.iter().map(|w| (w.ordinal, w.endpoints)).collect::<Vec<_>>()
        );
        assert_eq!(healed.junctions.keys().collect::<Vec<_>>(), before.junctions.keys().collect::<Vec<_>>());
        let revision = s.revision();
        run(&mut s, HEAL_WIRES, params);
        assert_eq!(s.revision(), revision);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// A self-crossing route can be preserved or normalized by the same command. Manual Heal
/// works after reopening the preserved shape and its cleanup remains one undoable edit.
#[test]
fn self_crossing_routes_obey_optional_healing() {
    for auto_heal in [false, true] {
        let mut s = session();
        let bends = vec![p(6, 0), p(6, 12), p(-4, 12), p(-4, 6), p(6, 6), p(6, 12)];
        run(&mut s, DRAW_WIRE, json!({"from":p(-10,0),"to":p(20,12),"bends":bends,"auto_heal":auto_heal}));
        let simple = vec![p(-10, 0), p(6, 0), p(6, 12), p(20, 12)];
        let expected = if auto_heal { simple.clone() } else { std::iter::once(p(-10, 0)).chain(bends).chain([p(20, 12)]).collect() };
        let project = s.project().unwrap();
        assert_eq!(project.wire_path(&project.wires[0]), expected);
        let wire = project.wires[0].id();
        let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        s.open_snapshot(persistence::decode_documents(&saved).unwrap());
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
        let before = s.snapshot().unwrap();
        run(&mut s, HEAL_WIRES, json!({"items":{"symbols":[],"wires":[wire]}}));
        assert_eq!(s.project().unwrap().wire_path(&s.project().unwrap().wires[0]), simple);
        if !auto_heal {
            s.apply("edit.undo", &Value::Null).unwrap();
            assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        }
    }
}

/// A shortcut drawn between two points of one existing route removes its longer alternative
/// with Auto-heal on; off preserves a serializable loop that explicit Heal can later remove.
#[test]
fn shortcuts_back_onto_the_same_wire_obey_optional_healing() {
    for auto_heal in [false, true] {
        let mut s = session();
        run(&mut s, DRAW_WIRE, json!({"from":p(-10,0),"to":p(20,12),"bends":[p(6,0),p(6,6),p(-4,6),p(-4,12)]}));
        let wire = s.project().unwrap().wires[0].id();
        let before = s.snapshot().unwrap();
        run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(6,6)},"to":{"wire":wire,"at":p(6,12)},"bends":[],"auto_heal":auto_heal}));
        let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
        assert_eq!(s.project().unwrap().wires.len(), if auto_heal { 2 } else { 5 });
        assert_eq!(s.project().unwrap().connectivity().nets.len(), 1);
        if !auto_heal {
            let shortcut = s.project().unwrap().wires.last().unwrap().id();
            let looped = s.snapshot().unwrap();
            run(&mut s, HEAL_WIRES, json!({"items":{"symbols":[],"wires":[shortcut]}}));
            assert_eq!(s.project().unwrap().wires.len(), 2);
            s.apply("edit.undo", &Value::Null).unwrap();
            assert!(Arc::ptr_eq(&looped, &s.snapshot().unwrap()));
        }
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// A branch on the longer side of a loop prevents deleting that route: cleanup must not
/// silently disconnect it or move the other junctions to produce a simpler drawing.
#[test]
fn loop_cleanup_preserves_intermediate_branches() {
    let mut s = session();
    run(&mut s, DRAW_WIRE, json!({"from":p(-10,0),"to":p(20,12),"bends":[p(6,0),p(6,6),p(-4,6),p(-4,12)]}));
    let wire = s.project().unwrap().wires[0].id();
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(-4,6)},"to":p(-4,2),"bends":[]}));
    let project = s.project().unwrap();
    let right = project.wires[1].id();
    let junction = project.junctions().find(|(_, point)| *point == p(-4, 6)).unwrap().0;
    run(&mut s, DRAW_WIRE, json!({"from":{"wire":wire,"at":p(6,6)},"to":{"wire":right,"at":p(6,12)},"bends":[]}));
    let project = s.project().unwrap();
    assert_eq!(project.junctions.get(&junction), Some(&p(-4, 6)));
    assert_eq!(project.wires.iter().filter(|w| w.endpoints.contains(&WireEndpoint::Junction(junction))).count(), 3);
    assert_eq!(project.connectivity().nets.len(), 1);
    assert!(project.wires.iter().any(|w| project.wire_path(w).contains(&p(-4, 12))));
}

/// Healing is bounded and atomic, and it never silently rewrites a project merely on reopening.
#[test]
fn healing_rejects_stale_or_ambiguous_requests_without_changes() {
    let mut s = session();
    draw(&mut s, p(0, 0), p(20, 0));
    draw(&mut s, p(10, -10), p(10, 10));
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    assert!(s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":p(14,0).x_nm,"y_nm":0,"expected_revision":revision})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(s.apply(HEAL_WIRES, &json!({"items":{"symbols":[],"wires":[before.wires[0].id()]},"expected_revision":revision-1})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let mut edit = Edit::new(&before, revision);
    edit.budget = 0;
    edit.touched.insert(before.wires[0].ordinal);
    assert!(edit.heal().is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}
/// Preview and committed-route normalization shorten partial and complete retraces in either
/// axis and direction, without drifting endpoints. Normalizing twice yields the same route.
#[test]
fn draft_retraces_normalize_in_every_orientation() {
    for swap in [false, true] {
        for sign in [-1, 1] {
            let transform = |(x, y): (i64, i64)| if swap { p(sign * y, x) } else { p(x, sign * y) };
            for (input, output) in [
                (vec![(0, 0), (0, 10), (0, 3), (6, 3)], vec![(0, 0), (0, 3), (6, 3)]),
                (vec![(0, 0), (0, 10), (0, -5), (6, -5)], vec![(0, 0), (0, -5), (6, -5)]),
                (vec![(0, 0), (0, 10), (0, 0), (6, 0)], vec![(0, 0), (6, 0)]),
                (vec![(0, 0), (0, 10), (0, 3), (0, 8), (0, 2), (6, 2)], vec![(0, 0), (0, 2), (6, 2)]),
            ] {
                let path: Vec<_> = input.into_iter().map(transform).collect();
                let expected: Vec<_> = output.into_iter().map(transform).collect();
                let normalized = normalized_path(&path).unwrap();
                assert_eq!(normalized, expected);
                assert_eq!(normalized.first(), path.first());
                assert_eq!(normalized.last(), path.last());
                assert_eq!(normalized_path(&normalized).unwrap(), normalized);
            }
        }
    }
}

/// Invalid preview geometry is rejected before cleanup; a bounded draft cannot silently
/// turn diagonal or out-of-range input into a different route.
#[test]
fn invalid_draft_geometry_is_refused() {
    for path in [Vec::new(), vec![p(0, 0), p(1, 1)], vec![p(0, 0), Point { x_nm: i64::MAX, y_nm: 0 }], vec![p(0, 0); 2 * (MAX_BENDS + 2) + 1]] {
        assert!(normalized_path(&path).is_err());
    }
}
