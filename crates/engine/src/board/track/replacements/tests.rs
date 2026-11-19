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
    let command = params(&s, json!([{"track":id,"points":[p(10,0),p(12,0),p(12,20),p(10,20)]}]));
    let prepared = finish(s.prepare_copper_edit(ADD_TRACKS, &command).unwrap()).unwrap();
    assert_eq!(s.revision(), rev);
    assert_eq!(before, encode_documents(s.snapshot().unwrap()).unwrap());
    s.commit_copper_edit(prepared).unwrap();
    assert_eq!(s.revision(), rev + 1);
    let tracks = view(&s).tracks();
    assert_eq!(tracks.len(), 5);
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
        assert!(error.contains("disconnect"), "{fixture}: {error}");
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
                {"track":a,"points":[p(0,5),p(1,6),p(16,6),p(17,5),p(20,5)]},
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
