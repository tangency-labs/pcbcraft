use super::*;
use crate::{
    APPLY_FOOTPRINT_REVISION, INITIALIZE_UPDATE_BASELINE,
    board::update::{Baseline, DISPLAY_FORMAT, QUANTITY_FORMAT},
    edits::{CommittedEdit, EditObserver},
    error_codes::ErrorCode,
    persistence::decode_documents,
};
use std::sync::Mutex;

#[derive(Debug, Default)]
struct EditLog(Mutex<Vec<CommittedEdit>>);

impl EditObserver for EditLog {
    fn opened(&self, _: Arc<Project>) {}

    fn committed(&self, edit: CommittedEdit) {
        self.0.lock().unwrap().push(edit);
    }
}

fn history(session: &Session) -> (u64, usize, usize) {
    (session.revision(), session.undo.len(), session.redo.len())
}

fn assert_history_unchanged(session: &Session, before: &Arc<Project>, before_history: (u64, usize, usize)) {
    assert!(Arc::ptr_eq(before, &session.snapshot().unwrap()));
    assert_eq!(history(session), before_history);
}

fn reviewed(session: &mut Session, target: &Value) -> Value {
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, target).unwrap();
    assert_eq!(report["delta"]["pad_numbers_changed"], true);
    assert!(report["delta"]["pads"].as_array().unwrap().iter().any(|pad| pad["number"] == "9" && pad["changes"] == json!(["added"])));
    let mut params = target.clone();
    params["acceptance"] = report["acceptance"].clone();
    params["expected_revision"] = report["revision"].clone();
    params
}

/// Pad 9 must be explicitly resolved. Copper, placement pose and accepted topology stay in
/// place; one edit updates all package records and can be undone/redone or opened offline.
#[test]
fn footprint_revision_adding_pad_nine_requires_mapping_and_preserves_copper() {
    let (mut session, _, target) = package_fixture(|pad| pad.center.x_nm += 200_000, false, true);
    let front = layer(&session, Side::Front);
    edit(&mut session, ADD_TRACKS, json!({"layer":front,"width_nm":100_000,"points":[{"x_nm":100_000,"y_nm":0},{"x_nm":200_000,"y_nm":0}]}));
    edit(&mut session, ADD_VIA, json!({"at":{"x_nm":650_000,"y_nm":0},"via_diameter_nm":120_000,"via_drill_nm":60_000}));
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let log = Arc::new(EditLog::default());
    session.observe_edits(Some(log.clone()));
    let mut params = reviewed(&mut session, &target);
    let before = session.snapshot().unwrap();
    let before_history = history(&session);
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = session.revision();
    let catalog = session.libraries().generation();
    for bad in [None, Some(json!({"1":"1","2":"2"})), Some(json!({"1":["1","9"],"2":["2","9"]})), Some(json!({"1":"1","2":["2","99"]}))] {
        if let Some(bad) = bad {
            params["mapping"] = bad;
        }
        assert_eq!(session.execute(APPLY_FOOTPRINT_REVISION, &params).unwrap_err().code(), ErrorCode::InvalidParams);
        assert_history_unchanged(&session, &before, before_history);
    }
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    let mut bad = params.clone();
    bad["acceptance"] = json!("0".repeat(64));
    assert_eq!(session.apply(APPLY_FOOTPRINT_REVISION, &bad).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_history_unchanged(&session, &before, before_history);
    assert!(log.0.lock().unwrap().is_empty());
    session.execute(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    {
        let log = log.0.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].command, APPLY_FOOTPRINT_REVISION);
        assert_eq!(log[0].params, params);
        assert_eq!(log[0].revision, revision + 1);
    }
    session.observe_edits(None);
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(session.libraries().generation(), catalog);
    let adopted = session.snapshot().unwrap();
    let old = &before.board.footprints()[0];
    let placed = &adopted.board.footprints()[0];
    assert_eq!(placed.definition_revision, 2);
    assert_eq!((placed.id, placed.x_nm, placed.y_nm, placed.rotation, placed.bottom), (old.id, old.x_nm, old.y_nm, old.rotation, old.bottom));
    assert_eq!(adopted.board.tracks, before.board.tracks);
    assert_eq!(adopted.board.vias, before.board.vias);
    assert!(placed.mapping.as_ref().unwrap()["2"].contains("9"));
    assert_eq!(adopted.board.assignments[&old.component].definition_revision, 2);
    let accepted = adopted.board.baseline.as_ref().unwrap();
    let original = before.board.baseline.as_ref().unwrap();
    assert_eq!(accepted.format, crate::board::update::NET_ORIGIN_FORMAT);
    let singleton = crate::board::update::TerminalKey { component: old.component, terminal: "2".into() };
    let net = accepted.nets.iter().find(|net| net.terminals.as_slice() == std::slice::from_ref(&singleton)).unwrap();
    assert_eq!(net.origin, Some(crate::board::update::NetOrigin::PadOnly));
    assert!(net.names.is_empty());
    assert_eq!(accepted.nets.iter().filter(|row| *row != net).collect::<Vec<_>>(), original.nets.iter().collect::<Vec<_>>());
    assert_eq!(accepted.source, original.source);
    let ratsnest = adopted.boards().next().unwrap().ratsnest().unwrap();
    let board = ratsnest.board.board;
    let footprint = format!("footprint-{}", placed.id);
    let two = crate::board::Pad::parse(&footprint, "2").unwrap().on_board(board);
    let nine = crate::board::Pad::parse(&footprint, "9").unwrap().on_board(board);
    assert!(ratsnest.lines.iter().any(|line| line.from.pad == two && line.to.pad == nine || line.from.pad == nine && line.to.pad == two));
    assert_eq!(accepted.components[&old.component].package.as_ref().unwrap().revision, 2);
    let adopted_bytes = encode_documents(adopted).unwrap();
    let mut offline = Session::default();
    offline.open_snapshot(decode_documents(&adopted_bytes).unwrap());
    assert_eq!(encode_documents(offline.snapshot().unwrap()).unwrap(), adopted_bytes);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), adopted_bytes);
}

/// Link two board-eligible parts on terminal 1, leaving terminal 2 free for pad-only nets.
fn connect_another_part(session: &mut Session) -> (String, String) {
    let project = session.snapshot().unwrap();
    let symbol = project.all_symbols().next().unwrap();
    let first = symbol.id();
    edit(
        session,
        PLACE_SYMBOL,
        json!({"library":symbol.definition.library,"definition":symbol.definition.id,"definition_revision":1,
        "definition_digest":symbol_digest(&symbol.definition).unwrap(),"x_nm":0,"y_nm":10_160_000}),
    );
    let second = session.project().unwrap().all_symbols().last().unwrap().id();
    let second_part = session.project().unwrap().components().last().unwrap().id();
    let package = project.package_assignments().values().next().unwrap();
    // Baseline initialization requires packages for all eligible parts, even unplaced ones.
    edit(
        session,
        ASSIGN_FOOTPRINT,
        json!({"component":second_part,"library":package.library,"definition":package.definition,
        "definition_revision":package.definition_revision,"mapping":{"1":"1","2":"2"}}),
    );
    edit(session, crate::CONNECT_PINS, json!({"from":{"symbol":first,"terminal":"1"},"to":{"symbol":second,"terminal":"1"}}));
    (first, second)
}

/// Load revision 3 with the original two pads and prepare a fresh review of pad 9's removal.
fn review_extra_land_removal(session: &mut Session, mut documents: Documents, target: Value) -> Value {
    let (path, mut next) = documents
        .iter()
        .find_map(|(path, text)| {
            let doc: Value = serde_json::from_str(text).ok()?;
            (doc["revision"] == 1 && doc["pads"].is_array()).then(|| (path.clone(), doc))
        })
        .unwrap();
    next["revision"] = json!(3);
    documents.insert(path.replace(".json", "-r3.json"), next.to_string());
    session.execute(REMOVE, &json!({"source":"test"})).unwrap();
    load(session, &documents);
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &target).unwrap();
    assert!(report["delta"]["pads"].as_array().unwrap().iter().any(|pad| pad["number"] == "9" && pad["changes"] == json!(["removed"])));
    let mut params = target;
    params["acceptance"] = report["acceptance"].clone();
    params["expected_revision"] = report["revision"].clone();
    params["mapping"] = json!({"1":"1","2":"2"});
    params
}

/// Removing the last extra land retires only the physical singleton and retains the accepted source.
#[test]
fn footprint_revision_removing_extra_land_retires_singleton_without_recapturing() {
    let (mut session, documents, target) = package_fixture(|_| {}, false, true);
    let (first, second) = connect_another_part(&mut session);
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let original = session.snapshot().unwrap();
    assert_eq!(original.board.baseline.as_ref().unwrap().nets.len(), 1);
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    let added = session.snapshot().unwrap();
    assert_eq!(added.boards().next().unwrap().ratsnest().unwrap().lines.len(), 1);
    // This pending logical connection must not enter the accepted baseline while applying a package.
    edit(&mut session, crate::CONNECT_PINS, json!({"from":{"symbol":first,"terminal":"2"},"to":{"symbol":second,"terminal":"2"}}));
    let params = review_extra_land_removal(&mut session, documents, target);
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    let after = session.snapshot().unwrap();
    let baseline = after.board.baseline.as_ref().unwrap();
    let before = original.board.baseline.as_ref().unwrap();
    assert_eq!(baseline.source, before.source);
    assert_eq!(baseline.format, before.format);
    assert_eq!(baseline.nets, before.nets);
    assert!(after.boards().next().unwrap().ratsnest().unwrap().lines.is_empty());
    let bytes = encode_documents(after).unwrap();
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
}

/// Reopen a valid fixture using the current persistence code, without adding a format reader.
fn reopen_with_baseline(session: &mut Session, update: impl FnOnce(&mut Baseline)) {
    let mut project = session.snapshot().unwrap();
    let board = Arc::make_mut(&mut Arc::make_mut(&mut project).board);
    let baseline = Arc::make_mut(board.baseline.as_mut().unwrap());
    update(baseline);
    baseline.validate().unwrap();
    let documents = encode_documents(project).unwrap();
    session.open_snapshot(decode_documents(&documents).unwrap());
}

/// Check the specific refusal, not just an unrelated validation error or a rejected preview.
fn assert_apply_refused_without_edit(session: &mut Session, params: &Value, message: &str) {
    let before = session.snapshot().unwrap();
    let before_history = history(session);
    let before_inspect = session.inspect();
    let error = session.apply(APPLY_FOOTPRINT_REVISION, params).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidParams);
    assert_eq!(error.to_string(), message);
    assert_history_unchanged(session, &before, before_history);
    let after_inspect = session.inspect();
    assert_eq!(after_inspect["can_undo"], before_inspect["can_undo"]);
    assert_eq!(after_inspect["can_redo"], before_inspect["can_redo"]);
}

/// Adding a pad-only net must not synthesize display metadata for an existing accepted net.
#[test]
fn footprint_revision_refuses_to_infer_missing_accepted_net_display() {
    let (mut session, _, target) = package_fixture(|_| {}, false, true);
    connect_another_part(&mut session);
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    reopen_with_baseline(&mut session, |baseline| {
        assert_eq!(baseline.nets.len(), 1);
        let net = &mut baseline.nets[0];
        assert_eq!(net.terminals.len(), 2);
        assert!(net.display_name.take().is_some());
        baseline.format = QUANTITY_FORMAT.into();
    });
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    assert_apply_refused_without_edit(
        &mut session,
        &params,
        "Cannot add an accepted connection without explicit net display names and singleton origins.",
    );
}

/// Removing the final extra pad must not retire an accepted singleton of unknown origin.
#[test]
fn footprint_revision_refuses_to_remove_singleton_of_unknown_origin() {
    let (mut session, documents, target) = package_fixture(|_| {}, false, true);
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    reopen_with_baseline(&mut session, |baseline| {
        assert_eq!(baseline.nets.len(), 1);
        let net = &mut baseline.nets[0];
        assert_eq!(net.terminals.len(), 1);
        assert_eq!(net.terminals[0].terminal, "2");
        assert!(net.names.is_empty());
        assert_eq!(net.origin.take(), Some(crate::board::update::NetOrigin::PadOnly));
        baseline.format = DISPLAY_FORMAT.into();
    });
    let params = review_extra_land_removal(&mut session, documents, target);
    assert_apply_refused_without_edit(&mut session, &params, "Cannot remove an accepted connection without explicit net origin.");
}

/// A populated board is not traversed by admission or publication. Encoding yields in record
/// batches, consumes one result, and cannot commit after a library reload or another edit.
#[test]
fn large_board_footprint_revision_job_batches_encoding_and_rechecks_authority() {
    let (mut session, documents, target) = package_fixture(|_| {}, false, true);
    let front = layer(&session, Side::Front);
    const TRACKS: usize = 8192;
    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).tracks = (1..=TRACKS)
        .map(|i| {
            let y_nm = 10_000_000 + 110_000 * i as i64;
            Track {
                id: uuid::Uuid::from_u128(i as u128),
                locked: false,
                layer: front,
                width_nm: 100_000,
                start: crate::schematic::Point { x_nm: 0, y_nm },
                end: crate::schematic::Point { x_nm: 500_000, y_nm },
            }
        })
        .collect();
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    let snapshot = session.snapshot().unwrap();
    let before_history = history(&session);
    let mut abandoned = session.prepare_footprint_revision(&params).unwrap();
    assert!(!abandoned.step(0));
    assert!(!abandoned.step(32), "candidate preparation must yield before encoding");
    assert!(abandoned.take_result().unwrap().is_none());
    drop(abandoned);
    assert_history_unchanged(&session, &snapshot, before_history);
    let prepare = |session: &Session, params: &Value| {
        let mut job = session.prepare_footprint_revision(params).unwrap();
        let steps = (1..=512).find(|_| job.step(32)).expect("8192 tracks and one package must fit 512 worker batches of 32 records");
        assert!(steps > TRACKS / 32, "no worker batch may write more than 32 track records");
        let edit = job.take_result().unwrap().unwrap();
        assert!(job.take_result().unwrap().is_none());
        edit
    };
    let edit = prepare(&session, &params);
    session.execute(REMOVE, &json!({"source":"test"})).unwrap();
    load(&mut session, &documents);
    assert!(session.commit_footprint_revision(edit).unwrap_err().to_string().contains("catalog"));
    assert_history_unchanged(&session, &snapshot, before_history);
    params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    let obsolete = prepare(&session, &params);
    session.apply("project.rename", &json!({"name":"Large board after review"})).unwrap();
    let current = session.snapshot().unwrap();
    let current_history = history(&session);
    assert_eq!(session.commit_footprint_revision(obsolete).unwrap_err().code(), ErrorCode::StaleRevision);
    assert_history_unchanged(&session, &current, current_history);
    params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    let edit = prepare(&session, &params);
    session.commit_footprint_revision(edit).unwrap();
    assert_eq!(session.project().unwrap().boards().next().unwrap().tracks().len(), TRACKS);
}

/// A review is invalidated by both document edits and source refresh; a pad-filtered
/// preview cannot grant review of the whole footprint.
#[test]
fn footprint_revision_refuses_stale_revision_and_catalog_review() {
    let (mut session, documents, target) = package_fixture(|_| {}, false, true);
    let mut filtered = target.clone();
    filtered["pad"] = json!("9");
    assert!(session.execute(PREVIEW_FOOTPRINT_REVISION, &filtered).unwrap()["acceptance"].is_null());
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":"2"});
    params["unconnected_pads"] = json!(["9"]);
    session.apply("project.rename", &json!({"name":"Updated after review"})).unwrap();
    let before = session.snapshot().unwrap();
    let before_history = history(&session);
    let before_inspect = session.inspect();
    assert_eq!(session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap_err().code(), ErrorCode::StaleRevision);
    assert_history_unchanged(&session, &before, before_history);
    assert_eq!(session.inspect()["can_undo"], before_inspect["can_undo"]);
    assert_eq!(session.inspect()["can_redo"], before_inspect["can_redo"]);
    params["acceptance"] = reviewed(&mut session, &target)["acceptance"].clone();
    params["expected_revision"] = json!(session.revision());
    session.execute(REMOVE, &json!({"source":"test"})).unwrap();
    load(&mut session, &documents);
    assert_eq!(session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_history_unchanged(&session, &before, before_history);
    assert_eq!(session.inspect()["can_undo"], before_inspect["can_undo"]);
    assert_eq!(session.inspect()["can_redo"], before_inspect["can_redo"]);
    params["acceptance"] = reviewed(&mut session, &target)["acceptance"].clone();
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    assert_eq!(session.project().unwrap().board.footprints()[0].unconnected_pads, ["9".to_owned()]);
}

/// A renumbered pad requires an explicit replacement identity; the old spelling cannot persist.
#[test]
fn footprint_revision_removes_the_old_pad_mapping_only_with_explicit_resolution() {
    let (mut session, _, target) = fixture(|pad| pad.number = "02".into(), false);
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &target).unwrap();
    let mut params = target;
    params["acceptance"] = report["acceptance"].clone();
    params["expected_revision"] = report["revision"].clone();
    params["mapping"] = report["current_mapping"].clone();
    let before = session.snapshot().unwrap();
    let before_history = history(&session);
    assert_eq!(session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_history_unchanged(&session, &before, before_history);
    params["mapping"] = json!({"1":"1","2":"02"});
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    let placement = &session.project().unwrap().board.footprints()[0];
    assert_eq!(placement.mapping.as_ref().unwrap()["2"].first(), Some("02"));
}

/// A native package is an immutable dependency: consent does not override its declared map.
#[test]
fn footprint_revision_of_native_part_requires_its_declared_map() {
    let (mut session, _, target) = package_fixture(|_| {}, true, true);
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":["1","9"],"2":"2"});
    let before = session.snapshot().unwrap();
    let before_history = history(&session);
    assert!(session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap_err().to_string().contains("declared map"));
    assert_history_unchanged(&session, &before, before_history);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    session.apply(APPLY_FOOTPRINT_REVISION, &params).unwrap();
    let after = session.snapshot().unwrap();
    let selection = after.components.values().next().unwrap().part.as_ref().unwrap();
    assert_eq!(selection.resolved.definition.revision, 2);
    assert_eq!(selection.package.as_deref(), Some("same-package"));
    let bytes = encode_documents(after).unwrap();
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
}

/// A locked placement refuses revision apply without touching project state or history.
#[test]
fn footprint_revision_refuses_locked_placement() {
    let (mut session, _, target) = package_fixture(|_| {}, false, true);
    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).footprints_mut()[0].locked = true;
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    assert_apply_refused_without_edit(&mut session, &params, "Unlock the footprint before applying a revision.");
}

/// Divergent placement and assignment terminal maps refuse revision apply without editing.
#[test]
fn footprint_revision_refuses_mismatched_placement_and_assignment_maps() {
    let (mut session, _, target) = package_fixture(|_| {}, false, true);
    let component = session.snapshot().unwrap().board.footprints()[0].component;
    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).assignments.get_mut(&component).unwrap().mapping =
        crate::board::parse_mapping(&json!({"1":"2","2":"1"})).unwrap();
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    assert_apply_refused_without_edit(&mut session, &params, "Placed footprint and package assignment have different terminal maps.");
}

/// A native part revision that also changes unrelated packages requires a broader review.
#[test]
fn footprint_revision_of_native_part_refuses_unrelated_package_changes() {
    let (mut session, mut documents, target) = package_fixture(|_| {}, true, true);
    let mut part: Value = serde_json::from_str(&documents["parts/part-r2.json"]).unwrap();
    let other = part["packages"]["same-package"].clone();
    part["packages"]["other-package"] = other;
    documents.insert("parts/part-r2.json".into(), part.to_string());
    session.execute(REMOVE, &json!({"source":"test"})).unwrap();
    load(&mut session, &documents);
    let mut params = reviewed(&mut session, &target);
    params["mapping"] = json!({"1":"1","2":["2","9"]});
    assert_apply_refused_without_edit(
        &mut session,
        &params,
        "This native part revision needs a broader review of its dependencies or manufacturer selection.",
    );
}

/// Board replacement guards reject mismatched keys, conflicting pinned content and invalid footprints.
#[test]
fn replace_reviewed_footprint_refuses_key_content_and_bounds_errors() {
    let (session, _, _) = package_fixture(|_| {}, false, true);
    let board = session.snapshot().unwrap().board.as_ref().clone();
    let placement = &board.footprints()[0];
    let assignment = board.assignments[&placement.component].clone();
    let pinned = board.definitions[&assignment.key()].clone();

    let mut wrong_key = assignment.clone();
    wrong_key.definition_revision += 1;
    let mut attempt = board.clone();
    assert_eq!(
        attempt.replace_reviewed_footprint(wrong_key, pinned.clone()).unwrap_err(),
        "Footprint definition differs from its reviewed assignment."
    );
    assert_eq!(attempt, board);

    let mut conflicting = pinned.as_ref().clone();
    conflicting.name.push_str(" conflict");
    let mut attempt = board.clone();
    assert_eq!(
        attempt.replace_reviewed_footprint(assignment.clone(), Arc::new(conflicting)).unwrap_err(),
        "Footprint content conflicts with the project's pinned revision."
    );
    assert_eq!(attempt, board);

    let mut rev2 = pinned.as_ref().clone();
    rev2.revision += 1;
    let mut empty_map = assignment;
    empty_map.definition_revision = rev2.revision;
    empty_map.mapping.clear();
    let mut attempt = board.clone();
    assert_eq!(attempt.replace_reviewed_footprint(empty_map, Arc::new(rev2)).unwrap_err(), "Replacement footprint is outside supported bounds.");
    assert_eq!(attempt, board);
}
