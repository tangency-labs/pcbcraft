//! Track merge regressions on boards built through the shared production commands.
use super::*;
use crate::persistence::{decode_documents, encode_documents};

const MM: i64 = 1_000_000;

fn at(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// A segment's ends in coordinate order, so either direction compares equal.
fn ends(start: Point, end: Point) -> [(i64, i64); 2] {
    let mut ends = [(start.x_nm, start.y_nm), (end.x_nm, end.y_nm)];
    ends.sort_unstable();
    ends
}

fn board(session: &Session) -> Uuid {
    session.project().unwrap().board_target().unwrap().board
}

/// Every track ID on the board, in ID order.
fn all_tracks(session: &Session) -> Vec<Uuid> {
    tests::view(session).tracks().iter().map(|track| track.id).collect()
}

/// Run the merge through the shared command entry point used by GUI and MCP callers.
fn merge(session: &mut Session, tracks: &[Uuid], dry_run: bool) -> Value {
    let params = json!({"board":board(session),"tracks":tracks,"dry_run":dry_run,"expected_revision":session.revision()});
    let result = session.execute(MERGE_TRACKS, &params).unwrap();
    assert_eq!(result["revision"], session.revision());
    result["merge_tracks"].clone()
}

fn add(session: &mut Session, layer: Uuid, width_nm: i64, points: &[Point]) {
    session.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":width_nm,"points":points,"expected_revision":session.revision()})).unwrap();
}

/// Which pads share an island, independent of island numbering and track identities.
fn pad_groups(session: &Session) -> BTreeSet<BTreeSet<(String, String)>> {
    let copper = tests::view(session).copper().unwrap();
    let mut groups: BTreeMap<usize, BTreeSet<(String, String)>> = BTreeMap::new();
    for (pad, island) in copper.pads {
        groups.entry(island).or_default().insert(pad);
    }
    groups.into_values().collect()
}

/// Three collinear segments leaving a pad on its net, plus a stacked duplicate of the middle
/// one, become one segment from the pad to the far end in one undoable edit; connectivity is
/// unchanged, a repeat is not an edit, and the result saves and reopens.
#[test]
fn collinear_and_duplicate_segments_merge_into_one_undoable_segment() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    let a = tests::view(&s).ratsnest().unwrap().lines[0].from.position;
    let down = |steps: i64| at(a.x_nm, a.y_nm + steps * MM);
    add(&mut s, front, DEFAULT_WIDTH_NM, &[a, down(1), down(2), down(3)]);
    add(&mut s, front, DEFAULT_WIDTH_NM, &[down(1), down(2)]);
    let original = s.snapshot().unwrap();
    let before = encode_documents(original.clone()).unwrap();
    let tracks = all_tracks(&s);
    assert_eq!(tracks.len(), 4);
    let groups = pad_groups(&s);
    let net = tests::view(&s).copper_nets().unwrap()[&tracks[0]].clone();
    let revision = s.revision();

    let report = merge(&mut s, &tracks, false);
    assert_eq!(s.revision(), revision + 1, "one merge is one edit");
    assert_eq!(report["changed"], true);
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["removed"].as_array().unwrap().len(), 3);
    assert_eq!(report["before"].as_array().unwrap().len(), 4);
    assert_eq!(report["after"].as_array().unwrap().len(), 1);
    let merged = tests::view(&s).tracks().to_vec();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].id, tracks[0], "the lowest UUID survives");
    assert_eq!(ends(merged[0].start, merged[0].end), ends(a, down(3)));
    assert_eq!((merged[0].layer, merged[0].width_nm), (front, DEFAULT_WIDTH_NM));
    assert_eq!(report["reshaped"], json!([merged[0].id]));
    assert_eq!(report["after"][0], merged[0].inspect());
    assert_eq!(pad_groups(&s), groups, "merging never changes which pads connect");
    assert_eq!(tests::view(&s).copper_nets().unwrap()[&merged[0].id], net);

    let merged_revision = s.revision();
    let again = merge(&mut s, &[merged[0].id], false);
    assert_eq!(again["changed"], false);
    assert_eq!(s.revision(), merged_revision, "nothing left to merge is not an edit");

    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.board.tracks, merged);
    assert_eq!(encode_documents(reopened).unwrap(), saved, "saving a reopened merge changes nothing");

    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before, "one undo restores all four segments");
    crate::assert_same_content(&original, &s.snapshot().unwrap());
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// A dry run reports the same diff as the applied merge without touching revision or history.
#[test]
fn a_dry_run_reports_the_merge_and_changes_nothing() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    let y = 40 * MM;
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, y), at(MM, y), at(2 * MM, y)]);
    let tracks = all_tracks(&s);
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let can_undo = s.inspect()["can_undo"].clone();
    let preview = merge(&mut s, &tracks, true);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["changed"], true);
    assert_eq!(preview["removed"], json!([tracks[1]]));
    assert!(std::sync::Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    assert_eq!(s.inspect()["can_undo"], can_undo);
    let applied = merge(&mut s, &tracks, false);
    for key in ["before", "after", "removed", "reshaped", "locked", "changed"] {
        assert_eq!(preview[key], applied[key], "{key}");
    }
}

/// A third track end at the shared end, a locked segment, a different width, a different
/// layer and a via on the shared end each keep the segments apart.
#[test]
fn junctions_locks_widths_layers_and_vias_block_a_join() {
    let mut s = tests::routed_fixture();
    let [front, back] = tests::copper_layers(&s)[..] else { panic!("two copper layers") };
    let row = |index: i64| 40 * MM + index * 5 * MM;
    // A T at the shared end: the stub is not addressed but still counts.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(0)), at(MM, row(0)), at(2 * MM, row(0))]);
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(MM, row(0)), at(MM, row(0) + MM)]);
    // A locked segment.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(1)), at(MM, row(1)), at(2 * MM, row(1))]);
    // Different widths.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(2)), at(MM, row(2))]);
    add(&mut s, front, 400_000, &[at(MM, row(2)), at(2 * MM, row(2))]);
    // Different layers.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(3)), at(MM, row(3))]);
    add(&mut s, back, DEFAULT_WIDTH_NM, &[at(MM, row(3)), at(2 * MM, row(3))]);
    // A via on the shared end.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(4)), at(MM, row(4)), at(2 * MM, row(4))]);
    s.apply(
        ADD_VIA,
        &json!({"board":board(&s),"at":at(MM, row(4)),"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let locked = tests::view(&s).tracks().iter().find(|track| track.start == at(0, row(1))).unwrap().id;
    s.apply(SET_COPPER_LOCKED, &json!({"board":board(&s),"tracks":[locked],"locked":true,"expected_revision":s.revision()})).unwrap();
    let stub = tests::view(&s).tracks().iter().find(|track| track.end == at(MM, row(0) + MM)).unwrap().id;
    let addressed: Vec<_> = all_tracks(&s).into_iter().filter(|id| *id != stub).collect();
    let revision = s.revision();
    let report = merge(&mut s, &addressed, false);
    assert_eq!(report["changed"], false, "{report}");
    assert_eq!(report["locked"], json!([locked]));
    assert_eq!(s.revision(), revision);
    assert_eq!(tests::view(&s).tracks().len(), 11);

    // A stacked copy merges into its twin even beside the via, which still blocks the join.
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, row(4)), at(MM, row(4))]);
    let tracks = all_tracks(&s);
    let report = merge(&mut s, &tracks, false);
    assert_eq!(report["removed"].as_array().unwrap().len(), 1, "only the stacked copy merges past the via: {report}");
}

/// Collinear segments that meet at a pad stay apart, and diagonal and reversed segments merge
/// along their own line while the survivor keeps its direction.
#[test]
fn pads_block_joins_and_any_direction_merges() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    let pad = tests::view(&s).ratsnest().unwrap().lines[0].from.position;
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(pad.x_nm, pad.y_nm - 3 * MM), pad, at(pad.x_nm, pad.y_nm + 3 * MM)]);
    let tracks = all_tracks(&s);
    let report = merge(&mut s, &tracks, false);
    assert_eq!(report["changed"], false, "a pad between two segments is a junction: {report}");

    let base = 50 * MM;
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(3 * MM, base + 3 * MM), at(2 * MM, base + 2 * MM)]);
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(MM, base + MM), at(2 * MM, base + 2 * MM)]);
    let diagonal: Vec<_> = tests::view(&s).tracks().iter().filter(|track| track.start.y_nm >= base).cloned().collect();
    let survivor = diagonal.iter().min_by_key(|track| track.id).unwrap().clone();
    let report = merge(&mut s, &diagonal.iter().map(|track| track.id).collect::<Vec<_>>(), false);
    assert_eq!(report["removed"].as_array().unwrap().len(), 1, "{report}");
    let merged = tests::view(&s).tracks().iter().find(|track| track.id == survivor.id).unwrap().clone();
    assert_eq!(ends(merged.start, merged.end), ends(at(MM, base + MM), at(3 * MM, base + 3 * MM)));
    let direction = |track: &Track| (track.end.x_nm - track.start.x_nm).signum();
    assert_eq!(direction(&merged), direction(&survivor), "the survivor keeps its direction");
}

/// Bad selections are refused before anything changes.
#[test]
fn bad_selections_are_refused() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, 40 * MM), at(MM, 40 * MM), at(2 * MM, 40 * MM)]);
    let tracks = all_tracks(&s);
    let revision = s.revision();
    let target = board(&s);
    for (bad, reason) in [
        (json!({"board":target,"tracks":[],"expected_revision":revision}), "must contain unique non-nil track UUIDs"),
        (json!({"board":target,"tracks":[tracks[0],tracks[0]],"expected_revision":revision}), "must contain unique non-nil track UUIDs"),
        (json!({"board":target,"tracks":[tracks[0],Uuid::from_u128(3)],"expected_revision":revision}), "must be unique IDs of existing board items"),
        (json!({"board":Uuid::from_u128(23),"tracks":tracks,"expected_revision":revision}), "unknown board ID"),
        (json!({"tracks":tracks,"expected_revision":revision}), "needs `board`"),
    ] {
        let error = s.execute(MERGE_TRACKS, &bad).unwrap_err();
        assert!(error.to_string().contains(reason), "{bad}: {error}");
        assert_eq!(s.revision(), revision);
    }
    let stale = json!({"board":target,"tracks":tracks,"expected_revision":revision - 1});
    let error = s.execute(MERGE_TRACKS, &stale).unwrap_err();
    assert!(matches!(error, CommandError::StaleRevision { current } if current == revision), "{error:?}");
    assert_eq!(all_tracks(&s), tracks);
}

/// A long chain with stacked duplicates collapses to one segment in one edit.
#[test]
fn a_long_chain_with_duplicates_collapses_to_one_segment() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    let y = 60 * MM;
    let chain: Vec<_> = (0..=200).map(|i| at(i * MM / 2, y)).collect();
    let copies: Vec<_> = (0..=49).map(|i| at(i * 2 * MM, y)).collect();
    add(&mut s, front, DEFAULT_WIDTH_NM, &chain);
    add(&mut s, front, DEFAULT_WIDTH_NM, &copies);
    let tracks = all_tracks(&s);
    assert_eq!(tracks.len(), 200 + 49);
    let revision = s.revision();
    let report = merge(&mut s, &tracks, false);
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(report["removed"].as_array().unwrap().len(), tracks.len() - 1);
    let merged = tests::view(&s).tracks().to_vec();
    assert_eq!(merged.len(), 1);
    assert_eq!(ends(merged[0].start, merged[0].end), ends(at(0, y), at(100 * MM, y)));
}

/// A report one byte over its budget is refused with `cap_exceeded` before anything changes,
/// for a dry run and an applied merge alike; at exactly its encoded size it is accepted.
#[test]
fn an_oversized_report_is_refused_before_publishing() {
    let mut s = tests::routed_fixture();
    let front = tests::copper_layers(&s)[0];
    let y = 40 * MM;
    add(&mut s, front, DEFAULT_WIDTH_NM, &[at(0, y), at(MM, y), at(2 * MM, y), at(3 * MM, y)]);
    let tracks = all_tracks(&s);
    let target = board(&s);
    let request = |s: &Session, dry_run: bool| json!({"board":target,"tracks":tracks,"dry_run":dry_run,"expected_revision":s.revision()});
    let preview = s.merge_tracks_within(&request(&s, true), usize::MAX).unwrap()["merge_tracks"].clone();
    let dry_size = serde_json::to_vec(&preview).unwrap().len();
    let mut applied = preview.clone();
    applied["dry_run"] = json!(false);
    let apply_size = serde_json::to_vec(&applied).unwrap().len();
    assert!(merge::report_fits(&preview, dry_size));
    assert!(!merge::report_fits(&preview, dry_size - 1));

    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let can_undo = s.inspect()["can_undo"].clone();
    for (dry_run, size) in [(true, dry_size), (false, apply_size)] {
        let error = s.merge_tracks_within(&request(&s, dry_run), size - 1).unwrap_err();
        assert_eq!(error.code(), crate::error_codes::ErrorCode::CapExceeded, "{error}");
        assert!(error.to_string().contains("merge fewer tracks"), "{error}");
        assert!(std::sync::Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
        assert_eq!(s.inspect()["can_undo"], can_undo);
    }
    let result = s.merge_tracks_within(&request(&s, false), apply_size).unwrap();
    assert_eq!(result["merge_tracks"], applied);
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(tests::view(&s).tracks().len(), 1);
}
