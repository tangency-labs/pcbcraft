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
    let mut remove_job = session.prepare_pair_edit(REMOVE_PAIR, &json!({"pair":pair})).unwrap();
    assert!(!remove_job.step(1));
    assert!(remove_job.take_result().unwrap().is_none());
    assert_eq!(session.project().unwrap().differential_pairs()[0].id, pair);
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
