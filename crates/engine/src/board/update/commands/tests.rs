use super::*;
use crate::persistence::{decode_documents, decode_packed, encode_documents, encode_packed};
use crate::{
    ASSIGN_FOOTPRINT, CONNECT_PINS, DELETE_SYMBOL, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM,
};

/// Submit edits through the normal shared command and current revision guard.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Begin with real placed packages and an explicitly accepted logical input.
fn initialized() -> Session {
    let mut session = super::super::tests::fixture();
    edit(&mut session, INITIALIZE_UPDATE_BASELINE, json!({}));
    session
}

/// A preview returns review data while leaving authored state and undo untouched.
fn plan(session: &mut Session) -> Value {
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let before = encode_documents(snapshot.clone()).unwrap();
    let result = session.execute(PREVIEW_UPDATE, &json!({"expected_revision":revision})).unwrap();
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    result["board_update_preview"].clone()
}

/// Capability authority is distinct from source incarnation and numeric revision.
#[test]
fn preview_tokens_are_ephemeral_replaced_cancelled_and_not_history() {
    let mut session = initialized();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let old = plan(&mut session);
    let new = plan(&mut session);
    assert_ne!(old["plan_token"], new["plan_token"]);
    assert_ne!(old["source"]["capture_id"], new["source"]["capture_id"]);
    assert_eq!(old["source"]["content_digest"], new["source"]["content_digest"]);
    assert_ne!(new["plan_token"], new["source"]["capture_id"]);
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":old["plan_token"],"expected_revision":session.revision()})).is_err());
    assert_eq!(session.board_update_report().unwrap(), new);
    session.apply(CANCEL_UPDATE, &json!({"plan_token":new["plan_token"]})).unwrap();
    assert!(session.board_update_report().is_none());
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":new["plan_token"],"expected_revision":session.revision()})).is_err());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Equal project IDs, revisions and even the same Arc do not authorize replay in
/// another independently opened session or after Undo/Redo/reopen.
#[test]
fn edits_history_reopen_and_independent_sessions_invalidate_old_capabilities() {
    let mut first = initialized();
    let snapshot = first.snapshot().unwrap();
    let mut second = Session::default();
    let mut third = Session::default();
    second.open_snapshot(snapshot.clone());
    third.open_snapshot(snapshot.clone());
    assert_eq!(second.revision(), third.revision());
    let foreign = plan(&mut second);
    let own = plan(&mut third);
    assert!(third.apply(APPLY_UPDATE, &json!({"plan_token":foreign["plan_token"],"expected_revision":third.revision()})).is_err());
    assert_eq!(third.board_update_report().unwrap(), own);
    let cached = plan(&mut first);
    let footprint = first.project().unwrap().board.footprints()[0].id;
    edit(&mut first, SET_FOOTPRINT_TRANSFORM, json!({"footprint":format!("footprint-{footprint}"),"x_nm":1_000_000,"y_nm":2_000_000,"rotation":90}));
    assert!(first.board_update_report().is_none());
    assert!(first.apply(APPLY_UPDATE, &json!({"plan_token":cached["plan_token"],"expected_revision":first.revision()})).is_err());
    let cached = plan(&mut first);
    first.apply("edit.undo", &json!({})).unwrap();
    first.apply("edit.redo", &json!({})).unwrap();
    assert!(first.board_update_report().is_none());
    assert!(first.apply(APPLY_UPDATE, &json!({"plan_token":cached["plan_token"],"expected_revision":first.revision()})).is_err());
    let cached = plan(&mut first);
    first.open_snapshot(first.snapshot().unwrap());
    assert!(first.apply(APPLY_UPDATE, &json!({"plan_token":cached["plan_token"],"expected_revision":first.revision()})).is_err());
}

/// Successful no-op acceptance consumes authority but preserves bytes, accepted
/// capture identity, snapshot identity, revision and history.
#[test]
fn noop_apply_does_not_persist_preview_capture_or_create_history() {
    let mut session = initialized();
    let snapshot = session.snapshot().unwrap();
    let old_source = snapshot.board.baseline.as_ref().unwrap().source.clone();
    let report = plan(&mut session);
    let revision = session.revision();
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().source, old_source);
    assert!(session.board_update_report().is_none());
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":report["plan_token"],"expected_revision":revision})).is_err());
}

/// Save-name adoption can replace an immutable input without changing numeric
/// revision, so that lifecycle path must also revoke a cached capability.
#[test]
fn saved_name_adoption_invalidates_preview_at_the_same_revision() {
    let mut session = initialized();
    let report = plan(&mut session);
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let renamed = Arc::new(snapshot.renamed("Saved board").unwrap());
    assert!(session.adopt_saved_name(&snapshot, &renamed));
    assert_eq!(session.revision(), revision);
    assert!(session.cached_board_update().is_none());
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":report["plan_token"],"expected_revision":revision})).is_err());
}

/// Pending placed choices persist independently of accepted geometry, then Apply
/// keeps placement ID/pose and adopts exactly the reviewed package/mapping.
#[test]
fn desired_package_roundtrip_then_atomic_apply_preserves_placement_pose() {
    let mut session = initialized();
    let ids: Vec<_> = session.project().unwrap().components.keys().copied().collect();
    let original = session.project().unwrap().board.footprints()[0].clone();
    let replacement = session.project().unwrap().board.assignments[&ids[1]].clone();
    edit(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{}",ids[0]),"library":replacement.library,
        "definition":replacement.definition,"definition_revision":replacement.definition_revision,"mapping":{"1":"2","2":"1"}}),
    );
    assert_eq!(session.project().unwrap().board.footprints()[0].definition, original.definition);
    assert_eq!(session.inspect()["project"]["board"]["assignments"][0]["pending_update"], true);
    let pending = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&pending[&crate::persistence::test_board_path(&pending)]).unwrap().contains("pcbcraft-board/11"));
    let reopened = decode_documents(&pending).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), pending);
    assert_eq!(encode_documents(decode_packed(&encode_packed(reopened.clone()).unwrap()).unwrap().project).unwrap(), pending);
    let mut bad = pending.clone();
    let old_header = std::str::from_utf8(&bad[&crate::persistence::test_board_path(&bad)]).unwrap().replace("pcbcraft-board/11", "pcbcraft-board/10");
    bad.insert(crate::persistence::test_board_path(&bad), old_header.into_bytes());
    assert!(decode_documents(&bad).is_err());
    session.open_snapshot(reopened);
    assert!(session.board_update_report().is_none());
    let report = plan(&mut session);
    let detail = &report["component_details"][format!("component-{}", ids[0])];
    assert_eq!(detail["accepted"]["package"]["definition"], "resistor-0402");
    assert_eq!(detail["proposed"]["package"]["definition"], "resistor-0603");
    let before_apply = session.snapshot().unwrap();
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
    let changed = &session.project().unwrap().board.footprints()[0];
    assert_eq!((changed.id, changed.x_nm, changed.y_nm, changed.rotation), (original.id, original.x_nm, original.y_nm, original.rotation));
    assert_eq!(changed.definition, "resistor-0603");
    assert_eq!(changed.mapping.as_ref().unwrap()["1"], "2");
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().source.capture_id.to_string(), report["source"]["capture_id"]);
    let applied = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&applied[&crate::persistence::test_board_path(&applied)]).unwrap().contains("pcbcraft-board/10"));
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(before_apply).unwrap());
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), applied);
    assert_eq!(encode_documents(decode_documents(&applied).unwrap()).unwrap(), applied);
}

/// Selecting the accepted choice after a pending replacement actually resets
/// intent; an existing footprint match must not short-circuit that authored edit.
#[test]
fn resetting_desired_intent_restores_the_accepted_package_bytes() {
    let mut session = initialized();
    let ids: Vec<_> = session.project().unwrap().components.keys().copied().collect();
    let original = session.project().unwrap().board.assignments[&ids[0]].clone();
    let replacement = session.project().unwrap().board.assignments[&ids[1]].clone();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    for choice in [replacement, original] {
        edit(
            &mut session,
            ASSIGN_FOOTPRINT,
            json!({"component":format!("component-{}",ids[0]),"library":choice.library,
            "definition":choice.definition,"definition_revision":choice.definition_revision,"mapping":choice.mapping}),
        );
    }
    assert!(!session.project().unwrap().board.has_pending_package_intent(session.project().unwrap()));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// First forwarding leaves new parts unplaced; later connectivity and removal
/// changes affect the accepted board only after their current preview is applied.
#[test]
fn fresh_forward_addition_connectivity_and_removal_share_one_apply_path() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    edit(&mut session, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let id = *session.project().unwrap().components.keys().next().unwrap();
    let blocked = plan(&mut session);
    assert_eq!(blocked["can_apply"], false);
    let before = session.snapshot().unwrap();
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":blocked["plan_token"],"expected_revision":session.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let definition = super::super::super::definition::starter("resistor-0402").unwrap();
    edit(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{id}"),"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
    let ready = plan(&mut session);
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":ready["plan_token"]}));
    assert!(session.project().unwrap().board.footprints().is_empty());
    assert!(session.project().unwrap().board.baseline.is_some());
    edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":format!("component-{id}"),"x_nm":0,"y_nm":0}));
    edit(&mut session, PLACE_RESISTOR, json!({"x_nm":10_000_000,"y_nm":0}));
    let new_id = *session.project().unwrap().components.keys().next_back().unwrap();
    edit(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{new_id}"),"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
    assert!(
        session
            .apply(
                PLACE_ASSIGNED_FOOTPRINT,
                &json!({"component":format!("component-{new_id}"),"x_nm":5_000_000,"y_nm":0,"expected_revision":session.revision()})
            )
            .is_err()
    );
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| symbol.id()).collect();
    edit(&mut session, CONNECT_PINS, json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"2"}}));
    assert!(session.project().unwrap().board.baseline.as_ref().unwrap().nets.is_empty());
    let ready = plan(&mut session);
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":ready["plan_token"]}));
    assert_eq!(session.project().unwrap().board.baseline.as_ref().unwrap().nets.len(), 1);
    assert!(!session.project().unwrap().board.by_component.contains_key(&new_id));
    edit(&mut session, DELETE_SYMBOL, json!({"symbol":symbols[0]}));
    assert_eq!(session.project().unwrap().board.footprints().len(), 1);
    let ready = plan(&mut session);
    edit(&mut session, APPLY_UPDATE, json!({"plan_token":ready["plan_token"]}));
    assert!(session.project().unwrap().board.footprints().is_empty());
    assert!(!session.project().unwrap().board.baseline.as_ref().unwrap().components.contains_key(&id));
}

/// Revalidation catches an invalid cached candidate without any partial physical
/// mutation. Runtime cache entries are never client-supplied project documents.
#[test]
fn failed_candidate_validation_is_atomic_and_uninitialized_legacy_is_explicit() {
    let mut legacy = super::super::tests::fixture();
    let before = legacy.snapshot().unwrap();
    assert!(legacy.execute(PREVIEW_UPDATE, &json!({"expected_revision":legacy.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &legacy.snapshot().unwrap()));
    let mut session = initialized();
    let symbol = session.project().unwrap().symbols[0].id();
    edit(&mut session, DELETE_SYMBOL, json!({"symbol":symbol}));
    let report = plan(&mut session);
    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let proposed = &mut session.update_plan.as_mut().unwrap().proposed;
    proposed.components.values_mut().next().unwrap().package.as_mut().unwrap().mapping.insert("1".into(), "INVALID".into());
    assert!(session.apply(APPLY_UPDATE, &json!({"plan_token":report["plan_token"],"expected_revision":revision})).is_err());
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
}

/// Counting serialization stops before oversized complete reports can be cached.
#[test]
fn complete_report_limit_is_enforced_without_retaining_payload() {
    let mut writer = ReportSize { bytes: MAX_PREVIEW_BYTES - 1 };
    writer.write_all(b"x").unwrap();
    assert!(writer.write_all(b"x").is_err());
    assert_eq!(writer.bytes, MAX_PREVIEW_BYTES);
}

mod eligibility;
