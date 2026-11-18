use super::*;
use crate::persistence::{Documents, DocumentsEncoder};

/// Build exactly two named nets through shared commands, then stage typed relationship intent
/// through the canonical transaction boundary; pair authoring commands are a separate workflow.
fn paired(board: bool) -> (Session, Pair, String) {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":board}));
    for (index, name) in ["Signal_P", "Signal_N"].iter().enumerate() {
        apply(&mut session, crate::net_labels::ADD, json!({"name":name,"style":"local","x_nm":index as i64*25_400_000,"y_nm":0}));
    }
    let project = session.project().unwrap();
    let labels = project.net_labels();
    let positive = id_for(&session, &json!({"net_label":labels[0].object_id()}));
    let negative = id_for(&session, &json!({"net_label":labels[1].object_id()}));
    let label = labels[0].object_id();
    let pair = Pair { id: Uuid::from_u128(9000), positive, negative, name: "Signal".into(), state: PairState::Active {} };
    let mut project = project.clone();
    Arc::make_mut(&mut project.net_registry).pairs = Arc::new(vec![Arc::new(pair.clone())]);
    session.change(project).unwrap();
    (session, pair, label)
}

/// Locate registry data without inventing a second ownership path.
fn stored_registry(value: &mut Value) -> &mut Value {
    if value.get("schematic_instances").is_some() { &mut value["schematic_instances"]["net_registry"] } else { &mut value["net_registry"] }
}

/// Replace one generated native document for negative codec tests; originals stay untouched.
fn replaced(docs: &Documents, path: &str, value: &Value) -> Documents {
    let mut bad = docs.clone();
    bad.insert(path.into(), serde_json::to_vec(value).unwrap());
    bad
}

/// Both storage homes and archives retain explicit polarity, identity and deterministic bytes
/// across a member rename, a no-op save and ordinary Undo/Redo.
#[test]
fn pair_save_reopen_and_net_rename_preserve_identity_and_bytes() {
    for board in [false, true] {
        let (mut session, pair, label) = paired(board);
        assert_eq!(session.project().unwrap().net_registry.records.len(), 2);
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let path = session.project().unwrap().instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let mut stored: Value = serde_json::from_slice(&before[&path]).unwrap();
        assert!(stored_registry(&mut stored)["pairs"][0].get("state").is_none());
        assert_eq!(serde_json::from_slice::<Value>(&before[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/45");
        if board {
            assert_eq!(stored["format"], "pcbcraft-instances/8");
        }
        assert_eq!(session.project().unwrap().inspect_net_registry().unwrap()["pairs"], json!([pair]));
        round_trip(&session);
        let packed = encode_packed(session.snapshot().unwrap()).unwrap();
        assert_eq!(encode_packed(decode_packed(&packed).unwrap().project).unwrap(), packed);
        apply(&mut session, crate::net_labels::SET, json!({"id":label,"name":"Renamed","style":"local","x_nm":0,"y_nm":0}));
        assert_eq!(session.project().unwrap().net_registry.pairs.as_ref(), &[Arc::new(pair.clone())]);
        let after = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(before[&path], after[&path]);
        round_trip(&session);
        apply(&mut session, "edit.undo", json!({}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        apply(&mut session, "edit.redo", json!({}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
        let mut encoder = DocumentsEncoder::new(session.snapshot().unwrap());
        let incremental = loop {
            if let Some(documents) = encoder.step(1).unwrap() {
                break documents;
            }
        };
        assert_eq!(incremental, after);
    }
}

/// Review state carries bounded context, while an explicit active state canonicalizes to omission.
#[test]
fn pair_review_state_round_trips_and_active_is_omitted() {
    for board in [false, true] {
        let (session, _, _) = paired(board);
        let mut project = session.project().unwrap().clone();
        let pair = Arc::make_mut(&mut Arc::make_mut(&mut project.net_registry).pairs);
        Arc::make_mut(&mut pair[0]).state = PairState::NeedsReview { reason: "Member endpoints require review".into() };
        let docs = encode_documents(Arc::new(project)).unwrap();
        let reopened = decode_documents(&docs).unwrap();
        assert!(matches!(&reopened.net_registry.pairs[0].state, PairState::NeedsReview { reason } if reason == "Member endpoints require review"));
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        let active = encode_documents(session.snapshot().unwrap()).unwrap();
        let path = session.project().unwrap().instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let mut value: Value = serde_json::from_slice(&active[&path]).unwrap();
        stored_registry(&mut value)["pairs"][0]["state"] = json!({"kind":"active"});
        assert_eq!(encode_documents(decode_documents(&replaced(&active, &path, &value)).unwrap()).unwrap(), active);
    }
}

/// Required fields, state vocabulary, text limits and exact scoped membership are independently
/// refused with the relevant error reason, not merely an earlier unrelated validation failure.
#[test]
fn pair_codec_rejects_malformed_records_with_specific_reasons() {
    for board in [false, true] {
        let (session, pair, _) = paired(board);
        let docs = encode_documents(session.snapshot().unwrap()).unwrap();
        let path = session.project().unwrap().instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let original: Value = serde_json::from_slice(&docs[&path]).unwrap();
        let canonical = json!(pair);
        for field in ["id", "positive", "negative", "name"] {
            for value in [None, Some(Value::Null), Some(json!(42)), Some(json!({}))] {
                let mut record = canonical.clone();
                match value {
                    None => {
                        record.as_object_mut().unwrap().remove(field);
                    }
                    Some(value) => record[field] = value,
                }
                let mut stored = original.clone();
                stored_registry(&mut stored)["pairs"] = json!([record]);
                let error = decode_documents(&replaced(&docs, &path, &stored)).unwrap_err();
                assert!(error.contains("missing field") || error.contains("invalid type"), "{field}: {error}");
            }
        }
        let mut cases = Vec::new();
        for (field, value, reason) in [
            ("id", json!(Uuid::nil()), "unique non-nil UUIDs"),
            ("id", json!(pair.positive), "distinct from net identities"),
            ("positive", json!(pair.negative), "distinct persistent nets"),
            ("positive", json!(Uuid::from_u128(8888)), "owning registry"),
            ("negative", json!(Uuid::nil()), "owning registry"),
            ("id", json!("not-a-uuid"), "UUID"),
            ("extra", json!(true), "unknown field"),
            ("name", json!(""), "Pair names"),
            ("name", json!(" padded"), "Pair names"),
            ("name", json!("padded "), "Pair names"),
            ("name", json!("line\nbreak"), "Pair names"),
            ("name", json!("界".repeat(MAX_PAIR_NAME_CHARS + 1)), "Pair names"),
            ("state", Value::Null, "invalid type"),
            ("state", json!("active"), "invalid type"),
            ("state", json!(["active"]), "a differential pair state object"),
            ("state", json!(["needs_review", "Check endpoints"]), "a differential pair state object"),
            ("state", json!({"kind":"future"}), "unknown variant"),
            ("state", json!({"kind":"active","extra":true}), "unknown field"),
            ("state", json!({"kind":"needs_review"}), "missing field"),
            ("state", json!({"kind":"needs_review","reason":null}), "invalid type"),
            ("state", json!({"kind":"needs_review","reason":42}), "invalid type"),
            ("state", json!({"kind":"needs_review","reason":"okay","extra":true}), "unknown field"),
        ] {
            let mut record = canonical.clone();
            record[field] = value;
            cases.push((json!([record]), reason));
        }
        for reason in ["".into(), " padded".into(), "padded ".into(), "line\nbreak".into(), "界".repeat(MAX_PAIR_NAME_CHARS + 1)] {
            let mut record = canonical.clone();
            record["state"] = json!({"kind":"needs_review","reason":reason});
            cases.push((json!([record]), "Pair review reasons"));
        }
        let mut second = canonical.clone();
        second["id"] = json!(Uuid::from_u128(9001));
        cases.extend([
            (json!([canonical.clone(), canonical.clone()]), "unique non-nil UUIDs"),
            (json!([second.clone(), canonical.clone()]), "unique non-nil UUIDs"),
            (json!([canonical.clone(), second]), "at most one differential pair"),
            (json!([[pair.id, pair.positive, pair.negative, pair.name]]), "a differential pair object"),
            (json!([null]), "a differential pair object"),
            (json!([]), "omitted or nonempty and bounded"),
            (Value::Null, "invalid type"),
            (json!({}), "invalid type"),
            (json!(vec![canonical; MAX_PAIRS + 1]), "omitted or nonempty and bounded"),
        ]);
        for (pairs, reason) in cases {
            let mut value = original.clone();
            stored_registry(&mut value)["pairs"] = pairs;
            let error = decode_documents(&replaced(&docs, &path, &value)).unwrap_err();
            assert!(error.contains(reason), "expected {reason}: {error}");
        }
        assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    }
}

/// Pair-capable envelopes require actual pairs; prior supported project and instance envelopes
/// refuse the extension, while removing the last pair restores the original generated-net bytes.
#[test]
fn pair_envelopes_are_lazy_and_refuse_mismatched_vocabulary() {
    for board in [false, true] {
        let (session, _, _) = paired(board);
        let project = session.snapshot().unwrap();
        let docs = encode_documents(project.clone()).unwrap();
        let path = project.instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let original: Value = serde_json::from_slice(&docs[&path]).unwrap();
        let mut missing = original.clone();
        stored_registry(&mut missing).as_object_mut().unwrap().remove("pairs");
        let error = decode_documents(&replaced(&docs, &path, &missing)).unwrap_err();
        assert!(error.contains("pair envelope requires a nonempty pair list"), "{error}");
        for (project_format, instances_format) in [("pcbcraft-project/23", "pcbcraft-instances/4"), ("pcbcraft-project/17", "pcbcraft-instances/2")] {
            let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
            entry["format"] = json!(project_format);
            let mut downgraded = replaced(&docs, ENTRY_POINT, &entry);
            if board {
                let mut instance = original.clone();
                instance["format"] = json!(instances_format);
                downgraded = replaced(&downgraded, &path, &instance);
            }
            let error = decode_documents(&downgraded).unwrap_err();
            assert!(error.contains("Differential pairs require project format"), "{error}");
        }
        if board {
            for version in [
                "pcbcraft-instances/1",
                "pcbcraft-instances/4",
                "pcbcraft-instances/5",
                "pcbcraft-instances/6",
                "pcbcraft-instances/7",
                "pcbcraft-instances/9",
            ] {
                let mut instance = original.clone();
                instance["format"] = json!(version);
                let error = decode_documents(&replaced(&docs, &path, &instance)).unwrap_err();
                assert!(error.contains("unsupported instances format"), "{error}");
            }
            let mut foreign = original.clone();
            foreign["board"] = json!(Uuid::nil());
            let error = decode_documents(&replaced(&docs, &path, &foreign)).unwrap_err();
            assert!(error.contains("foreign board header"), "{error}");
            let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
            let mut duplicate = original.clone();
            duplicate.as_object_mut().unwrap().remove("format");
            duplicate.as_object_mut().unwrap().remove("board");
            entry["schematic_instances"] = duplicate;
            let error = decode_documents(&replaced(&docs, ENTRY_POINT, &entry)).unwrap_err();
            assert!(error.contains("exactly one schematic or registered-board storage home"), "{error}");
        }
        let mut without = (*project).clone();
        Arc::make_mut(&mut without.net_registry).pairs = Arc::default();
        let lower = encode_documents(Arc::new(without)).unwrap();
        let entry: Value = serde_json::from_slice(&lower[ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/23");
        let mut stored: Value = serde_json::from_slice(&lower[&path]).unwrap();
        if board {
            assert_eq!(stored["format"], "pcbcraft-instances/4");
        }
        assert!(stored_registry(&mut stored).get("pairs").is_none());
        let lower_project = decode_documents(&lower).unwrap();
        assert!(lower_project.inspect_net_registry().unwrap().get("pairs").is_none());
        assert_eq!(encode_documents(lower_project).unwrap(), lower);
    }
}

/// First-board creation moves the same logical relationship, without changing either member ID.
#[test]
fn pair_first_board_transfer_preserves_relationship_and_undo() {
    let (mut session, pair, _) = paired(false);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    apply(&mut session, crate::board::identity::CREATE_BOARD, json!({}));
    assert_eq!(session.project().unwrap().net_registry.pairs.as_ref(), &[Arc::new(pair)]);
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Deleting a member's last anchor retires the net without retargeting the pair or setting review state.
#[test]
fn pair_keeps_retired_member_and_reuses_the_relationship_through_history() {
    for board in [false, true] {
        let (mut session, pair, label) = paired(board);
        let original = session.snapshot().unwrap();
        let before = encode_documents(original.clone()).unwrap();
        apply(&mut session, crate::net_labels::DELETE, json!({"id":label}));
        let current = session.snapshot().unwrap();
        assert_eq!(*current.net_registry.pairs[0], pair);
        assert!(Arc::ptr_eq(&original.net_registry.pairs, &current.net_registry.pairs));
        assert!(matches!(current.net_registry.record(pair.positive).unwrap().state, State::Retired { reason: Retirement::Removed, .. }));
        assert!(matches!(current.net_registry.record(pair.negative).unwrap().state, State::Active {}));
        round_trip(&session);
        let after = encode_documents(current).unwrap();
        apply(&mut session, "edit.undo", json!({}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        apply(&mut session, "edit.redo", json!({}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
    }
}

/// The last valid pair/name/reason sizes round-trip, the next sizes fail, and copying a registry
/// shares both the pair list and pair values instead of copying their text into every Undo step.
#[test]
fn pair_resource_edges_are_bounded_and_history_shares_records() {
    let (session, pair, _) = paired(false);
    let mut project = session.project().unwrap().clone();
    let template = project.net_registry.records[0].clone();
    let snapshot = crate::board::update::capture_source(&project).unwrap();
    let registry = Arc::make_mut(&mut project.net_registry);
    registry.records = (1..=MAX_RECORDS as u128)
        .map(|id| Record {
            id: Uuid::from_u128(id),
            generated_number: Some(id as u32),
            state: State::Retired { snapshot: snapshot.clone(), reason: Retirement::Removed, successors: vec![] },
            ..template.clone()
        })
        .collect();
    registry.pairs = Arc::new(
        (0..MAX_PAIRS)
            .map(|index| {
                Arc::new(Pair {
                    id: Uuid::from_u128(10_000 + index as u128),
                    positive: Uuid::from_u128(2 * index as u128 + 1),
                    negative: Uuid::from_u128(2 * index as u128 + 2),
                    name: "界".repeat(MAX_PAIR_NAME_CHARS),
                    state: PairState::NeedsReview { reason: "界".repeat(MAX_PAIR_NAME_CHARS) },
                })
            })
            .collect(),
    );
    let copy = registry.clone();
    assert!(Arc::ptr_eq(&copy.pairs, &registry.pairs));
    assert!(Arc::ptr_eq(&copy.pairs[0], &registry.pairs[0]));
    registry.validate_pairs().unwrap();
    let documents = encode_documents(Arc::new(project.clone())).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    Arc::make_mut(&mut Arc::make_mut(&mut project.net_registry).pairs).push(Arc::new(pair));
    assert_eq!(project.net_registry.validate_pairs().unwrap_err(), "Differential pair count exceeds its limit.");
}

/// Add historical records without taking the two live networks out of the fixture.
fn fill_with_retired_nets(project: &mut crate::Project, count: usize) -> crate::board::update::SourceCapture {
    let snapshot = crate::board::update::capture_source(project).unwrap();
    let registry = Arc::make_mut(&mut project.net_registry);
    let template = registry.records[0].clone();
    let occupied = registry.reserved_ids();
    let last_number = registry.records.iter().filter_map(|record| record.generated_number).max().unwrap();
    let missing = count - registry.records.len();
    registry.records.extend((1..).map(Uuid::from_u128).filter(|id| !occupied.contains(id)).take(missing).enumerate().map(|(index, id)| Record {
        id,
        generated_number: Some(last_number + index as u32 + 1),
        state: State::Retired { snapshot: snapshot.clone(), reason: Retirement::Removed, successors: vec![] },
        ..template.clone()
    }));
    registry.records.sort_by_key(|record| record.id);
    snapshot
}

/// Pair IDs reserve UUIDs, not net slots: the last legal named net fits, one more does not.
#[test]
fn pair_ids_do_not_consume_net_record_capacity() {
    let (mut session, pair, _) = paired(false);
    let mut project = session.project().unwrap().clone();
    fill_with_retired_nets(&mut project, MAX_RECORDS - 1);
    assert_eq!(project.net_registry.reserved_ids().len(), MAX_RECORDS);
    project.net_registry.validate(&project).unwrap();
    session.open_snapshot(Arc::new(project));
    apply(&mut session, crate::net_labels::ADD, json!({"name":"Last slot","style":"local","x_nm":0,"y_nm":25_400_000}));
    let full = session.snapshot().unwrap();
    assert_eq!(full.net_registry.records.len(), MAX_RECORDS);
    assert_eq!(*full.net_registry.pairs[0], pair);
    let added = id_for(&session, &json!({"net_label":full.net_labels().last().unwrap().object_id()}));
    assert_ne!(added, pair.id);
    let revision = session.revision();
    let error = session
        .apply(
            crate::net_labels::ADD,
            &json!({
                "name":"No slot","style":"local","x_nm":0,"y_nm":50_800_000,"expected_revision":revision
            }),
        )
        .unwrap_err();
    assert!(matches!(error, crate::CommandError::Unavailable(ref reason) if reason == "Net identity registry is full."), "{error:?}");
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&full, &session.snapshot().unwrap()));
}

/// Preserve the exact stack contents as well as the current project and revision on refusal.
fn refuse_candidate_atomically(session: &mut Session, candidate: crate::Project, expected: &str) {
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let (undo, redo) = (session.undo.clone(), session.redo.clone());
    assert!(!undo.is_empty() && !redo.is_empty());
    let error = session.change(candidate).unwrap_err();
    assert!(matches!(error, crate::CommandError::InvalidParams(ref reason) if reason == expected), "{error:?}");
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    for (actual, saved) in [(&session.undo, &undo), (&session.redo, &redo)] {
        assert_eq!(actual.len(), saved.len());
        for (a, b) in actual.iter().zip(saved) {
            assert!(match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            });
        }
    }
}

/// Stage an undone rename so a later atomic refusal has both history stacks to preserve.
fn populate_history(session: &mut Session) {
    apply(session, "project.rename", json!({"name":"Keep"}));
    apply(session, "project.rename", json!({"name":"Redo"}));
    apply(session, "edit.undo", json!({}));
}

/// Do not silently discard an invalid pair mutation while publishing a topology change.
#[test]
fn pair_edit_with_topology_change_refuses_the_whole_candidate() {
    let (mut session, pair, _) = paired(false);
    populate_history(&mut session);
    let mut candidate = session.project().unwrap().clone();
    Arc::make_mut(&mut candidate.sheet_mut().net_labels).remove(0);
    candidate.sheet_mut().connectivity = Arc::default();
    let pairs = Arc::make_mut(&mut Arc::make_mut(&mut candidate.net_registry).pairs);
    Arc::make_mut(&mut pairs[0]).negative = pair.positive;
    refuse_candidate_atomically(&mut session, candidate, "Differential pair edits cannot accompany net topology changes.");
}

/// Legal records and pair text can together exceed the shared byte cap. Exercise both native
/// decoding and unchanged-topology candidate refusal, not just a list or text limit.
#[test]
fn pair_bytes_cross_the_registry_budget_without_publishing_edits() {
    let (mut session, _, _) = paired(false);
    let mut project = session.project().unwrap().clone();
    let snapshot = fill_with_retired_nets(&mut project, MAX_RECORDS);
    let registry = Arc::make_mut(&mut project.net_registry);
    registry.pairs = Arc::default();
    let retired_ids: Vec<_> = registry.records.iter().filter(|record| matches!(record.state, State::Retired { .. })).map(|r| r.id).collect();
    let mut bytes = serde_json::to_vec(registry).unwrap().len();
    let cap_before_pairs = MAX_REGISTRY_BYTES - 4096;
    for record in registry.records.iter_mut().filter(|record| matches!(record.state, State::Retired { .. })) {
        for object in 1..=8 {
            let source = Lineage { snapshot: snapshot.clone(), net: None, anchor: Anchor { object, ..record.primary.clone() } };
            let extra = serde_json::to_vec(&source).unwrap().len() + usize::from(!record.lineage.is_empty());
            if bytes + extra > cap_before_pairs {
                break;
            }
            bytes += extra;
            record.lineage.push(source);
        }
    }
    assert_eq!(serde_json::to_vec(registry).unwrap().len(), bytes);
    assert!((cap_before_pairs - 1024..=cap_before_pairs).contains(&bytes));
    project.net_registry.validate(&project).unwrap();
    session.open_snapshot(Arc::new(project));
    populate_history(&mut session);
    let before = session.snapshot().unwrap();
    let mut candidate = (*before).clone();
    let registry = Arc::make_mut(&mut candidate.net_registry);
    registry.pairs = Arc::new(
        retired_ids
            .as_chunks::<2>()
            .0
            .iter()
            .take(8)
            .enumerate()
            .map(|(index, members)| {
                Arc::new(Pair {
                    id: Uuid::from_u128(10_000 + index as u128),
                    positive: members[0],
                    negative: members[1],
                    name: "界".repeat(MAX_PAIR_NAME_CHARS),
                    state: PairState::NeedsReview { reason: "界".repeat(MAX_PAIR_NAME_CHARS) },
                })
            })
            .collect(),
    );
    registry.validate_pairs().unwrap();
    assert!(serde_json::to_vec(registry).unwrap().len() > MAX_REGISTRY_BYTES);
    let docs = encode_documents(before).unwrap();
    let mut stored: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    stored["format"] = json!("pcbcraft-project/45");
    stored_registry(&mut stored)["pairs"] = json!(registry.pairs);
    let error = decode_documents(&replaced(&docs, ENTRY_POINT, &stored)).unwrap_err();
    assert!(error.contains("Net registry exceeds its byte limit."), "{error}");
    refuse_candidate_atomically(&mut session, candidate, "Net registry exceeds its byte limit.");
}

/// Pair envelopes can retain fully unnumbered compatibility registries, but not mixed numbering.
#[test]
fn pair_envelope_preserves_unnumbered_registry_and_rejects_partial_numbers() {
    let (session, _, _) = paired(false);
    let mut project = session.project().unwrap().clone();
    for record in &mut Arc::make_mut(&mut project.net_registry).records {
        record.generated_number = None;
    }
    let documents = encode_documents(Arc::new(project.clone())).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    Arc::make_mut(&mut project.net_registry).records[0].generated_number = Some(1);
    assert_eq!(encode_documents(Arc::new(project)).unwrap_err(), "Numbered net registries require a number on every record.");
    let mut value: Value = serde_json::from_slice(&documents[ENTRY_POINT]).unwrap();
    stored_registry(&mut value)["records"][0]["generated_number"] = json!(1);
    let error = decode_documents(&replaced(&documents, ENTRY_POINT, &value)).unwrap_err();
    assert!(error.contains("with a number on every record"), "{error}");
}

/// Pair-only candidates have no valid members and cannot publish a snapshot, revision or history
/// step, either against an unchanged empty graph or when adopting the first document.
#[test]
fn pair_only_candidates_reject_atomically_with_an_empty_registry() {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({}));
    apply(&mut session, "project.rename", json!({"name":"First"}));
    apply(&mut session, "project.rename", json!({"name":"Second"}));
    apply(&mut session, "edit.undo", json!({}));
    assert!(!session.undo.is_empty() && !session.redo.is_empty());
    let original = session.snapshot().unwrap();
    assert!(original.net_registry.records.is_empty());
    let mut invalid = (*original).clone();
    Arc::make_mut(&mut invalid.net_registry).pairs = Arc::new(vec![Arc::new(Pair {
        id: Uuid::from_u128(9000),
        positive: Uuid::from_u128(9001),
        negative: Uuid::from_u128(9002),
        name: "Missing members".into(),
        state: PairState::default(),
    })]);
    let mut unopened = Session::default();
    for target in [&mut session, &mut unopened] {
        let before = target.project.clone();
        let revision = target.revision();
        let history = (target.undo.len(), target.redo.len());
        let error = target.change(invalid.clone()).unwrap_err();
        assert!(
            matches!(error, crate::CommandError::InvalidParams(ref reason) if reason.contains("distinct persistent nets in the owning registry")),
            "{error:?}"
        );
        assert_eq!(target.revision(), revision);
        assert_eq!((target.undo.len(), target.redo.len()), history);
        if let Some(before) = before {
            assert!(Arc::ptr_eq(&before, &target.snapshot().unwrap()));
        } else {
            assert!(target.project().is_none());
        }
    }
}

/// The pair envelope composes existing class bindings and project parameters without a second
/// catalog, while an invalid relationship candidate publishes no partial transaction.
#[test]
fn pair_envelope_keeps_existing_intent_and_invalid_candidates_are_atomic() {
    for board in [false, true] {
        let (mut session, pair, _) = paired(board);
        apply(&mut session, crate::net_classes::CREATE, json!({"name":"Signal class","rules":{}}));
        let project = session.project().unwrap();
        let class = project.net_classes().definitions[0].id;
        let target = NetAddress { project: project.id, scope: project.net_scope(), net: pair.positive };
        apply(
            &mut session,
            SET_CLASSES,
            json!({"target":{"kind":"persistent","project":target.project,"scope":target.scope,"net":target.net},"classes":[class]}),
        );
        apply(&mut session, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
        round_trip(&session);
        let before = session.snapshot().unwrap();
        let revision = session.revision();
        let mut invalid = (*before).clone();
        let pairs = Arc::make_mut(&mut Arc::make_mut(&mut invalid.net_registry).pairs);
        Arc::make_mut(&mut pairs[0]).negative = pair.positive;
        let error = session.change(invalid).unwrap_err();
        assert!(matches!(error, crate::CommandError::InvalidParams(ref reason) if reason.contains("distinct persistent nets")), "{error:?}");
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}

/// Pair headers coexist with current assembly profiles and both membership encodings. Removing
/// the pairs restores the exact older bytes, and contradictory membership encodings are refused.
#[test]
fn pair_envelopes_keep_profiles_and_both_membership_encodings() {
    use crate::part_groups::{CREATE, SET_GROUPS};

    for board in [false, true] {
        let (mut session, _, _) = paired(board);
        apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":25_400_000}));
        let member = session.project().unwrap().part_instance_addresses().next().unwrap();
        apply(&mut session, CREATE, json!({"name":"Sample group"}));
        let group = session.project().unwrap().part_groups().groups[0].id;
        apply(&mut session, SET_GROUPS, json!({"part_instance":member,"groups":[group]}));
        apply(&mut session, crate::no_connect::SET, json!({"target":{"part":member,"terminal":"2"},"marked":true}));
        if board {
            apply(
                &mut session,
                crate::board::assembly_profile::SET_ASSEMBLY_PROFILE,
                json!({"content":{"name":"Grouped project","revision":1,"provenance":{"kind":"authored"}}}),
            );
        }

        let project = session.snapshot().unwrap();
        let path = project.instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        for compact in [false, true] {
            let mut candidate = (*project).clone();
            candidate.prefer_compact_part_groups = compact;
            let docs = encode_documents(Arc::new(candidate.clone())).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&docs[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/45");
            let stored: Value = serde_json::from_slice(&docs[&path]).unwrap();
            if board {
                assert_eq!(stored["format"], "pcbcraft-instances/8");
            }
            let home = if board { &stored } else { &stored["schematic_instances"] };
            assert_eq!(home.get("part_group_memberships").is_some(), !compact);
            assert_eq!(home.get("compact_part_group_memberships").is_some(), compact);
            let reopened = decode_documents(&docs).unwrap();
            assert_eq!(reopened.assembly_profiles(), candidate.assembly_profiles());
            assert_eq!(reopened.part_groups(), candidate.part_groups());
            assert_eq!(reopened.part_group_memberships(), candidate.part_group_memberships());
            assert_eq!(reopened.no_connect_count(), 1);
            assert_eq!(reopened.net_registry, candidate.net_registry);
            assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);

            if compact {
                let mut mixed = stored.clone();
                let home = if board { &mut mixed } else { &mut mixed["schematic_instances"] };
                home["part_group_memberships"] = json!(candidate.part_group_memberships().iter().collect::<Vec<_>>());
                let error = decode_documents(&replaced(&docs, &path, &mixed)).unwrap_err();
                assert!(error.contains("exactly one membership encoding"), "{error}");
            }
            let without_pairs = |mut project: crate::Project| {
                Arc::make_mut(&mut project.net_registry).pairs = Arc::default();
                encode_documents(Arc::new(project)).unwrap()
            };
            let lower = without_pairs(candidate);
            assert_eq!(without_pairs((*reopened).clone()), lower);
            let earlier_format = match (board, compact) {
                (true, _) => "pcbcraft-project/36",
                (false, true) => "pcbcraft-project/39",
                (false, false) => "pcbcraft-project/24",
            };
            assert_eq!(serde_json::from_slice::<Value>(&lower[ENTRY_POINT]).unwrap()["format"], earlier_format);
            if board {
                assert_eq!(
                    serde_json::from_slice::<Value>(&lower[&path]).unwrap()["format"],
                    if compact { "pcbcraft-instances/7" } else { "pcbcraft-instances/5" }
                );
            }
            assert_eq!(encode_documents(decode_documents(&lower).unwrap()).unwrap(), lower);
        }
    }
}
