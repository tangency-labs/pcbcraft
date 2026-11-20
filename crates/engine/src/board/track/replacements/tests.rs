//! Original shared-command fixtures; candidate rejection must preserve both history stacks.
use super::super::tests::{copper_layers, routed_fixture, view};
use super::*;
use crate::{
    INITIALIZE_BOARD_SETUP,
    persistence::{decode_documents, encode_documents},
};

fn p(x: i64, y: i64) -> Point {
    Point { x_nm: x * 1_000_000, y_nm: y * 1_000_000 }
}

/// A blank board avoids unrelated library/pad geometry in transaction tests.
fn board() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s
}

/// Add ordinary authored copper through the same operation that will publish compound routes.
fn add(s: &mut Session, points: Vec<Point>, width: i64) -> Vec<Uuid> {
    let before: BTreeSet<_> = view(s).tracks().iter().map(|t| t.id).collect();
    s.apply(ADD_TRACKS, &json!({"layer":copper_layers(s)[0],"width_nm":width,"points":points,"expected_revision":s.revision()})).unwrap();
    view(s).tracks().iter().filter(|t| !before.contains(&t.id)).map(|t| t.id).collect()
}

/// A separate new route gives each replacement test the same atomic-add boundary.
fn params(s: &Session, replacements: Value) -> Value {
    json!({"board":view(s).target.board,"layer":copper_layers(s)[0],"width_nm":250_000,
        "points":[p(-10,-10),p(-5,-10)],"replacements":replacements,"expected_revision":s.revision()})
}

/// Drain tiny batches, requiring idempotent completion and consumable-once publication data.
fn finish(mut job: preparation::CopperEditJob) -> Result<preparation::PreparedCopperEdit, CommandError> {
    assert!(!job.step(0));
    for _ in 0..100_000 {
        if job.step(1) {
            assert!(job.step(1));
            let result = job.take_result();
            assert!(job.take_result().unwrap().is_none());
            return result.and_then(|edit| edit.ok_or_else(|| CommandError::Unavailable("No edit".into())));
        }
    }
    panic!("finite fixture exceeded validation bound")
}

/// A branch attached inside a mixed-width segment stays connected after replacement. GUI-style
/// prepared publication and the direct shared command produce equivalent stable existing copper.
#[test]
fn compound_route_preserves_branch_width_layer_identity_and_one_history_entry() {
    let mut s = board();
    let id = add(&mut s, vec![p(10, 0), p(10, 20)], 500_000)[0];
    let branch = add(&mut s, vec![p(10, 10), p(15, 10)], 250_000)[0];
    let old_branch = view(&s).tracks().iter().find(|t| t.id == branch).unwrap().clone();
    let layer = copper_layers(&s)[0];
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let rev = s.revision();
    let command = params(&s, json!([{"track":id,"points":[p(10,0),p(12,0),p(12,8),p(10,10),p(12,12),p(12,20),p(10,20)]}]));
    let prepared = finish(s.prepare_copper_edit(ADD_TRACKS, &command).unwrap()).unwrap();
    assert_eq!(s.revision(), rev);
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    s.commit_copper_edit(prepared).unwrap();
    assert_eq!(s.revision(), rev + 1);
    let tracks = view(&s).tracks();
    assert_eq!(tracks.len(), 8);
    assert_eq!(tracks.iter().find(|t| t.id == branch), Some(&old_branch));
    let retained = tracks.iter().find(|t| t.id == id).unwrap();
    assert_eq!((retained.start, retained.end, retained.width_nm, retained.layer), (p(10, 0), p(12, 0), 500_000, layer));
    assert_eq!(view(&s).copper().expect("arc-free fixture copper").tracks[&id], view(&s).copper().expect("arc-free fixture copper").tracks[&branch]);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// Disconnected branches, fixed vias and pad-interior contacts cannot be lost, even when the
/// replacement has no short and retains both of its original endpoints.
#[test]
fn interior_contacts_are_preserved_or_the_whole_edit_is_refused() {
    for fixture in 0..3 {
        let mut s = if fixture == 2 { routed_fixture() } else { board() };
        let (at, reach) = if fixture == 2 {
            let pad = view(&s).footprints().next().unwrap().pads[0].center;
            (pad, 2_000_000)
        } else {
            (p(10, 10), 10_000_000)
        };
        let start = Point { y_nm: at.y_nm - reach, ..at };
        let end = Point { y_nm: at.y_nm + reach, ..at };
        let id = add(&mut s, vec![start, end], 250_000)[0];
        if fixture == 0 {
            add(&mut s, vec![at, Point { x_nm: at.x_nm + 3_000_000, ..at }], 250_000);
        }
        if fixture == 1 {
            s.apply(ADD_VIA, &json!({"at":at,"expected_revision":s.revision()})).unwrap();
        }
        let x = at.x_nm - 5_000_000;
        let command = params(&s, json!([{"track":id,"points":[start,Point { x_nm:x,..start },Point { x_nm:x,..end },end]}]));
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let rev = s.revision();
        let error = finish(s.prepare_copper_edit(ADD_TRACKS, &command).unwrap()).err().unwrap().to_string();
        assert!(error.contains("disconnect") || error.contains("protected contact"), "{fixture}: {error}");
        assert_eq!(rev, s.revision());
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
        assert!(s.apply(ADD_TRACKS, &command).is_err());
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    }
}

/// Unknown membership cannot gain a clearance exemption by making contact during the candidate
/// edit; an empty replacement list still invokes the final compound validator.
#[test]
fn compound_route_refuses_foreign_unknown_copper_and_clearance() {
    for y in [0, 300_000] {
        let mut s = board();
        add(&mut s, vec![p(0, 0), p(10, 0)], 250_000);
        let mut command = params(&s, json!([]));
        command["points"] = json!([Point { x_nm: 5_000_000, y_nm: y }, Point { x_nm: 15_000_000, y_nm: y }]);
        // Start beyond the foreign copper rather than picking it up as the route's own island.
        command["points"][0] = json!(Point { x_nm: 15_000_000, y_nm: y });
        command["points"][1] = json!(Point { x_nm: 5_000_000, y_nm: y });
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        assert!(s.apply(ADD_TRACKS, &command).is_err());
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    }
}

/// Invalid IDs, duplicate originals, malformed paths, attempts to change locks and stale
/// prepared edits cannot publish any part of the new route or replacement.
#[test]
fn malformed_locked_and_stale_replacements_are_atomic() {
    let mut s = board();
    let id = add(&mut s, vec![p(10, 0), p(10, 20)], 250_000)[0];
    let replacement = json!({"track":id,"points":[p(10,0),p(12,0),p(12,20),p(10,20)]});
    for bad in [
        json!([{"track":Uuid::from_u128(7),"points":[p(0,0),p(1,1)]}]),
        json!([replacement.clone(), replacement.clone()]),
        json!([{"track":id,"points":[p(0,0),p(0,0)]}]),
    ] {
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        assert!(s.apply(ADD_TRACKS, &params(&s, bad)).is_err());
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    }
    let prepared = finish(s.prepare_copper_edit(ADD_TRACKS, &params(&s, json!([replacement.clone()]))).unwrap()).unwrap();
    s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[id],"locked":true,"expected_revision":s.revision()})).unwrap();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(s.commit_copper_edit(prepared).is_err());
    assert!(s.apply(ADD_TRACKS, &params(&s, json!([replacement]))).is_err());
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
}

/// Joining rails elsewhere through a branch does not authorize collapsing their separated
/// bodies. Final validation must reject the overlap even when ordinary same-island DRC exempts it.
#[test]
fn compound_validation_rejects_new_overlap_within_a_connected_island() {
    for named in [false, true] {
        let mut s = if named { routed_fixture() } else { board() };
        let a = add(&mut s, vec![p(0, 5), p(20, 5)], 500_000)[0];
        let b = add(&mut s, vec![Point { x_nm: 0, y_nm: 5_800_000 }, Point { x_nm: 20_000_000, y_nm: 5_800_000 }], 250_000)[0];
        add(&mut s, vec![p(17, 5), p(17, 8)], 250_000);
        if named {
            let pad = view(&s).footprints().flat_map(|f| f.pads).find(|p| p.net.is_some()).unwrap().center;
            add(&mut s, vec![pad, Point { y_nm: 5_000_000, ..pad }, p(0, 5)], 250_000);
            let query = s.clearance_query_with_rules(view(&s).target, &json!({})).unwrap();
            assert_eq!(query.objects().iter().find(|o| o.name == a.to_string()).unwrap().intended_nets.len(), 1);
        }
        assert_eq!(view(&s).copper().expect("arc-free fixture copper").tracks[&a], view(&s).copper().expect("arc-free fixture copper").tracks[&b]);
        let command = params(
            &s,
            json!([
                {"track":a,"points":[p(0,5),p(17,5),Point{x_nm:17_500_000,y_nm:5_000_000},Point{x_nm:18_500_000,y_nm:6_000_000},p(19,6),p(20,5)]},
                {"track":b,"points":[Point{x_nm:0,y_nm:5_800_000},Point{x_nm:1_000_000,y_nm:6_100_000},Point{x_nm:16_000_000,y_nm:6_100_000},Point{x_nm:17_000_000,y_nm:5_800_000},Point{x_nm:20_000_000,y_nm:5_800_000}]}
            ]),
        );
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let revision = s.revision();
        assert!(s.apply(ADD_TRACKS, &command).is_err(), "a shared island incorrectly permitted a new overlap");
        assert_eq!(s.revision(), revision);
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
        let mut merging = command;
        merging["merge_same_net"] = json!(true);
        s.apply(ADD_TRACKS, &merging).unwrap();
        assert_eq!(s.revision(), revision + 1);
        let after = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
        s.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(after, encode_documents(s.snapshot().unwrap()).unwrap());
    }
}

/// Opting into same-net merging never authorizes a new connection between distinct nets.
#[test]
fn same_net_merge_option_still_refuses_a_short_atomically() {
    let mut s = routed_fixture();
    let footprint = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
    let mut command = params(&s, json!([]));
    command["points"] = json!([footprint.pads[0].center, footprint.pads[1].center]);
    command["merge_same_net"] = json!(true);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert!(s.apply(ADD_TRACKS, &command).is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
}

/// Uniform degree-two joins may move together; branching, vias, locks and width
/// transitions still define protected source contacts, even for direct command clients.
#[test]
fn compound_chains_move_bends_but_keep_protected_contacts() {
    for fixed in 0..5 {
        let mut s = board();
        add(&mut s, vec![p(0, 0), p(2, 0)], 250_000);
        add(&mut s, vec![p(2, 0), p(2, 2)], if fixed == 4 { 500_000 } else { 250_000 });
        add(&mut s, vec![p(2, 2), p(4, 2)], 250_000);
        let source: Vec<_> = view(&s).tracks().to_vec();
        let ids: Vec<_> = [p(0, 0), p(2, 0), p(2, 2)].into_iter().map(|start| source.iter().find(|t| t.start == start).unwrap().id).collect();
        if fixed == 1 {
            add(&mut s, vec![p(2, 2), p(2, 5)], 250_000);
        }
        if fixed == 2 {
            s.apply(ADD_VIA, &json!({"at":p(2,2),"expected_revision":s.revision()})).unwrap();
        }
        if fixed == 3 {
            s.apply(SET_COPPER_LOCKED, &json!({"board":view(&s).target.board,"tracks":[ids[1]],"locked":true,"expected_revision":s.revision()}))
                .unwrap();
        }
        let command = params(
            &s,
            json!([
                {"track":ids[0],"points":[p(0,0),p(1,0)]},
                {"track":ids[1],"points":[p(1,0),p(1,3)]},
                {"track":ids[2],"points":[p(1,3),p(4,3),p(4,2)]}
            ]),
        );
        let before = encode_documents(s.snapshot().unwrap()).unwrap();
        let rev = s.revision();
        if fixed == 0 {
            let edit = finish(s.prepare_copper_edit(ADD_TRACKS, &command).unwrap()).unwrap();
            assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
            s.commit_copper_edit(edit).unwrap();
            assert_eq!(s.revision(), rev + 1);
            for old in &source {
                let now = view(&s).tracks().iter().find(|t| t.id == old.id).unwrap();
                assert_eq!((now.width_nm, now.layer), (old.width_nm, old.layer));
            }
            let after = encode_documents(s.snapshot().unwrap()).unwrap();
            assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
            s.apply("edit.undo", &Value::Null).unwrap();
            assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
            s.apply("edit.redo", &Value::Null).unwrap();
            assert_eq!(after, encode_documents(s.snapshot().unwrap()).unwrap());
        } else {
            assert!(s.apply(ADD_TRACKS, &command).is_err(), "protected fixture{fixed} was accepted");
            assert_eq!(s.revision(), rev);
            assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
        }
    }
}

/// Chain membership cannot hide a folded trace's lost spacing or a centerline
/// crossing. Explicit merging permits close returning copper, but never a crossing.
#[test]
fn compound_chain_self_spacing_and_crossings_are_checked() {
    for allocation in [false, true] {
        for crossing in [false, true] {
            for merge in [false, true] {
                let mut s = board();
                add(&mut s, vec![p(0, 0), p(2, 0), p(2, 2), p(4, 2)], 250_000);
                let source = view(&s).tracks().to_vec();
                let id = |start| source.iter().find(|t| t.start == start).unwrap().id;
                let near = Point { x_nm: 5_000_000, y_nm: 300_000 };
                let tail = if crossing { vec![near, p(2, -1), p(4, 2)] } else { vec![near, Point { x_nm: 0, y_nm: 300_000 }, p(0, 2), p(4, 2)] };
                let middle =
                    if allocation && !crossing { vec![p(5, 0), near, Point { x_nm: 0, y_nm: 300_000 }, p(0, 2)] } else { vec![p(5, 0), near] };
                let tail = if allocation && !crossing { vec![p(0, 2), p(4, 2)] } else { tail };
                let mut command = params(
                    &s,
                    json!([
                        {"track":id(p(0,0)),"points":[p(0,0),p(5,0)]},
                        {"track":id(p(2,0)),"points":middle},
                        {"track":id(p(2,2)),"points":tail}
                    ]),
                );
                command["merge_same_net"] = json!(merge);
                let before = encode_documents(s.snapshot().unwrap()).unwrap();
                if merge && !crossing {
                    s.apply(ADD_TRACKS, &command).unwrap();
                } else {
                    assert!(s.apply(ADD_TRACKS, &command).is_err());
                    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
                }
            }
        }
    }
}

/// An endpoint touching a nonadjacent diagonal is a self-contact even when its
/// floating distance projection has a tiny residual. Merging does not erase this check.
#[test]
fn compound_chain_self_contact_uses_exact_integer_incidence() {
    let mut s = board();
    add(&mut s, vec![p(0, 0), p(2, 0), p(2, 2), p(4, 2)], 250_000);
    let source = view(&s).tracks().to_vec();
    let id = |start| source.iter().find(|t| t.start == start).unwrap().id;
    let turn = Point { x_nm: 9_000_001, y_nm: 9_000_001 };
    let contact = Point { x_nm: 3_000_011, y_nm: 3_000_011 };
    let mut command = params(
        &s,
        json!([
            {"track":id(p(0,0)),"points":[p(0,0),turn]},
            {"track":id(p(2,0)),"points":[turn,p(10,0),contact]},
            {"track":id(p(2,2)),"points":[contact,p(4,2)]}
        ]),
    );
    command["merge_same_net"] = json!(true);
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert!(s.apply(ADD_TRACKS, &command).is_err(), "nonadjacent exact self-contact was accepted");
    assert_eq!(revision, s.revision());
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
}

/// An off-angle authored leg may retain a fractional crossing while another
/// span detours. A path that disconnects the contact must fail atomically.
#[test]
fn compound_edits_preserve_fractional_crossings() {
    let mut s = board();
    let at = |x_nm, y_nm| Point { x_nm, y_nm };
    let a = at(0, 0);
    let b = at(10_000_000, 3_000_000);
    let root = add(&mut s, vec![a, b], 250_000)[0];
    add(&mut s, vec![at(5_000_001, -2_000_000), at(5_000_001, 4_000_000)], 250_000);
    let revision = s.revision();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let moved = params(&s, json!([{"track":root,"points":[a,p(0,6),p(10,6),b]}]));
    assert!(s.apply(ADD_TRACKS, &moved).is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let retained = params(&s, json!([{"track":root,"points":[a,at(8_000_000,2_400_000),p(9,2),b]}]));
    s.apply(ADD_TRACKS, &retained).expect("the unchanged diagonal span retains the exact crossing");
}

/// A retained T-junction does not permit following the connected branch farther away.
#[test]
fn compound_edits_reject_new_overlap_along_existing_branch() {
    let mut s = board();
    let a = p(0, 0);
    let joint = p(0, 5);
    let b = p(0, 10);
    let track = add(&mut s, vec![a, b], 250_000)[0];
    add(&mut s, vec![joint, p(10, 5)], 250_000);
    let invalid = params(&s, json!([{"track":track,"points":[a,joint,p(8,5),p(8,10),b]}]));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert!(s.apply(ADD_TRACKS, &invalid).is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let valid = params(&s, json!([{"track":track,"points":[a,joint,p(-4,9),p(-3,10),b]}]));
    s.apply(ADD_TRACKS, &valid).expect("an edit departing from the retained junction is legal");
}

/// Through via dimensions obey the via rules even when the track minimum is larger.
#[test]
fn compound_edits_apply_track_width_only_to_tracks() {
    let mut s = board();
    let layers = copper_layers(&s);
    s.apply(
        crate::board::rules::SET_RULES,
        &json!({"board":view(&s).target.board,"rules":crate::board::rules::Values { min_track_width_nm:1_000_000, preferred_track_width_nm:1_000_000, ..Default::default() },"expected_revision":s.revision()}),
    )
    .unwrap();
    let track = add(&mut s, vec![p(0, 0), p(10, 0)], 1_000_000)[0];
    let command = json!({"board":view(&s).target.board,"layer":layers[0],"width_nm":1_000_000,
        "points":[p(-10,-10),p(-5,-10),p(-3,-10)],
        "layer_changes":[{"at":1,"layer":layers[1]}],"via_diameter_nm":600_000,"via_drill_nm":300_000,
        "replacements":[{"track":track,"points":[p(0,0),p(4,4),p(6,4),p(10,0)]}],"expected_revision":s.revision()});
    s.apply(ADD_TRACKS, &command).expect("the via land can be narrower than a trace");
    assert_eq!(view(&s).vias().len(), 1);
}

/// A slight slope on a returning leg must not let a full U-turn pass as a corner.
#[test]
fn compound_edit_enforces_spacing_on_nearly_parallel_return() {
    let mut s = board();
    let source = add(&mut s, vec![p(0, 0), p(12, 5)], 250_000)[0];
    let command = |s: &Session, separation| {
        let at = |x_nm, y_nm| Point { x_nm, y_nm };
        let path = [p(0, 0), p(10, 0), at(10_000_000, separation), at(1_000_000, separation + 1), p(1, 5), p(12, 5)];
        params(s, json!([{"track":source,"points":path}]))
    };
    let revision = s.revision();
    assert!(s.apply(ADD_TRACKS, &command(&s, 300_000)).is_err());
    assert_eq!(s.revision(), revision);
    s.apply(ADD_TRACKS, &command(&s, 600_000)).expect("a separated return is legal");
}
