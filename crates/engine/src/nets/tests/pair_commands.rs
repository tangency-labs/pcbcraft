use super::*;

/// Author independent nets through ordinary commands, not hand-edited native files.
fn pair_fixture(board: bool) -> (Session, [Uuid; 3]) {
    let (mut session, pins) = circuit(6, board);
    for pair in pins.chunks_exact(2) {
        connect(&mut session, &pair[0], &pair[1]);
    }
    let ids = [0, 2, 4].map(|index| promote(&mut session, &pins[index]));
    (session, ids)
}

/// Capture the exact serialized project including its current envelope.
fn bytes(session: &Session) -> std::collections::BTreeMap<String, Vec<u8>> {
    encode_documents(session.snapshot().unwrap()).unwrap()
}

/// Creating, editing and removing a pair changes just its relationship; each action can be
/// undone independently, on both the schematic-owned and board-owned registry encodings.
#[test]
fn pair_commands_round_trip_and_one_step_undo() {
    for board in [false, true] {
        let (mut session, [p, n, spare]) = pair_fixture(board);
        let original = session.snapshot().unwrap();
        let initial = bytes(&session);
        let id = new_pair_id().unwrap();
        let name = "λ".repeat(MAX_PAIR_NAME_CHARS);
        let original_revision = session.revision();
        let mut create = json!({"pair":id,"name":name,"positive":p,"negative":n,"expected_revision":original_revision,"dry_run":true});
        let preview = session.execute(CREATE_PAIR, &create).unwrap();
        assert_eq!(preview["pair_change"], json!({"changed":true,"before":null,"after":{"id":id,"name":name,"positive":p,"negative":n}}));
        assert_eq!(session.apply(CREATE_PAIR, &create).unwrap(), original_revision);
        assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
        create["dry_run"] = json!(false);
        session.apply(CREATE_PAIR, &create).unwrap();
        let created = bytes(&session);
        assert_ne!(created, initial);
        round_trip(&session);
        apply(&mut session, "edit.undo", json!({}));
        assert_eq!(bytes(&session), initial);
        apply(&mut session, "edit.redo", json!({}));
        assert_eq!(bytes(&session), created);
        let update = json!({"pair":id,"name":"Retitled","positive":spare,"negative":p});
        apply(&mut session, UPDATE_PAIR, update.clone());
        let updated = bytes(&session);
        let snapshot = session.snapshot().unwrap();
        let revision = session.revision();
        assert_eq!(snapshot.differential_pairs()[0].id, id);
        assert_eq!((snapshot.differential_pairs()[0].positive, snapshot.differential_pairs()[0].negative), (spare, p));
        assert!(Arc::ptr_eq(&snapshot.net_registry.records, &original.net_registry.records));
        assert!(
            session
                .undo
                .iter()
                .flatten()
                .all(|earlier| earlier.differential_pairs().is_empty() || Arc::ptr_eq(&earlier.net_registry.records, &original.net_registry.records))
        );
        assert!(Arc::ptr_eq(&snapshot.sheets, &original.sheets));
        assert!(Arc::ptr_eq(&snapshot.board, &original.board));
        apply(&mut session, UPDATE_PAIR, update);
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &snapshot));
        apply(&mut session, "edit.undo", json!({}));
        assert_eq!(bytes(&session), created);
        apply(&mut session, "edit.redo", json!({}));
        assert_eq!(bytes(&session), updated);
        assert!(Arc::ptr_eq(&session.project().unwrap().net_registry.records, &original.net_registry.records));
        apply(&mut session, REMOVE_PAIR, json!({"pair":id}));
        assert_eq!(bytes(&session), initial);
        apply(&mut session, "edit.undo", json!({}));
        assert_eq!(bytes(&session), updated);
        apply(&mut session, "edit.redo", json!({}));
        assert_eq!(bytes(&session), initial);
    }
}

/// Stale revisions, reused/nil IDs, duplicate/foreign members, and invalid names are atomic
/// refusals. The same errors are reported for previews without consuming redo or changing bytes.
#[test]
fn pair_commands_reject_invalid_inputs_without_history_changes() {
    let (mut session, [p, n, spare]) = pair_fixture(false);
    let id = new_pair_id().unwrap();
    let other = new_pair_id().unwrap();
    apply(&mut session, CREATE_PAIR, json!({"pair":id,"name":"Pair","positive":p,"negative":n}));
    apply(&mut session, UPDATE_PAIR, json!({"pair":id,"name":"Other name","positive":p,"negative":n}));
    apply(&mut session, "edit.undo", json!({}));
    let original = session.snapshot().unwrap();
    let revision = session.revision();
    let before = session.inspect();
    let foreign = pair_fixture(false).1[0];
    let nil = Uuid::nil();
    let base = json!({"pair":other,"name":"Pair","positive":p,"negative":n,"expected_revision":revision});
    let pair_id_error = "an unused, non-nil pair UUID";
    let member_error = "distinct persistent nets";
    let name_error = "Pair names must be trimmed";
    let stale = "project changed";
    let missing = "Unknown differential pair";
    let cases = [
        (CREATE_PAIR, "pair", json!(id), pair_id_error),
        (CREATE_PAIR, "pair", json!(nil), pair_id_error),
        (CREATE_PAIR, "pair", json!(p), pair_id_error),
        (CREATE_PAIR, "negative", json!(p), member_error),
        (CREATE_PAIR, "positive", json!(spare), "at most one differential pair"),
        (CREATE_PAIR, "positive", json!(foreign), member_error),
        (CREATE_PAIR, "negative", json!(nil), member_error),
        (UPDATE_PAIR, "name", json!(""), name_error),
        (UPDATE_PAIR, "name", json!(" leading"), name_error),
        (UPDATE_PAIR, "name", json!("internal\ncontrol"), name_error),
        (UPDATE_PAIR, "name", json!("x".repeat(MAX_PAIR_NAME_CHARS + 1)), name_error),
        (CREATE_PAIR, "expected_revision", json!(revision - 1), stale),
        (UPDATE_PAIR, "pair", json!(other), missing),
        (UPDATE_PAIR, "negative", json!(foreign), member_error),
    ];
    for (command, key, value, expected) in cases {
        let mut params = base.clone();
        if command == UPDATE_PAIR {
            params["pair"] = json!(id);
        }
        params[key] = value;
        let error = session.apply(command, &params).unwrap_err();
        assert!(error.to_string().contains(expected), "{command}: {params}: {error}");
        params["dry_run"] = json!(true);
        let preview_error = session.execute(command, &params).unwrap_err();
        assert_eq!(preview_error, error, "{command}: {params}");
    }
    let absent = session.apply(REMOVE_PAIR, &json!({"pair":other,"expected_revision":revision})).unwrap_err();
    assert_eq!(absent.code().as_str(), "not_found");
    assert!(absent.to_string().contains(missing));
    let expired = session.apply(REMOVE_PAIR, &json!({"pair":id,"expected_revision":revision-1})).unwrap_err();
    assert_eq!(expired.code().as_str(), "stale_revision");
    assert_eq!(expired.details(), json!({"current_revision":revision}));
    assert!(expired.to_string().contains(stale));
    assert_eq!(session.inspect(), before);
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &original));
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(session.project().unwrap().differential_pairs()[0].name, "Other name");
}

/// A legal metadata-heavy schematic already near the native entry-point limit refuses
/// pair growth in both command paths, even though the registry has plenty of room.
#[test]
fn pair_budget_refuses_growth_without_touching_undo_or_redo() {
    use crate::{CommandError, persistence::MAX_DOCUMENT_BYTES, schematic::fields::Field};

    let (mut session, pins) = circuit(36, false);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[2], &pins[3]);
    let p = promote(&mut session, &pins[0]);
    let n = promote(&mut session, &pins[2]);
    let pair = new_pair_id().unwrap();
    let edit = |name: &str| json!({"pair":pair,"name":name,"positive":p,"negative":n});
    apply(&mut session, CREATE_PAIR, edit("A"));
    let mut project = session.project().unwrap().clone();
    for component in Arc::make_mut(&mut project.components).values_mut() {
        component.metadata.fields =
            (1..=64).map(|i| (Uuid::from_u128(i).to_string(), Field { name: format!("Test field {i}"), value: String::new() })).collect();
    }
    let base = encode_documents(Arc::new(project.clone())).unwrap();
    let mut padding = MAX_DOCUMENT_BYTES - base[ENTRY_POINT].len() - 32;
    for component in Arc::make_mut(&mut project.components).values_mut() {
        for field in component.metadata.fields.values_mut() {
            let chunk = padding.min(2048 * 4);
            field.value = "😀".repeat(chunk / 4);
            field.value.push_str(["", "x", "λ", "界"][chunk % 4]);
            padding -= chunk;
        }
        component.metadata.validate().unwrap();
    }
    assert_eq!(padding, 0);
    let docs = encode_documents(Arc::new(project)).unwrap();
    assert_eq!(MAX_DOCUMENT_BYTES - docs[ENTRY_POINT].len(), 32);
    session.open_snapshot(decode_documents(&docs).unwrap());
    apply(&mut session, UPDATE_PAIR, edit("AB"));
    apply(&mut session, UPDATE_PAIR, edit("ABC"));
    apply(&mut session, "edit.undo", json!({}));
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let [undo, redo] = [session.undo.len(), session.redo.len()];
    assert!(undo > 0 && redo > 0);
    let mut small = edit("ABCD");
    small["expected_revision"] = json!(revision);
    small["dry_run"] = json!(true);
    assert_eq!(session.execute(UPDATE_PAIR, &small).unwrap()["pair_change"]["changed"], true);
    let mut large = edit(&"😀".repeat(MAX_PAIR_NAME_CHARS));
    large["expected_revision"] = json!(revision);
    let error = session.apply(UPDATE_PAIR, &large).unwrap_err();
    assert!(matches!(&error, CommandError::CapExceeded(message) if message.contains("size limit")));
    large["dry_run"] = json!(true);
    assert_eq!(session.execute(UPDATE_PAIR, &large).unwrap_err(), error);
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &snapshot));
    assert_eq!([session.undo.len(), session.redo.len()], [undo, redo]);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(session.project().unwrap().differential_pairs()[0].name, "ABC");
    assert!(encode_documents(session.snapshot().unwrap()).is_ok());
}

/// The first and last pair require the native probe; neither preparation changes history.
/// Publication must still reject a different snapshot at the very same revision.
#[test]
fn pair_commands_cold_preparation_is_batched_and_snapshot_bound() {
    let (mut session, [p, n, _]) = pair_fixture(false);
    let pair = new_pair_id().unwrap();
    let create = json!({"pair":pair,"name":"First","positive":p,"negative":n,"expected_revision":session.revision()});
    let revision = session.revision();
    let document = session.snapshot().unwrap();
    let undo_count = session.undo.len();
    let mut create_job = session.prepare_pair_edit(CREATE_PAIR, &create).unwrap();
    assert!(!create_job.step(0));
    assert!(!create_job.step(1));
    assert!(!create_job.step(1));
    assert!(create_job.take_result().unwrap().is_none());
    for _ in 0..1000 {
        if create_job.step(1) {
            break;
        }
    }
    let prepared = create_job.take_result().unwrap().expect("cold probe completed");
    assert!(Arc::ptr_eq(&document, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.undo.len(), undo_count);
    session.project = Some(Arc::new((*document).clone()));
    assert_eq!(session.complete_pair_edit(prepared, false).unwrap_err().code().as_str(), "stale_revision");
    assert_eq!(session.revision(), revision);
    assert!(session.project().unwrap().differential_pairs().is_empty());
    apply(&mut session, CREATE_PAIR, create);
    let revision = session.revision();
    let undo_count = session.undo.len();
    let paired = session.snapshot().unwrap();
    let mut remove_job = session.prepare_pair_edit(REMOVE_PAIR, &json!({"pair":pair,"expected_revision":revision})).unwrap();
    assert!(!remove_job.step(1));
    assert!(!remove_job.step(1));
    assert!(remove_job.take_result().unwrap().is_none());
    for _ in 0..1000 {
        if remove_job.step(1) {
            break;
        }
    }
    let prepared = remove_job.take_result().unwrap().expect("last-pair probe completed");
    assert!(Arc::ptr_eq(&paired, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.undo.len(), undo_count);
    session.complete_pair_edit(prepared, false).unwrap();
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(session.undo.len(), undo_count + 1);
    assert!(session.project().unwrap().differential_pairs().is_empty());
}

/// Large history fields in even one record must advance independently, preserving
/// the exact compact registry and native row bytes of the Serde representation.
#[test]
fn large_net_history_row_retains_exact_serde_bytes() {
    let (session, _) = pair_fixture(true);
    let project = session.snapshot().unwrap();
    let mut row = project.net_registry.records[0].clone();
    let snapshot = crate::board::update::capture_source(&project).unwrap();
    let identities: Vec<_> = (1..=128).map(Uuid::from_u128).collect();
    row.lineage = identities.iter().map(|&id| Lineage { snapshot: snapshot.clone(), net: Some(id), anchor: row.primary.clone() }).collect();
    for state in [
        State::Active {},
        State::Ambiguous { snapshot: snapshot.clone(), branches: identities.clone() },
        State::Retired { snapshot, reason: Retirement::Merged, successors: identities.clone() },
    ] {
        row.state = state;
        let mut cursor = crate::nets::RecordEncoding::default();
        let mut bytes = Vec::new();
        let mut steps = 0;
        while !cursor.step(&mut bytes, &row).unwrap() {
            steps += 1;
            assert!(steps < 512);
        }
        assert!(steps > identities.len());
        assert_eq!(bytes, serde_json::to_vec(&row).unwrap());
        let registry = Registry { records: Arc::new(vec![row.clone()]), pairs: Arc::default() };
        assert_eq!(PairEditJob::final_registry_sizes(&registry).unwrap(), (serde_json::to_vec(&registry).unwrap().len(), 0));
    }
}

/// A large board registry cannot be serialized in one editor job step. A canceled sizing
/// attempt leaves the original snapshot and history intact, while a fresh attempt still commits.
#[test]
fn large_board_pair_registry_sizing_respects_the_poll_budget() {
    const RETIRED: usize = 512;
    const ROWS_PER_POLL: usize = 7;
    let (mut session, [p, n, _]) = pair_fixture(true);
    let mut project = (*session.snapshot().unwrap()).clone();
    let capture = crate::board::update::capture_source(&project).unwrap();
    let records = Arc::make_mut(&mut Arc::make_mut(&mut project.net_registry).records);
    let template = records[0].clone();
    let final_number = records.iter().filter_map(|r| r.generated_number).max().unwrap();
    records.extend((1..=RETIRED).map(|id| Record {
        id: Uuid::from_u128(id as u128),
        generated_number: Some(final_number + id as u32),
        state: State::Retired { snapshot: capture.clone(), reason: Retirement::Removed, successors: Vec::new() },
        ..template.clone()
    }));
    records.sort_by_key(|r| r.id);
    session.open_snapshot(decode_documents(&encode_documents(Arc::new(project)).unwrap()).unwrap());
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let undo = session.undo.len();
    let size = PairEditJob::final_registry_sizes(&snapshot.net_registry).unwrap();
    assert_eq!(size, (serde_json::to_vec(&snapshot.net_registry).unwrap().len(), 0));
    let pair = new_pair_id().unwrap();
    let params = json!({"pair":pair,"name":"Quoted \"Data\" λ","positive":p,"negative":n,"expected_revision":revision});
    let mut job = session.prepare_pair_edit(CREATE_PAIR, &params).unwrap();
    assert!(!job.step(ROWS_PER_POLL));
    assert_eq!(job.drafted_pair_rows(), Some(0));
    assert!(!job.step(ROWS_PER_POLL));
    assert_eq!(job.measured_registry_rows(), Some(0));
    for poll in 1..=RETIRED / ROWS_PER_POLL {
        assert!(!job.step(ROWS_PER_POLL));
        assert_eq!(job.measured_registry_rows(), Some(poll * ROWS_PER_POLL));
        assert!(job.take_result().unwrap().is_none());
    }
    drop(job);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len()), (revision, undo));
    session.apply(CREATE_PAIR, &params).unwrap();
    assert_eq!((session.revision(), session.undo.len()), (revision + 1, undo + 1));
    let current = session.snapshot().unwrap();
    assert_eq!(current.differential_pairs()[0].id, pair);
    let (compact, framed) = PairEditJob::final_registry_sizes(&current.net_registry).unwrap();
    assert_eq!(compact, serde_json::to_vec(&current.net_registry).unwrap().len());
    let home = current.instances_document_path().unwrap();
    let docs = bytes(&session);
    let encoded = std::str::from_utf8(&docs[&home]).unwrap();
    let pair_rows = encoded.split_once("\"pairs\": [\n").unwrap().1.split_once("    ]\n").unwrap().0;
    assert_eq!(framed, pair_rows.len());
    round_trip(&session);
}

/// Candidate construction copies and validates only its allotted pair rows, including when
/// an insertion shifts the entire array. Cancellation never replaces the original list.
#[test]
fn large_pair_draft_respects_the_poll_budget() {
    const PAIRS: usize = 384;
    const ROWS_PER_POLL: usize = 11;
    let (mut session, [p, n, _]) = pair_fixture(false);
    let mut project = Project::clone(session.project().unwrap());
    let capture = crate::board::update::capture_source(&project).unwrap();
    let registry = Arc::make_mut(&mut project.net_registry);
    let records = Arc::make_mut(&mut registry.records);
    let template = records[0].clone();
    let last = records.iter().filter_map(|record| record.generated_number).max().unwrap();
    records.extend((1..=PAIRS * 2).map(|index| Record {
        id: Uuid::from_u128(index as u128),
        generated_number: Some(last + index as u32),
        state: State::Retired { snapshot: capture.clone(), reason: Retirement::Removed, successors: Vec::new() },
        ..template.clone()
    }));
    records.sort_by_key(|record| record.id);
    registry.pairs = Arc::new(
        (0..PAIRS)
            .map(|index| {
                Arc::new(Pair {
                    id: Uuid::from_u128((PAIRS * 2 + index + 2) as u128),
                    name: format!("Retained pair {index}"),
                    positive: Uuid::from_u128((2 * index + 1) as u128),
                    negative: Uuid::from_u128((2 * index + 2) as u128),
                    state: PairState::default(),
                })
            })
            .collect(),
    );
    session.open_snapshot(decode_documents(&encode_documents(Arc::new(project)).unwrap()).unwrap());
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let pair = Uuid::from_u128((PAIRS * 2 + 1) as u128);
    let params = json!({"pair":pair,"name":"First by UUID","positive":p,"negative":n,"expected_revision":revision});
    let mut job = session.prepare_pair_edit(CREATE_PAIR, &params).unwrap();
    assert!(!job.step(ROWS_PER_POLL));
    assert_eq!(job.drafted_pair_rows(), Some(0));
    for poll in 1..=PAIRS / ROWS_PER_POLL {
        assert!(!job.step(ROWS_PER_POLL));
        assert_eq!(job.drafted_pair_rows(), Some(poll * ROWS_PER_POLL));
    }
    drop(job);
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    session.apply(CREATE_PAIR, &params).unwrap();
    let created = session.snapshot().unwrap();
    assert_eq!(created.differential_pairs()[0].id, pair);
    assert!(created.differential_pairs()[1..].iter().zip(snapshot.differential_pairs()).all(|(new, old)| Arc::ptr_eq(new, old)));
    apply(&mut session, REMOVE_PAIR, json!({"pair":pair}));
    assert_eq!(session.snapshot().unwrap().differential_pairs(), snapshot.differential_pairs());
}

/// A proof covers only the exact authored inputs to ordinary registry validation, not an
/// arbitrary project Arc with the same IDs. A changed net record is checked anew on a miss.
#[test]
fn pair_capacity_reuses_only_exact_net_record_proof() {
    let (mut session, [p, n, _]) = pair_fixture(false);
    let pair = new_pair_id().unwrap();
    session.open_snapshot(decode_documents(&bytes(&session)).unwrap());
    let original = session.snapshot().unwrap();
    assert!(original.net_registry.has_records_proof(&original));
    apply(&mut session, CREATE_PAIR, json!({"pair":pair,"name":"New","positive":p,"negative":n}));
    let candidate = session.snapshot().unwrap();
    assert!(candidate.net_registry.has_records_proof(&candidate));
    let mut invalid = Project::clone(&candidate);
    Arc::make_mut(&mut Arc::make_mut(&mut invalid.net_registry).records)[0].id = Uuid::nil();
    assert!(!invalid.net_registry.has_records_proof(&invalid));
    assert!(invalid.net_registry.require_records_proof(&invalid).unwrap_err().contains("validated source"));
    assert!(invalid.net_registry.validate(&invalid).unwrap_err().contains("non-nil UUIDs"));
    let mut changed_hierarchy = Project::clone(&candidate);
    Arc::make_mut(&mut changed_hierarchy.sheets)[0].sheet_name.push_str(" B");
    assert!(changed_hierarchy.net_registry.require_records_proof(&changed_hierarchy).unwrap_err().contains("validated source"));
    changed_hierarchy.net_registry.validate(&changed_hierarchy).unwrap();
    assert!(changed_hierarchy.net_registry.require_records_proof(&changed_hierarchy).is_ok());

    // Opening an older current-layout file may add paper. That representation-only
    // change must not force the next Pair poll to rebuild its graph or reject the open.
    let mut missing_paper = Project::clone(&candidate);
    Arc::make_mut(&mut missing_paper.sheets)[0].page = None;
    let decoded = decode_documents(&crate::persistence::encode_documents(Arc::new(missing_paper)).unwrap()).unwrap();
    assert!(decoded.sheets[0].page.is_none());
    let opened = crate::persistence::prepare_opened(decoded).unwrap();
    assert!(opened.sheets[0].page.is_some());
    assert!(opened.net_registry.require_records_proof(&opened).is_ok());
}

/// An explicitly recorded review reason survives member swaps and naming edits.
#[test]
fn pair_update_preserves_retained_review_state() {
    let (mut session, [p, n, _]) = pair_fixture(false);
    let id = new_pair_id().unwrap();
    apply(&mut session, CREATE_PAIR, json!({"pair":id,"name":"Original","positive":p,"negative":n}));
    let mut project = (*session.snapshot().unwrap()).clone();
    let pairs = &mut Arc::make_mut(&mut project.net_registry).pairs;
    let reason = PairState::NeedsReview { reason: "Retained review".into() };
    Arc::make_mut(&mut Arc::make_mut(pairs)[0]).state = reason.clone();
    let project = decode_documents(&encode_documents(Arc::new(project)).unwrap()).unwrap();
    session.open_snapshot(project);
    apply(&mut session, UPDATE_PAIR, json!({"pair":id,"name":"Changed","positive":n,"negative":p}));
    assert_eq!(session.project().unwrap().differential_pairs()[0].state, reason);
    round_trip(&session);
}
