use super::{
    super::tests::{copper_layers, route, routed_fixture, view},
    *,
};
use crate::{
    SET_FOOTPRINT_TRANSFORM,
    board::{
        ratsnest::TERMINALS_BUILT,
        update::{NetState, TerminalKey},
        via_nets::SET_VIA_NET,
    },
    persistence::{decode_documents, encode_documents},
};
use std::{cell::Cell, sync::Arc};

struct Tee {
    session: Session,
    args: Value,
    front: Uuid,
    tip: Point,
    junction: Point,
    trunk: Track,
    arm: Uuid,
    orphan: Uuid,
    standalone: Uuid,
    right: Value,
    net: usize,
}

/// Author a T whose branch meets the middle of one unsplit trunk segment, with a via on
/// the branch and a separate standalone via. No project document is hand edited.
fn tee() -> Tee {
    let mut session = routed_fixture();
    let ratsnest = view(&session).ratsnest().unwrap();
    let left = ratsnest.lines[0].from.clone();
    let middle = ratsnest.lines[0].to.clone();
    let right = ratsnest.lines[1].to.clone();
    let net = ratsnest.lines[0].net;
    let board = view(&session).target.board;
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":board,"footprint":middle.footprint,
        "x_nm":middle.position.x_nm,"y_nm":middle.position.y_nm + 4_000_000,"rotation":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    // Face the routed pads inward so the straight trunk doesn't cross a neighboring land.
    let right_center = view(&session).footprints().find(|f| f.id == right.footprint).unwrap().position;
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":board,"footprint":right.footprint,
        "x_nm":right_center.x_nm,"y_nm":right_center.y_nm,"rotation":180,"expected_revision":session.revision()}),
        )
        .unwrap();
    let tip = session.project().unwrap().board.pad_position(middle.pad).unwrap();
    let right_end = session.project().unwrap().board.pad_position(right.pad).unwrap();
    let junction = Point { x_nm: tip.x_nm, y_nm: left.position.y_nm };
    let front = copper_layers(&session)[0];
    route(&mut session, front, &[left.position, right_end]).unwrap();
    let trunk = view(&session).tracks()[0].clone();
    route(&mut session, front, &[tip, junction]).unwrap();
    let arm = view(&session).tracks().iter().find(|track| track.id != trunk.id).unwrap().id;
    let branch_via = Point { x_nm: tip.x_nm, y_nm: tip.y_nm - 2_000_000 };
    session.apply(ADD_VIA, &json!({"board":board,"at":branch_via,"expected_revision":session.revision()})).unwrap();
    let orphan = view(&session).vias()[0].id;
    let isolated = Point { x_nm: tip.x_nm, y_nm: tip.y_nm + 10_000_000 };
    session.apply(ADD_VIA, &json!({"board":board,"at":isolated,"expected_revision":session.revision()})).unwrap();
    let standalone = view(&session).vias().iter().find(|v| v.id != orphan).unwrap().id;
    let args = json!({"board":board,"from":address(middle.pad),"to":address(left.pad),"expected_revision":session.revision()});
    Tee { session, args, front, tip, junction, trunk, arm, orphan, standalone, right: address(right.pad), net }
}

/// Command arguments use board-local pad addresses.
fn address(pad: crate::board::Pad) -> Value {
    let mut value = pad.inspect();
    value.as_object_mut().unwrap().remove("board");
    value
}

/// Preview and apply agree on the removed IDs; an interior contact doesn't require splitting
/// or rebuilding the shared trunk. One airwire returns, and Undo/Redo and save/reopen agree.
#[test]
fn unroute_tee_preserves_shared_trunk() {
    let Tee { mut session, mut args, mut trunk, arm, orphan, standalone, net, .. } = tee();
    session
        .apply(SET_COPPER_LOCKED, &json!({"board":args["board"],"tracks":[trunk.id],"locked":true,"expected_revision":session.revision()}))
        .unwrap();
    trunk.locked = true;
    assert!(view(&session).ratsnest().unwrap().lines.iter().all(|line| line.net != net));
    let revision = session.revision();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    args["expected_revision"] = json!(revision);
    args["dry_run"] = json!(true);
    let preview = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(preview["removed"], json!({"tracks":[arm],"vias":[orphan]}));
    assert_eq!(preview["requested"], json!({"from":args["from"],"to":args["to"]}));
    assert_eq!(preview["unrouted"], json!([{"from":args["to"],"to":args["from"]}]));
    assert_eq!(preview["revision"], revision);
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    args["dry_run"] = json!(false);
    let applied = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(applied["removed"], preview["removed"]);
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(view(&session).tracks(), &[trunk]);
    assert_eq!(view(&session).vias().iter().map(|via| via.id).collect::<Vec<_>>(), vec![standalone]);
    let ratsnest = view(&session).ratsnest().unwrap();
    assert_eq!(ratsnest.lines.iter().filter(|line| line.net == net).count(), 1);
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&after).unwrap();
    assert_eq!(reopened.board_view(reopened.board_target().unwrap()).unwrap().ratsnest(), Ok(ratsnest));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
    args["expected_revision"] = json!(session.revision());
    refuses_without_edit(&mut session, &args, "not joined by routed copper");
    session.apply("edit.undo", &Value::Null).unwrap();
    args["expected_revision"] = json!(session.revision());
    session.apply(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
}

/// Tiny budgets make monotonic progress without exposing a partial edit. Dropping an apply
/// midway is inert; changing batch size leaves the preview and real display delta identical.
#[test]
fn unroute_preparation_yields_and_preserves_report() {
    let Tee { mut session, mut args, arm, orphan, .. } = tee();
    let revision = session.revision();
    let before = session.snapshot().unwrap();
    let mut abandoned = session.prepare_copper_edit(UNROUTE_CONNECTION, &args).unwrap();
    let initial = abandoned.progress();
    assert!(!abandoned.step(0));
    assert_eq!(abandoned.progress(), initial);
    for completed in 1..=16 {
        assert!(!abandoned.step(1), "unroute finished before visiting the contacts");
        assert_eq!(abandoned.progress().completed, completed);
        assert!(abandoned.take_result().unwrap().is_none());
    }
    drop(abandoned);
    assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);

    args["dry_run"] = json!(true);
    let oracle = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(oracle["removed"], json!({"tracks":[arm],"vias":[orphan]}));
    assert_eq!(oracle["unrouted"], json!([{"from":args["to"],"to":args["from"]}]));
    for budget in [1, 7] {
        let mut job = session.prepare_copper_edit(UNROUTE_CONNECTION, &args).unwrap();
        let mut pending = 0;
        loop {
            let previous = job.progress().completed;
            if job.step(budget) {
                break;
            }
            assert_eq!(job.progress().completed, previous + budget);
            pending += 1;
            assert!(pending < 10_000, "tiny requests must not starve the planner");
            assert!(job.take_result().unwrap().is_none());
        }
        assert!(pending > 1);
        let prepared = job.take_result().unwrap().unwrap();
        let (unchanged, report) = session.commit_copper_edit_with_report(prepared).unwrap();
        assert_eq!(unchanged, revision);
        assert_eq!(report, Some(oracle.clone()));
        assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}

/// Sparse overlapping envelopes prune globally, including across the former block boundary.
/// Poll once per charged index visit and compare source-ordered hits to a full scan.
#[test]
fn sparse_unroute_index_prunes_at_budget_one() {
    const COUNT: usize = 16_384;
    let bounds: Vec<_> = (0..COUNT)
        .map(|id| {
            let x = ((COUNT - id) * 10) as f64;
            [x, -1., x + 10., 1.]
        })
        .collect();
    let mut cx = Context::from_waker(Waker::noop());
    let mut build = std::pin::pin!(Index::new(&bounds));
    let index = loop {
        if let Poll::Ready(index) = build.as_mut().poll(&mut cx) {
            break index;
        }
    };
    for id in [0, 127, 128, COUNT / 2, COUNT - 1] {
        let query = bounds[id];
        let expected: Vec<_> = bounds.iter().enumerate().filter_map(|(other, b)| (query[0] <= b[2] && b[0] <= query[2]).then_some(other)).collect();
        let mut search = std::pin::pin!(index.query(query));
        let mut visits = 0;
        let found = loop {
            match search.as_mut().poll(&mut cx) {
                Poll::Pending => {
                    visits += 1;
                    assert!(visits < 64, "sparse query {id} failed to prune the global hierarchy");
                }
                Poll::Ready(found) => break found,
            }
        };
        assert!(visits > 1, "the sparse search must be resumable");
        assert_eq!(found.into_iter().collect::<Vec<_>>(), expected);
    }
}

/// Overlapping end caps let the shortest walk skip the pad-side segment. Remove both UUIDs,
/// preserve the trunk, and report the left-to-middle edge even though `to` is the right pad.
#[test]
fn segmented_arm_reports_actual_display_delta() {
    let Tee { mut session, mut args, front, tip, junction, trunk, arm, right, .. } = tee();
    session.apply(DELETE_TRACKS, &json!({"tracks":[arm],"expected_revision":session.revision()})).unwrap();
    let retained_left = std::mem::replace(&mut args["to"], right);
    let split = Point { x_nm: tip.x_nm, y_nm: tip.y_nm - 400_000 };
    route(&mut session, front, &[tip, split, junction]).unwrap();
    let removed: Vec<_> = view(&session).tracks().iter().filter(|t| t.id != trunk.id).map(|t| t.id).collect();
    assert_eq!(removed.len(), 2);
    args["expected_revision"] = json!(session.revision());
    args["dry_run"] = json!(true);
    let preview = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(preview["removed"]["tracks"], json!(removed));
    assert_eq!(preview["requested"]["to"], args["to"]);
    assert_eq!(preview["unrouted"], json!([{"from":retained_left,"to":args["from"]}]));
    args["dry_run"] = json!(false);
    let committed = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(committed["unrouted"], preview["unrouted"]);
    assert_eq!(view(&session).tracks(), &[trunk]);
}

/// The first and last track end caps touch across the short middle segment. The cut must
/// include that now-padless segment, but leave the unrelated floating track and via alone.
#[test]
fn segmented_arm_removes_detached_interior_piece() {
    for locked in [false, true] {
        let Tee { mut session, mut args, front, tip, junction, trunk, arm, standalone, .. } = tee();
        session.apply(DELETE_TRACKS, &json!({"tracks":[arm],"expected_revision":session.revision()})).unwrap();
        route(&mut session, front, &[Point { x_nm: -30_000_000, y_nm: -20_000_000 }, Point { x_nm: -29_000_000, y_nm: -20_000_000 }]).unwrap();
        let unrelated = view(&session).tracks().iter().find(|track| track.id != trunk.id).unwrap().clone();
        let a = Point { x_nm: tip.x_nm, y_nm: tip.y_nm - 1_000_000 };
        let b = Point { x_nm: tip.x_nm, y_nm: tip.y_nm - 1_100_000 };
        route(&mut session, front, &[tip, a, b, junction]).unwrap();
        let removed: Vec<_> = view(&session).tracks().iter().filter(|t| t.id != trunk.id && t.id != unrelated.id).map(|t| t.id).collect();
        assert_eq!(removed.len(), 3);
        if locked {
            let middle = view(&session).tracks().iter().find(|t| t.start == a && t.end == b).unwrap().id;
            session
                .apply(SET_COPPER_LOCKED, &json!({"board":args["board"],"tracks":[middle],"locked":true,"expected_revision":session.revision()}))
                .unwrap();
        }
        args["expected_revision"] = json!(session.revision());
        let before = session.snapshot().unwrap();
        for dry_run in [true, false] {
            args["dry_run"] = json!(dry_run);
            if locked {
                refuses_without_edit(&mut session, &args, "locked");
            } else {
                let report = session.execute(UNROUTE_CONNECTION, &args).unwrap();
                assert_eq!(report["removed"]["tracks"], json!(removed));
            }
            if locked || dry_run {
                assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
            }
        }
        if !locked {
            let mut retained = vec![trunk, unrelated];
            retained.sort_unstable_by_key(|track| track.id);
            assert_eq!(view(&session).tracks(), retained);
            assert_eq!(view(&session).vias().iter().map(|via| via.id).collect::<Vec<_>>(), vec![standalone]);
        }
    }
}

/// Two mutually touching automatic vias are removed together unless a retained via anchors
/// them. The via that was already standalone never enters the affected component.
#[test]
fn newly_floating_via_group_respects_anchors() {
    for locked in [false, true] {
        let Tee { mut session, mut args, tip, orphan, standalone, .. } = tee();
        let at = Point { x_nm: tip.x_nm + 500_000, y_nm: tip.y_nm - 2_000_000 };
        session.apply(ADD_VIA, &json!({"at":at,"expected_revision":session.revision()})).unwrap();
        let outer = view(&session).vias().iter().find(|v| v.at == at).unwrap().id;
        if locked {
            session
                .apply(SET_COPPER_LOCKED, &json!({"board":args["board"],"vias":[outer],"locked":true,"expected_revision":session.revision()}))
                .unwrap();
        }
        let mut affected = vec![orphan, outer];
        affected.sort_unstable();
        args["expected_revision"] = json!(session.revision());
        let report = session.execute(UNROUTE_CONNECTION, &args).unwrap();
        assert_eq!(report["removed"]["vias"], if locked { json!([]) } else { json!(affected) });
        let mut remaining = vec![standalone];
        if locked {
            remaining.extend(affected);
            remaining.sort_unstable();
        }
        assert_eq!(view(&session).vias().iter().map(|v| v.id).collect::<Vec<_>>(), remaining);
    }
}

/// Check both paths for atomicity when deletion conflicts with the existing copper lock.
#[test]
fn locked_arm_refuses_preview_and_apply() {
    let Tee { mut session, mut args, arm, .. } = tee();
    session.apply(SET_COPPER_LOCKED, &json!({"board":args["board"],"tracks":[arm],"locked":true,"expected_revision":session.revision()})).unwrap();
    args["expected_revision"] = json!(session.revision());
    for dry_run in [true, false] {
        args["dry_run"] = json!(dry_run);
        refuses_without_edit(&mut session, &args, "locked");
    }
}

/// Malformed, stale, wrong-net and ambiguous requests must fail before publishing any cut.
#[test]
fn invalid_endpoints_and_nonexclusive_arms_do_not_edit() {
    let Tee { mut session, args, front, tip, junction, .. } = tee();
    let mut bad = args.clone();
    bad["expected_revision"] = json!(session.revision() - 1);
    refuses_without_edit(&mut session, &bad, "revision");
    bad = args.clone();
    bad["from"]["pad"] = json!("999");
    refuses_without_edit(&mut session, &bad, "pads with copper");
    bad["from"]["pad"] = json!("1");
    refuses_without_edit(&mut session, &bad, "same intended net");
    bad["from"] = args["to"].clone();
    refuses_without_edit(&mut session, &bad, "distinct pads");
    bad["from"]["unexpected"] = json!(true);
    refuses_without_edit(&mut session, &bad, "footprint ID and a pad number");
    bad = args.clone();
    bad["board"] = json!(Uuid::nil());
    refuses_without_edit(&mut session, &bad, "board");
    bad = args.clone();
    bad["from"] = args["to"].clone();
    bad["to"] = args["from"].clone();
    refuses_without_edit(&mut session, &bad, "No exclusive routed arm");
    // Add an independent path from the first pad to the trunk. A cut confined to one
    // shortest path cannot make it unrouted, so keep both routes.
    let split = Point { x_nm: tip.x_nm, y_nm: tip.y_nm - 650_000 };
    let offset = Point { x_nm: tip.x_nm - 2_000_000, y_nm: split.y_nm };
    // A single parallel diagonal would pass too near the other pad of a 0402. Depart
    // vertically until clear of both lands, then continue to a separate trunk contact.
    route(&mut session, front, &[tip, split, offset, Point { x_nm: offset.x_nm, y_nm: junction.y_nm }]).unwrap();
    let mut loop_args = args.clone();
    loop_args["expected_revision"] = json!(session.revision());
    refuses_without_edit(&mut session, &loop_args, "without cutting another connection");
}

/// On a large board, no planning step copies or checks more than one candidate item or
/// resolves more than one display terminal. A large unrelated accepted net adds no display
/// work, and applying the cut leaves the same board as the shared deletion command.
#[test]
fn large_board_candidate_and_delta_work_is_bounded_per_step() {
    let Tee { mut session, mut args, front, trunk, arm, orphan, net, .. } = tee();
    let mut project = session.snapshot().unwrap().as_ref().clone();
    let board = Arc::make_mut(&mut project.board);
    for i in 0..1024_i64 {
        let start = Point { x_nm: -80_000_000 + (i % 32) * 400_000, y_nm: -80_000_000 + (i / 32) * 400_000 };
        let end = Point { x_nm: start.x_nm + 100_000, y_nm: start.y_nm };
        board.tracks.push(Track { id: Uuid::from_u128(i as u128 + 1), locked: false, layer: front, width_nm: trunk.width_nm, start, end });
    }
    board.tracks.sort_by_key(|track| track.id);
    let baseline = Arc::make_mut(board.baseline.as_mut().unwrap());
    let terminals = baseline.nets[net].terminals.len();
    let unrelated = (0..4096).map(|i| TerminalKey { component: u64::MAX - i, terminal: "1".into() }).collect();
    baseline.nets.insert(0, NetState { display_name: None, terminals: unrelated, names: vec!["UNRELATED".into()], origin: None });
    let items = board.tracks.len() + board.vias.len() + board.footprints().len();
    session.open_snapshot(Arc::new(project));
    args["expected_revision"] = json!(session.revision());
    args["dry_run"] = json!(true);

    let counts = || (CANDIDATE_ITEMS.with(Cell::get), TERMINALS_BUILT.with(Cell::get));
    CANDIDATE_ITEMS.with(|count| count.set(0));
    TERMINALS_BUILT.with(|count| count.set(0));
    let mut job = session.prepare_copper_edit(UNROUTE_CONNECTION, &args).unwrap();
    loop {
        let before = counts();
        let done = job.step(1);
        let after = counts();
        assert!(after.0 - before.0 <= 1, "one step copied or checked several candidate items");
        assert!(after.1 - before.1 <= 1, "one step resolved several display terminals");
        if done {
            break;
        }
    }
    let (candidate, built) = counts();
    assert!(candidate >= items, "candidate construction must visit every board item cooperatively: {candidate} < {items}");
    assert_eq!(built, 2 * terminals, "only the net containing the detached pad is built, once before and once after");
    let (_, preview) = session.commit_copper_edit_with_report(job.take_result().unwrap().unwrap()).unwrap();
    let preview = preview.unwrap();
    assert_eq!(preview["removed"], json!({"tracks":[arm],"vias":[orphan]}));
    assert_eq!(preview["unrouted"], json!([{"from":args["to"],"to":args["from"]}]));

    args["dry_run"] = json!(false);
    let applied = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(applied["removed"], preview["removed"]);
    assert_eq!(applied["unrouted"], preview["unrouted"]);
    let unrouted = session.snapshot().unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    session.apply(DELETE_TRACKS, &json!({"tracks":[arm],"vias":[orphan],"expected_revision":session.revision()})).unwrap();
    assert!(*session.snapshot().unwrap().board == *unrouted.board);
}

/// Overlapping pads that both stay on the retained side don't block a cut on another arm.
/// The overlap is made before any copper is routed, since a footprint move carries track ends.
#[test]
fn retained_side_pad_overlap_allows_cut() {
    let mut session = routed_fixture();
    let ratsnest = view(&session).ratsnest().unwrap();
    let (left, middle, right) = (ratsnest.lines[0].from.clone(), ratsnest.lines[0].to.clone(), ratsnest.lines[1].to.clone());
    let board = view(&session).target.board;
    let place = |session: &mut Session, footprint: &str, at: Point, rotation: u16| {
        session
            .apply(
                SET_FOOTPRINT_TRANSFORM,
                &json!({"board":board,"footprint":footprint,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"expected_revision":session.revision()}),
            )
            .unwrap();
    };
    place(&mut session, &middle.footprint, Point { x_nm: middle.position.x_nm, y_nm: middle.position.y_nm + 4_000_000 }, 0);
    // Turn the right part so its other pad stands clear of the trunk, then set its routed pad
    // on the retained left pad.
    let center = view(&session).footprints().find(|f| f.id == right.footprint).unwrap().position;
    place(&mut session, &right.footprint, center, 90);
    let pad_at = |session: &Session, pad: crate::board::Pad| session.project().unwrap().board.pad_position(pad).unwrap();
    let (target, current) = (pad_at(&session, left.pad), pad_at(&session, right.pad));
    let shifted = Point { x_nm: center.x_nm + target.x_nm - current.x_nm, y_nm: center.y_nm + target.y_nm - current.y_nm };
    place(&mut session, &right.footprint, shifted, 90);
    assert_eq!(pad_at(&session, right.pad), pad_at(&session, left.pad));
    let tip = pad_at(&session, middle.pad);
    let junction = Point { x_nm: tip.x_nm, y_nm: left.position.y_nm };
    let front = copper_layers(&session)[0];
    route(&mut session, front, &[left.position, Point { x_nm: tip.x_nm + 2_000_000, y_nm: junction.y_nm }]).unwrap();
    let trunk = view(&session).tracks()[0].id;
    route(&mut session, front, &[tip, junction]).unwrap();
    let arm = view(&session).tracks().iter().find(|track| track.id != trunk).unwrap().id;
    let mut args = json!({"board":board,"from":address(middle.pad),"to":address(left.pad),"expected_revision":session.revision()});
    for dry_run in [true, false] {
        args["dry_run"] = json!(dry_run);
        let report = session.execute(UNROUTE_CONNECTION, &args).unwrap();
        assert_eq!(report["removed"], json!({"tracks":[arm],"vias":[]}));
        assert_eq!(report["unrouted"].as_array().unwrap().len(), 1);
    }
    assert_eq!(view(&session).tracks().iter().map(|track| track.id).collect::<Vec<_>>(), vec![trunk]);
}

/// Replace the Tee branch with one that drops from the branch pad to back copper through a
/// via at `anchor` and returns to the trunk through a via at the junction, then assign the
/// anchor its net explicitly. An anchor at the tip sits on the pad, with only back copper.
fn layered_branch(tee: &mut Tee, anchor: Point) {
    let Tee { session, tip, junction, arm, orphan, .. } = tee;
    let (tip, junction) = (*tip, *junction);
    let [front, back] = [copper_layers(session)[0], copper_layers(session)[1]];
    session.apply(DELETE_TRACKS, &json!({"tracks":[arm],"vias":[orphan],"expected_revision":session.revision()})).unwrap();
    let (layer, points, changes) = if anchor == tip {
        (back, json!([tip, junction]), json!([{"at":1,"layer":front}]))
    } else {
        (front, json!([tip, anchor, junction]), json!([{"at":1,"layer":back},{"at":2,"layer":front}]))
    };
    session
        .apply(
            ADD_TRACKS,
            &json!({"layer":layer,"width_nm":DEFAULT_WIDTH_NM,"points":points,"layer_changes":changes,"expected_revision":session.revision()}),
        )
        .unwrap();
    if anchor == tip {
        session.apply(ADD_VIA, &json!({"at":anchor,"expected_revision":session.revision()})).unwrap();
    }
    let id = view(session).vias().iter().find(|via| via.at == anchor).unwrap().id;
    let target = json!(view(session).net_class_target_for_copper(&id.to_string()).unwrap().unwrap());
    session.apply(SET_VIA_NET, &json!({"vias":[id],"target":target,"expected_revision":session.revision()})).unwrap();
    assert!(view(session).vias().iter().any(|via| via.id == id && via.net_assignment.is_some()));
}

/// An explicitly assigned via on the selected path is kept with the copper beyond it; only
/// the copper between it and the detached pad is cut, in preview and apply alike.
#[test]
fn assigned_via_on_selected_path_keeps_its_copper() {
    let mut tee = tee();
    let anchor = Point { x_nm: tee.tip.x_nm, y_nm: tee.tip.y_nm - 2_000_000 };
    layered_branch(&mut tee, anchor);
    let Tee { mut session, mut args, tip, .. } = tee;
    let cut = view(&session).tracks().iter().find(|track| track.start == tip || track.end == tip).unwrap().id;
    let kept: Vec<_> = view(&session).tracks().iter().filter(|track| track.id != cut).cloned().collect();
    let vias: Vec<_> = view(&session).vias().iter().map(|via| via.id).collect();
    assert_eq!(vias.len(), 3, "the anchor, the trunk via and the standalone via");
    args["expected_revision"] = json!(session.revision());
    let before = session.snapshot().unwrap();
    args["dry_run"] = json!(true);
    let preview = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(preview["removed"], json!({"tracks":[cut],"vias":[]}));
    assert_eq!(preview["unrouted"], json!([{"from":args["to"],"to":args["from"]}]));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    args["dry_run"] = json!(false);
    let applied = session.execute(UNROUTE_CONNECTION, &args).unwrap();
    assert_eq!(applied["removed"], preview["removed"]);
    assert_eq!(applied["unrouted"], preview["unrouted"]);
    assert_eq!(view(&session).tracks(), kept);
    assert_eq!(view(&session).vias().iter().map(|via| via.id).collect::<Vec<_>>(), vias);
}

/// When the assigned via itself joins the detached pad to its arm, no cut keeps it, so both
/// preview and apply refuse without an edit.
#[test]
fn assigned_via_at_detached_pad_refuses_preview_and_apply() {
    let mut tee = tee();
    let tip = tee.tip;
    layered_branch(&mut tee, tip);
    let Tee { mut session, mut args, .. } = tee;
    args["expected_revision"] = json!(session.revision());
    for dry_run in [true, false] {
        args["dry_run"] = json!(dry_run);
        refuses_without_edit(&mut session, &args, "assigned via");
    }
}

/// Detaching tracks cannot guarantee separation when the detached pad overlaps another.
#[test]
fn directly_overlapping_lands_are_not_reported_as_unrouted() {
    let Tee { mut session, mut args, tip, .. } = tee();
    let footprint = |end: &str| args[end]["footprint"].as_str().unwrap();
    let view = view(&session);
    let branch = view.footprints().find(|f| f.id == footprint("from")).unwrap();
    let retained = view.footprints().find(|f| f.id == footprint("to")).unwrap();
    let selected = retained.pads.iter().find(|p| p.number == args["to"]["pad"].as_str().unwrap()).unwrap().center;
    let x_nm = selected.x_nm - (tip.x_nm - branch.position.x_nm);
    let y_nm = selected.y_nm - (tip.y_nm - branch.position.y_nm);
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"board":args["board"],"footprint":footprint("from"),
        "x_nm":x_nm,"y_nm":y_nm,"rotation":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    args["expected_revision"] = json!(session.revision());
    for dry_run in [true, false] {
        args["dry_run"] = json!(dry_run);
        refuses_without_edit(&mut session, &args, "overlaps another pad");
    }
}

/// Aside from the returned diagnostic, a refusal is observationally read-only.
fn refuses_without_edit(session: &mut Session, args: &Value, message: &str) {
    let before = session.inspect();
    let error = session.execute(UNROUTE_CONNECTION, args).unwrap_err().to_string();
    assert!(error.to_lowercase().contains(&message.to_lowercase()), "{error}");
    assert_eq!(session.inspect(), before);
}
