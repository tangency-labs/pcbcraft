use super::{
    RelationStage::Outer,
    capacity_tests::fill_board,
    commands::{self, *},
    probes::{Op, trace},
    tests::{Observe, add, bytes, fixture, origin, pick, run, unchanged},
};
use crate::{Session, board::update, persistence::test_board_path};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

const SOURCE: [Op; 4] = [Op::SourceEncode, Op::SourceDigest, Op::EcoDiff, Op::NativeReportBound];
pub(super) const CHECKED: [Op; 11] = [
    Op::SourceEncode,
    Op::SourceDigest,
    Op::EcoDiff,
    Op::NativeReportBound,
    Op::BuildEco,
    Op::Prepare(Outer),
    Op::VerifyReferences,
    Op::CandidateEncode,
    Op::CandidateDigest,
    Op::CheckedReportBound,
    Op::RetainChecked,
];

/// Observe and compare every event; inspect the same borrowed report used by the native UI.
fn preview(s: &mut Session, args: Value, expected: &[Op]) -> Value {
    let base = s.snapshot().unwrap();
    let state = (s.revision(), s.undo.len(), s.redo.len());
    let (r, ops) = trace(|| run(s, update::PREVIEW_UPDATE, args));
    let r = r.unwrap()["board_update_preview"].clone();
    assert_eq!(ops, expected);
    assert_eq!(s.board_update_report().unwrap(), r);
    assert_eq!(s.cached_board_update().unwrap().can_apply, r["can_apply"].as_bool().unwrap());
    assert_eq!(s.checked_eco_candidate().is_some(), expected.contains(&Op::RetainChecked));
    assert!(Arc::ptr_eq(&base, &s.snapshot().unwrap()));
    assert_eq!((s.revision(), s.undo.len(), s.redo.len()), state);
    r
}

fn token(report: &Value) -> Value {
    json!({"plan_token":report["plan_token"]})
}

fn observe(s: &mut Session) -> Arc<Observe> {
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    observer
}

/// Hold a genuinely nonempty proposal and keep the first related placement present.
fn pending(s: &mut Session) {
    let symbol = s.project().unwrap().symbols.last().unwrap().id();
    run(s, crate::DELETE_SYMBOL, json!({"symbol":symbol})).unwrap();
}

/// Re-review and Cancel inspect a large real native bundle but never construct a candidate.
/// Test native explicit/default board aliases and omitted and explicit empty decisions at Apply.
#[test]
fn a_eco_lazy_large_preview_rereview_cancel_first_apply() {
    let mut s = fixture(true);
    pending(&mut s);
    // Keep the real removed-net report under its separate 8 MiB envelope bound.
    let large_bytes = 4 * 1024 * 1024;
    s.open_snapshot(fill_board(&s, crate::persistence::MAX_DOCUMENT_BYTES - large_bytes));
    let before = bytes(&s);
    assert_eq!(before[&test_board_path(&before)].len(), large_bytes);
    let board = s.project().unwrap().board_target().unwrap().board;
    let observer = observe(&mut s);
    let old = preview(&mut s, json!({"board":board}), &SOURCE);
    let live = preview(&mut s, json!({"remap_choices":[]}), &SOURCE);
    assert_ne!(old["plan_token"], live["plan_token"]);
    assert_eq!(old["source"]["content_digest"], live["source"]["content_digest"]);
    for foreign in [json!(Uuid::nil()), json!(Uuid::from_u128(42)), json!("not a board"), json!(null)] {
        let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({"board":foreign}))));
        assert!(ops.is_empty());
        let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"plan_token":live["plan_token"],"board":foreign}))));
        assert!(ops.is_empty());
        assert_eq!(s.board_update_report().unwrap(), live);
    }
    let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, token(&old))));
    assert!(ops.is_empty());
    let (r, ops) = trace(|| run(&mut s, update::CANCEL_UPDATE, token(&live)));
    assert!(r.is_ok() && ops.is_empty() && s.board_update_report().is_none());
    let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&live))));
    assert!(ops.is_empty());
    assert!(observer.0.lock().unwrap().is_empty());
    assert_eq!(bytes(&s), before);
    for decision_fields in [json!({}), json!({"accept_loss":[],"confirm_remaps":[],"board":board})] {
        let live = preview(&mut s, json!({}), &SOURCE);
        let mut args = decision_fields;
        args["plan_token"] = live["plan_token"].clone();
        let history = s.undo.len();
        let notifications = observer.0.lock().unwrap().len();
        let (r, ops) = trace(|| run(&mut s, update::APPLY_UPDATE, args));
        assert!(r.is_ok());
        assert_eq!(ops, [Op::BuildEco, Op::CandidateEncode, Op::Prepare(Outer), Op::Publish(Outer)]);
        assert_eq!(s.undo.len(), history + 1);
        assert_eq!(observer.0.lock().unwrap().len(), notifications + 1);
        assert_eq!(observer.0.lock().unwrap().last().unwrap().command, update::APPLY_UPDATE);
        assert_eq!(s.project().unwrap().board.baseline.as_ref().unwrap().source.capture_id.to_string(), live["source"]["capture_id"]);
        let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&live))));
        assert!(ops.is_empty());
        let (undo, ops) = trace(|| s.apply("edit.undo", &Value::Null));
        undo.unwrap();
        assert!(ops.is_empty());
        assert_eq!(bytes(&s), before);
    }
}

/// Blockers and genuine no-op reviews never build, retain or prepare; no-op consumes only its token.
#[test]
fn a_eco_lazy_blocked_noop_and_no_board() {
    let mut s = fixture(true);
    let accepted = s.project().unwrap().board.baseline.as_ref().unwrap().source.clone();
    let symbol = s.project().unwrap().symbols[0].id();
    let reference = s.project().unwrap().components().next().unwrap().reference.clone();
    run(&mut s, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":"content only"})).unwrap();
    let before = bytes(&s);
    let snapshot = s.snapshot().unwrap();
    let state = (s.revision(), s.undo.len(), s.redo.len());
    let observer = observe(&mut s);
    let noop = preview(&mut s, json!({}), &SOURCE);
    assert!(s.cached_board_update().unwrap().changes.is_noop());
    let (_, ops) = trace(|| {
        unchanged(&mut s, |s| {
            run(
                s,
                update::PREVIEW_UPDATE,
                json!({"remap_choices":[{
                    "relation":Uuid::from_u128(27),"pick":1,"choice":{"kind":"keep_lost"}
                }]}),
            )
        })
    });
    assert_eq!(ops, SOURCE);
    assert_eq!(s.board_update_report().unwrap(), noop);
    for exact_empty in [false, true] {
        let noop = if exact_empty { preview(&mut s, json!({}), &SOURCE) } else { noop.clone() };
        let mut args = token(&noop);
        if exact_empty {
            args["accept_loss"] = json!([]);
            args["confirm_remaps"] = json!([]);
        }
        let (r, ops) = trace(|| run(&mut s, update::APPLY_UPDATE, args));
        r.unwrap();
        assert!(ops.is_empty());
        assert!(s.board_update_report().is_none());
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!((s.revision(), s.undo.len(), s.redo.len()), state);
        assert_eq!(s.project().unwrap().board.baseline.as_ref().unwrap().source, accepted);
        assert_eq!(bytes(&s), before);
        unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&noop)));
    }
    assert!(observer.0.lock().unwrap().is_empty());
    run(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":70_000_000,"y_nm":0})).unwrap();
    // Native diff with no structural rows, but the same unresolved package blocker.
    let p = s.snapshot().unwrap();
    let mut p = p.as_ref().clone();
    let source = p.board.baseline.as_ref().unwrap().source.clone();
    let accepted_with_blocker = update::Baseline::proposed(&p, &update::Baseline::current_choices(&p), source).unwrap();
    Arc::make_mut(&mut p.board).baseline = Some(Arc::new(accepted_with_blocker));
    s.open_snapshot(Arc::new(p));
    let before = bytes(&s);
    let observer = observe(&mut s);
    let blocked = preview(&mut s, json!({}), &SOURCE);
    for key in ["components", "added_nets", "removed_nets"] {
        assert_eq!(blocked["changes"][key], json!([]));
    }
    assert!(!s.cached_board_update().unwrap().can_apply);
    assert!(!blocked["changes"]["blockers"].as_array().unwrap().is_empty());
    let (error, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&blocked))));
    assert!(ops.is_empty() && error.to_string().contains("package blockers"));
    let (r, ops) = trace(|| run(&mut s, update::CANCEL_UPDATE, token(&blocked)));
    r.unwrap();
    assert!(ops.is_empty());
    assert!(observer.0.lock().unwrap().is_empty());
    assert_eq!(bytes(&s), before);

    let mut no_board = Session::default();
    run(&mut no_board, "project.new", json!({"create_board":false})).unwrap();
    for args in [json!({}), json!({"board":Uuid::from_u128(42)})] {
        let (_, ops) = trace(|| unchanged(&mut no_board, |s| run(s, update::PREVIEW_UPDATE, args)));
        assert!(ops.is_empty());
    }
    let (_, ops) = trace(|| unchanged(&mut no_board, |s| run(s, update::APPLY_UPDATE, token(&blocked))));
    assert!(ops.is_empty());
}

/// An oversized native report must fail without displacing a previously approved-size review.
#[test]
fn a_eco_lazy_complete_report_bound_and_checked_cancel() {
    let mut s = fixture(true);
    pending(&mut s);
    let a = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), a);
    let r = preview(&mut s, json!({}), &CHECKED);
    let base = s.snapshot().unwrap();
    let (_, ops) = trace(|| run(&mut s, update::CANCEL_UPDATE, token(&r)).unwrap());
    assert!(ops.is_empty() && s.checked_eco_candidate().is_none());
    assert!(Arc::ptr_eq(&base, &s.snapshot().unwrap()));
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&r)));

    s.open_snapshot(fill_board(&s, 7 * 1024 * 1024));
    let fp = s.project().unwrap().board.footprints()[0].id;
    let p = super::tests::reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{fp}")]}));
    let observer = observe(&mut s);
    let (e, ops) = trace(|| unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({}))));
    assert!(e.to_string().contains("complete preview exceeds 8 MiB"), "{e}");
    assert_eq!(ops, SOURCE, "native-only overflow must never reach the physical builder");
    assert!(s.checked_eco_candidate().is_none());
    assert!(observer.0.lock().unwrap().is_empty());
    let (_, ops) = trace(|| run(&mut s, APPLY, super::tests::decision(&p)).unwrap());
    assert_eq!(ops, [Op::Publish(Outer)]);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
}

/// A native report that just fits may still refuse after adding checked detail, without
/// losing the prior token. Calibrate only real historical display text, not a mock result cap.
#[test]
fn a_eco_checked_detail_overflow_keeps_prior_generic_review() {
    let mut s = fixture(true);
    let departing = pick(&mut s, 2, "fp.origin", None);
    add(&mut s, "horizontal", origin(), departing);
    pending(&mut s);
    let normal = preview(&mut s, json!({}), &CHECKED);
    assert_eq!(normal["relation_detail"]["losses"].as_array().unwrap().len(), 1);
    let limit = update::MAX_PREVIEW_BYTES;
    // 130 entries * 60,000 bytes fits with framing; 130 * 65,536 exceeds the cap.
    let filled = fill_board(&s, crate::persistence::MAX_DOCUMENT_BYTES - 130 * 62_000);
    let mut native = filled.as_ref().clone();
    Arc::make_mut(&mut native.board).relations = None;
    s.open_snapshot(Arc::new(native.clone()));
    let raw = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap();
    let mut remaining = limit - serde_json::to_vec(&raw).unwrap().len();
    drop(raw);
    let baseline = Arc::make_mut(Arc::make_mut(&mut native.board).baseline.as_mut().unwrap());
    for text in baseline.nets.iter_mut().filter_map(|net| net.display_name.as_mut()).filter(|text| text.len() >= 60_000) {
        let n = remaining.min(65_536 - text.len());
        text.push_str(&"x".repeat(n));
        remaining -= n;
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(remaining, 0);
    s.open_snapshot(Arc::new(native.clone()));
    let (raw, ops) = trace(|| run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap());
    assert_eq!(ops, SOURCE);
    assert_eq!(serde_json::to_vec(&raw).unwrap().len(), limit);
    assert!(raw["board_update_preview"].get("candidate_digest").is_none());
    drop(raw);
    Arc::make_mut(&mut native.board).relations = filled.board.relations.clone();
    s.open_snapshot(Arc::new(native));
    let fp = s.project().unwrap().board.footprints()[0].id;
    let prior = super::tests::reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{fp}")]}));
    let checked = s.checked_generic_candidate().unwrap().clone();
    let observer = observe(&mut s);
    let (error, ops) = trace(|| unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({}))));
    assert!(error.to_string().contains("complete preview exceeds 8 MiB"), "{error}");
    assert_eq!(ops, CHECKED[..CHECKED.len() - 1]);
    assert!(observer.0.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(&checked, s.checked_generic_candidate().unwrap()));
    let (_, ops) = trace(|| run(&mut s, APPLY, super::tests::decision(&prior)).unwrap());
    assert_eq!(ops, [Op::Publish(Outer)]);
    assert!(Arc::ptr_eq(&checked, &s.snapshot().unwrap()));
    assert_eq!(observer.0.lock().unwrap().len(), 1);
}

/// A content-only change replaces the source digest even when the structural ECO diff is identical.
#[test]
fn a_preview_bind_eco_source_changes_without_changing_structural_diff() {
    let mut s = fixture(true);
    let reference = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), reference);
    pending(&mut s);
    let first = preview(&mut s, json!({}), &CHECKED);
    let symbol = s.project().unwrap().symbols[0].id();
    let reference = s.project().unwrap().components().next().unwrap().reference().to_owned();
    run(&mut s, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":"new source"})).unwrap();
    let latest = preview(&mut s, json!({}), &CHECKED);
    assert_ne!(first["source"]["content_digest"], latest["source"]["content_digest"]);
    assert_ne!(first["intent_digest"], latest["intent_digest"]);
    assert_eq!(first["changes"], latest["changes"]);
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(&first)));
    unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, token(&first)));
    let mut override_source = token(&latest);
    override_source["source"] = first["source"].clone();
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, override_source));
    assert_eq!(s.board_update_report().unwrap(), latest);
    let (_, ops) = trace(|| run(&mut s, update::APPLY_UPDATE, token(&latest)).unwrap());
    assert_eq!(ops, [Op::Publish(Outer)]);
}

/// Change eligibility both ways and test stale/replaced capabilities, not route inference from args.
#[test]
fn a_eco_lazy_checked_both_stale_directions_and_cross_route_replacement() {
    for starts_checked in [false, true] {
        let mut s = fixture(true);
        pending(&mut s);
        let a = pick(&mut s, 0, "fp.origin", None);
        let row = starts_checked.then(|| add(&mut s, "horizontal", origin(), a.clone()));
        let first = preview(&mut s, json!({}), if starts_checked { &CHECKED } else { &SOURCE[..] });
        if let Some(row) = row {
            run(&mut s, commands::DELETE, json!({"ids":[row]})).unwrap();
        } else {
            let row = add(&mut s, "horizontal", origin(), a);
            run(&mut s, EDIT, json!({"id":row,"edit":{"suppressed":true}})).unwrap();
        }
        let next_ops: &[Op] = if starts_checked { &SOURCE } else { &CHECKED };
        let next = preview(&mut s, json!({}), next_ops);
        let base = s.snapshot().unwrap();
        assert!(s.adopt_saved_metadata(&base, &Arc::new(base.renamed("Saved during ECO").unwrap())));
        for stale in [&first, &next] {
            let (_, ops) = trace(|| unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, token(stale))));
            assert!(ops.is_empty());
        }
        let next = preview(&mut s, json!({}), next_ops);
        let fp = s.project().unwrap().board.footprints()[1].id;
        let generic = super::tests::reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{fp}")]}));
        assert!(s.board_update_report().is_none());
        unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, token(&next)));
        assert!(matches!(s.update_plan, Some(super::ReviewSlot::GenericPrepared(_))));
        let live = preview(&mut s, json!({}), next_ops);
        for old in [&first, &next] {
            unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, token(old)));
        }
        unchanged(&mut s, |s| run(s, CANCEL, json!({"token":generic["token"]})));
        unchanged(&mut s, |s| run(s, APPLY, super::tests::decision(&generic)));
        unchanged(&mut s, |s| run(s, APPLY, json!({"token":live["plan_token"],"accept_loss":[],"confirm_remaps":[]})));
        assert_eq!(s.board_update_report().unwrap(), live);
        let retained = s.checked_eco_candidate().cloned();
        let (r, ops) = trace(|| run(&mut s, update::APPLY_UPDATE, token(&live)));
        r.unwrap();
        if let Some(retained) = retained {
            assert_eq!(ops, [Op::Publish(Outer)]);
            assert!(Arc::ptr_eq(&retained, &s.snapshot().unwrap()));
            assert_eq!(bytes(&s), crate::persistence::encode_documents(retained).unwrap());
        } else {
            assert_eq!(ops, [Op::BuildEco, Op::CandidateEncode, Op::Prepare(Outer), Op::Publish(Outer)]);
        }
    }
}
