use super::*;
use crate::{
    Session,
    board::{arc, drawing, setup::LayerPurpose, teardrop, test_points, track, zones},
    persistence::{decode_documents, encode_documents},
    schematic::Point,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

/// Apply a production command against the current revision, as other clients do.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// A two-layer board with a two-segment route and a through via clear of other copper.
fn fixture() -> Session {
    let mut session = track::tests::routed_fixture();
    let front = track::tests::copper_layers(&session)[0];
    let points = [0, 5_000_000, 10_000_000].map(|x_nm| Point { x_nm, y_nm: 30_000_000 });
    track::tests::route(&mut session, front, &points).unwrap();
    edit(&mut session, track::ADD_VIA, json!({"at":{"x_nm":0,"y_nm":40_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    session
}

/// Moving one via and deleting one track yields precisely two UUID-addressed changes,
/// including after reopening both revisions. Comparison leaves history and native bytes alone.
#[test]
fn moved_via_and_deleted_track_are_the_only_changes() {
    let mut session = fixture();
    let before = session.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let view = before.board_view(board).unwrap();
    let track = &view.tracks()[0];
    let via = &view.vias()[0];
    let track_id = track.id;
    let via_id = via.id;
    edit(&mut session, track::MOVE_TRACKS, json!({"board":board.board,"vias":[via_id],"dx_nm":1_000_000,"dy_nm":0}));
    edit(&mut session, track::DELETE_TRACKS, json!({"board":board.board,"tracks":[track_id]}));
    let after = session.snapshot().unwrap();
    let revision = session.revision();
    let before_docs = encode_documents(before.clone()).unwrap();
    let after_docs = encode_documents(after.clone()).unwrap();

    let diff = compare(&before, &after, board, MAX_REPORTED_CHANGES).unwrap();
    let expected_track = json!({"id":track_id,"kind":"track","change":"removed","before":{
        "layer":track.layer,"width_nm":track::DEFAULT_WIDTH_NM,"start":track.start,"end":track.end
    }});
    let expected_via = json!({"id":via_id,"kind":"via","change":"changed","properties":{
        "at":{"before":{"x_nm":0,"y_nm":40_000_000},"after":{"x_nm":1_000_000,"y_nm":40_000_000}}
    }});
    let expected = if track_id < via_id { [expected_track, expected_via] } else { [expected_via, expected_track] };
    assert_eq!(
        serde_json::to_value(&diff).unwrap(),
        json!({"board":board,"counts":{"added":0,"removed":1,"changed":1},"truncated":false,"changes":expected})
    );
    let reopened_before = decode_documents(&before_docs).unwrap();
    let reopened_after = decode_documents(&after_docs).unwrap();
    assert_eq!(compare(&reopened_before, &reopened_after, board, 2).unwrap(), diff);
    let reversed = compare(&after, &before, board, 2).unwrap();
    assert_eq!(reversed.counts, ChangeCounts { added: 1, removed: 0, changed: 1 });
    assert!(reversed.changes.iter().any(|entry| entry.id == track_id && matches!(entry.change, Change::Added { .. })));
    let same = compare(&after, &reopened_after, board, 0).unwrap();
    assert_eq!(same.counts, ChangeCounts::default());
    assert!(same.changes.is_empty() && !same.truncated);
    assert!(Arc::ptr_eq(&after, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(before.clone()).unwrap(), before_docs);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(compare(&before, session.project().unwrap(), board, 2).unwrap().counts, ChangeCounts { added: 0, removed: 0, changed: 1 });
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after_docs);
}

/// Marking and clearing one via keeps its identity. A missing property side represents no mark.
#[test]
fn via_test_point_set_and_clear_report_only_the_mark() {
    let mut session = fixture();
    let before = session.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let via_id = before.board_view(board).unwrap().vias()[0].id;
    let target = json!({"kind":"via","via":via_id});
    edit(&mut session, test_points::SET_TEST_POINT, json!({"board":board.board,"target":target,"purpose":"assembly","sides":"top"}));
    let marked = session.snapshot().unwrap();
    let reopened_marked = decode_documents(&encode_documents(marked.clone()).unwrap()).unwrap();
    edit(&mut session, test_points::SET_TEST_POINT, json!({"board":board.board,"target":target,"purpose":"assembly","sides":null}));
    let cleared = session.project().unwrap();

    for (old, new, reopened_old, reopened_new, property) in [
        (&*before, &*marked, &*before, &*reopened_marked, json!({"after":{"assembly":"top"}})),
        (&*marked, cleared, &*reopened_marked, cleared, json!({"before":{"assembly":"top"}})),
    ] {
        let diff = compare(old, new, board, 1).unwrap();
        assert_eq!(compare(reopened_old, reopened_new, board, 1).unwrap(), diff);
        assert_eq!(
            serde_json::to_value(diff).unwrap(),
            json!({"board":board,"counts":{"added":0,"removed":0,"changed":1},"truncated":false,"changes":[
                {"id":via_id,"kind":"via","change":"changed","properties":{"test_point":property}}
            ]})
        );
    }
    assert_eq!(compare(&before, cleared, board, 1).unwrap().counts, ChangeCounts::default());
}

/// A comparison spanning creation or removal includes both authored test-point purposes.
#[test]
fn marked_via_addition_and_removal_include_test_point_properties() {
    let mut session = fixture();
    let board = session.project().unwrap().board_target().unwrap();
    let via_id = session.project().unwrap().board_view(board).unwrap().vias()[0].id;
    edit(&mut session, track::DELETE_TRACKS, json!({"board":board.board,"vias":[via_id]}));
    let absent = session.snapshot().unwrap();
    edit(
        &mut session,
        track::ADD_VIA,
        json!({"board":board.board,"at":{"x_nm":0,"y_nm":40_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000}),
    );
    let via_id = session.project().unwrap().board_view(board).unwrap().vias()[0].id;
    let target = json!({"kind":"via","via":via_id});
    edit(&mut session, test_points::SET_TEST_POINT, json!({"board":board.board,"target":target,"purpose":"assembly","sides":"top"}));
    edit(&mut session, test_points::SET_TEST_POINT, json!({"board":board.board,"target":target,"purpose":"fabrication","sides":"both"}));
    let marked = session.project().unwrap();
    let properties = json!({"at":{"x_nm":0,"y_nm":40_000_000},"diameter_nm":600_000,"drill_nm":300_000,
        "test_point":{"assembly":"top","fabrication":"both"}});

    let added = compare(&absent, marked, board, 1).unwrap();
    assert_eq!(added.counts, ChangeCounts { added: 1, removed: 0, changed: 0 });
    assert_eq!(serde_json::to_value(added.changes).unwrap(), json!([{"id":via_id,"kind":"via","change":"added","after":properties}]));
    let removed = compare(marked, &absent, board, 1).unwrap();
    assert_eq!(removed.counts, ChangeCounts { added: 0, removed: 1, changed: 0 });
    assert_eq!(serde_json::to_value(removed.changes).unwrap(), json!([{"id":via_id,"kind":"via","change":"removed","before":properties}]));
}

/// Geometry reuse does not merge UUIDs. A small report retains the deterministic prefix and still
/// counts every omitted row. Check both sides of the report cap and both absent-board errors.
#[test]
fn replacement_and_report_bounds() {
    let mut session = fixture();
    let before = session.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let via = &before.board_view(board).unwrap().vias()[0];
    edit(&mut session, track::DELETE_TRACKS, json!({"board":board.board,"vias":[via.id]}));
    edit(&mut session, track::ADD_VIA, json!({"board":board.board,"at":via.at,"via_diameter_nm":via.diameter_nm,"via_drill_nm":via.drill_nm}));
    let after = session.project().unwrap();
    let full = compare(&before, after, board, MAX_REPORTED_CHANGES).unwrap();
    let replacement = &after.board_view(board).unwrap().vias()[0];
    assert_ne!(replacement.id, via.id);
    assert_eq!(full.counts, ChangeCounts { added: 1, removed: 1, changed: 0 });
    assert!(full.changes.iter().any(|entry| entry.id == replacement.id && matches!(entry.change, Change::Added { .. })));
    assert!(full.changes.iter().any(|entry| entry.id == via.id && matches!(entry.change, Change::Removed { .. })));
    assert!(!full.truncated);
    for limit in [0, 1] {
        let partial = compare(&before, after, board, limit).unwrap();
        assert_eq!(partial.counts, full.counts);
        assert_eq!(partial.changes, full.changes[..limit]);
        assert!(partial.truncated);
    }
    assert_eq!(compare(&before, after, board, MAX_REPORTED_CHANGES + 1), Err(BoardDiffError::LimitExceeded));
    let mut other = Session::default();
    other.apply("project.new", &json!({"create_board":true})).unwrap();
    let other = other.project().unwrap();
    assert_eq!(compare(other, after, board, 1), Err(BoardDiffError::MissingBeforeBoard));
    assert_eq!(compare(&before, other, board, 1), Err(BoardDiffError::MissingAfterBoard));
}

/// Save the earlier native rectangle representation, which current drawing commands no longer create.
fn saved_legacy_rectangle() -> Arc<Project> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let mut project = session.snapshot().unwrap();
    Arc::make_mut(&mut Arc::make_mut(&mut project).board).outline =
        Some(crate::board::outline::Outline::rectangle(Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 10_000_000, y_nm: 10_000_000 }).unwrap());
    decode_documents(&encode_documents(project).unwrap()).unwrap()
}

/// Editing the retained rectangle materializes it with the same drawing ID and reports only the changed points.
#[test]
fn legacy_rectangle_edit_is_one_changed_drawing_after_reopen() {
    let before = saved_legacy_rectangle();
    let board = before.board_target().unwrap();
    let outline = before.board_view(board).unwrap().drawings().remove(0);
    let mut points = outline.points.clone();
    points[1].x_nm += 1_000_000;
    let mut session = Session::default();
    session.open_snapshot(before.clone());
    edit(&mut session, drawing::SET_DRAWING, json!({"drawing":outline.id,"points":points,"closed":true}));
    let after = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    let diff = compare(&before, &after, board, 2).unwrap();
    assert_eq!(diff.counts, ChangeCounts { added: 0, removed: 0, changed: 1 });
    assert_eq!(
        serde_json::to_value(diff.changes).unwrap(),
        json!([{
            "id":outline.id,"kind":"drawing","change":"changed","properties":{"points":{"before":outline.points,"after":points}}
        }])
    );
}

/// Adding another graphic materializes the legacy rectangle, but the unchanged rectangle is not a difference.
#[test]
fn legacy_rectangle_materialized_by_other_edit_is_unchanged_after_reopen() {
    let before = saved_legacy_rectangle();
    let board = before.board_target().unwrap();
    let outline = before.board_view(board).unwrap().drawings().remove(0);
    let points = [Point { x_nm: 20_000_000, y_nm: 0 }, Point { x_nm: 20_000_000, y_nm: 10_000_000 }];
    let mut session = Session::default();
    session.open_snapshot(before.clone());
    edit(&mut session, drawing::ADD_DRAWING, json!({"layer":outline.layer,"points":points,"closed":false}));
    let after = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    let drawings = after.board_view(board).unwrap().drawings();
    assert!(drawings.contains(&outline));
    let added = drawings.iter().find(|drawing| drawing.id != outline.id).unwrap();
    let diff = compare(&before, &after, board, 2).unwrap();
    assert_eq!(diff.counts, ChangeCounts { added: 1, removed: 0, changed: 0 });
    assert_eq!(
        serde_json::to_value(diff.changes).unwrap(),
        json!([{
            "id":added.id,"kind":"drawing","change":"added","after":{"layer":outline.layer,"points":points,"closed":false}
        }])
    );
}

/// Authored drawings and zone outlines use their own UUIDs. Adding a previously absent stroke
/// must not report it as null; the outline is compared as an authored object, not filled copper.
#[test]
fn drawings_and_zone_outlines_report_authored_properties() {
    let mut session = fixture();
    let view = track::tests::view(&session);
    let board = view.target;
    let outline_layer = view.layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let copper_layer = track::tests::copper_layers(&session)[0];
    let points = [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 10_000_000, y_nm: 0 }, Point { x_nm: 0, y_nm: 10_000_000 }];
    let nets = session.project().unwrap().inspect_net_registry().unwrap();
    edit(&mut session, zones::ADD_ZONE, json!({"layer":copper_layer,"points":points,"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]}}));
    edit(&mut session, drawing::ADD_DRAWING, json!({"layer":outline_layer,"points":points,"closed":true}));
    let before = session.snapshot().unwrap();
    let view = before.board_view(board).unwrap();
    let drawing = view.drawings().into_iter().find(|drawing| drawing.points == points).unwrap().id;
    let zone = view.zones()[0].id;
    edit(&mut session, drawing::SET_DRAWING, json!({"drawing":drawing,"points":points,"closed":true,"stroke_nm":150_000}));
    edit(&mut session, zones::DELETE_ZONES, json!({"zones":[zone]}));
    let diff = compare(&before, session.project().unwrap(), board, 2).unwrap();
    let rows = serde_json::to_value(&diff.changes).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(diff.counts, ChangeCounts { added: 0, removed: 1, changed: 1 });
    assert!(rows.contains(&json!({"id":drawing,"kind":"drawing","change":"changed","properties":{"stroke_nm":{"after":150_000}}})));
    assert!(
        rows.iter()
            .any(|row| row["id"] == json!(zone) && row["kind"] == "zone" && row["change"] == "removed" && row["before"]["points"] == json!(points))
    );
}

/// Circular arcs are stored records the comparison cannot yet report, so adding one refuses
/// the comparison in either order instead of reporting an unchanged board.
#[test]
fn boards_with_arc_tracks_are_refused_instead_of_reported_unchanged() {
    let mut session = fixture();
    let before = session.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let arc = arc::ArcTrack {
        id: Uuid::from_u128(0xa11c),
        layer: track::tests::copper_layers(&session)[0],
        width_nm: 250_000,
        center: Point { x_nm: 0, y_nm: 0 },
        start: Point { x_nm: 5_000_000, y_nm: 0 },
        end: Point { x_nm: 0, y_nm: 5_000_000 },
        direction: arc::Direction::Clockwise,
    };
    edit(&mut session, arc::SET_ARCS, json!({"board":board.board,"arcs":[arc]}));
    let after = session.snapshot().unwrap();
    assert_eq!(after.board_view(board).unwrap().arc_tracks().len(), 1);
    for (old, new) in [(&before, &after), (&after, &before), (&after, &after)] {
        assert_eq!(compare(old, new, board, MAX_REPORTED_CHANGES), Err(BoardDiffError::UnsupportedArcs));
    }
    assert_eq!(BoardDiffError::UnsupportedArcs.to_string(), arc::UNSUPPORTED);
    assert_eq!(compare(&before, &before, board, 0).unwrap().counts, ChangeCounts::default());
}

/// Via teardrops are stored records the comparison does not report, so a revision holding one
/// is refused in either order with the teardrop error, not the arc error.
#[test]
fn boards_with_via_teardrops_are_refused_instead_of_reported_unchanged() {
    let session = fixture();
    let before = session.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let view = before.board_view(board).unwrap();
    let record = teardrop::Record {
        id: Uuid::from_u128(0x7ea4),
        via: view.vias()[0].id,
        track: view.tracks()[0].id,
        parameters: teardrop::Parameters { length: teardrop::Length::Percent(50), max_length_nm: None, width_pct: 80, max_width_nm: None },
    };
    let mut after = before.clone();
    Arc::make_mut(&mut Arc::make_mut(&mut after).board).teardrops.push(record);
    assert_eq!(after.board_view(board).unwrap().teardrops().len(), 1);
    for (old, new) in [(&before, &after), (&after, &before), (&after, &after)] {
        assert_eq!(compare(old, new, board, MAX_REPORTED_CHANGES), Err(BoardDiffError::UnsupportedTeardrops));
    }
    assert_eq!(BoardDiffError::UnsupportedTeardrops.to_string(), teardrop::UNSUPPORTED);
    assert_eq!(compare(&before, &before, board, 0).unwrap().counts, ChangeCounts::default());
}
