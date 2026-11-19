//! Fill the native board section, not a mock budget or a side document.
use super::tests::{add, bytes, decision, fixture, origin, pick, reviewed, run, unchanged};
use super::{State, commands::*};
use crate::{
    Project, Session,
    board::{self, update},
    persistence::{MAX_DOCUMENT_BYTES, decode_documents, encode_documents, test_board_path},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Use bounded display names in accepted history so the fixture needs very few live objects.
/// Only the display length is adjusted; no board bytes or production limits are patched.
pub(super) fn fill_board(s: &Session, slack: usize) -> Arc<Project> {
    let mut p = s.project().unwrap().clone();
    let t = p.board_target().unwrap();
    let board = Arc::make_mut(&mut p.board);
    let mut baseline = board.baseline.as_deref().cloned().unwrap_or_else(|| {
        update::Baseline::empty(t.project, t.board, update::SourceCapture { capture_id: uuid::Uuid::from_u128(7), content_digest: "7".repeat(64) })
            .unwrap()
    });
    baseline.format = update::DISPLAY_FORMAT.into();
    for net in &mut baseline.nets {
        net.origin = None;
        if net.names.is_empty() && net.terminals.len() == 1 {
            net.names.push("saved".into());
        }
        net.display_name.get_or_insert_with(|| "saved".into());
    }
    let package = update::Package::from_assignment(board.assignments.values().next().unwrap());
    let start = 1000;
    let entries = ((MAX_DOCUMENT_BYTES - slack) / 62_000) as u64;
    for i in start..start + entries {
        baseline.components.insert(
            i,
            update::PartInstanceState {
                library: "history".into(),
                definition: "retired".into(),
                revision: 1,
                reference: format!("R{i}"),
                value: "retired".into(),
                primary_quantity: None,
                terminals: vec!["1".into(), "2".into()],
                package: Some(package.clone()),
            },
        );
        baseline.nets.push(update::NetState {
            terminals: vec![update::TerminalKey { component: i, terminal: "1".into() }],
            names: vec![format!("saved-{i}")],
            display_name: Some("x".repeat(60_000)),
            origin: None,
        });
    }
    baseline.nets.sort_by(|a, b| a.terminals.cmp(&b.terminals));
    baseline.validate().unwrap();
    board.baseline = Some(Arc::new(baseline));
    let path = p.board_document_path().unwrap();
    let current = encode_documents(Arc::new(p.clone())).unwrap()[&path].len();
    let mut remaining = MAX_DOCUMENT_BYTES - slack - current;
    for net in &mut Arc::make_mut(Arc::make_mut(&mut p.board).baseline.as_mut().unwrap()).nets {
        let text = net.display_name.as_mut().unwrap();
        if text.len() != 60_000 {
            continue;
        }
        let n = remaining.min(65_536 - text.len());
        text.push_str(&"x".repeat(n));
        remaining -= n;
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(remaining, 0);
    Arc::new(p)
}

/// Real writer accounting includes F1 even when no routed copper exists or every row is inactive.
#[test]
fn a_f1_board_size_fit_overflow_inactive_and_native_control() {
    for status in ["reference", "suppressed", "lost"] {
        let mut s = fixture(false);
        let a = pick(&mut s, 0, "fp.origin", None);
        let row = add(&mut s, "horizontal", origin(), a);
        let ids: Vec<_> = s.project().unwrap().board.footprints().iter().map(|f| format!("footprint-{}", f.id)).collect();
        if status == "suppressed" {
            run(&mut s, EDIT, json!({"id":row,"edit":{"suppressed":true}})).unwrap();
        }
        if status == "lost" {
            let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[ids[0]]}));
            run(&mut s, APPLY, decision(&p)).unwrap();
        }
        let initial = fill_board(&s, 96);
        let mut native = initial.as_ref().clone();
        Arc::make_mut(&mut native.board).relations = None;
        s.open_snapshot(initial);
        let before = bytes(&s);
        let path = test_board_path(&before);
        assert_eq!(before[&path].len(), MAX_DOCUMENT_BYTES - 96);
        assert!(s.project().unwrap().board.tracks.is_empty() && s.project().unwrap().board.vias.is_empty());
        let args = |text: String| json!({"footprint":ids[2],"appearance":{"text":text}});
        let big = args("z".repeat(128));
        let err = unchanged(&mut s, |s| run(s, crate::SET_FOOTPRINT_REFERENCE_TEXT, big.clone()));
        assert!(err.to_string().contains("16 MiB limit"), "{err}");
        assert_eq!(bytes(&s), before);
        run(&mut s, crate::SET_FOOTPRINT_REFERENCE_TEXT, args("a".repeat(40))).unwrap();
        let saved = bytes(&s);
        assert!(saved[&path].len() > before[&path].len());
        s.open_snapshot(decode_documents(&saved).unwrap());
        assert_eq!(bytes(&s), saved);
        assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], status);
        s.open_snapshot(Arc::new(native));
        let (result, trace) = super::probes::trace(|| run(&mut s, crate::SET_FOOTPRINT_REFERENCE_TEXT, big));
        result.unwrap();
        use super::{RelationStage::Outer, probes::Op};
        assert_eq!(trace, [Op::Prepare(Outer), Op::Publish(Outer)]);
        assert!(bytes(&s)[&path].len() <= MAX_DOCUMENT_BYTES);
    }
}

/// An independent canonical encoder failure cannot install an approval, even though the
/// exact board size accounting alone would succeed. Inject an absent immutable library record
/// in private fixture state, not a saver override or production failure hook.
#[test]
fn a_final_bytes_native_writer_failure_is_not_approvable() {
    let mut s = fixture(false);
    let r = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), r);
    let mut p = s.project().unwrap().clone();
    let bad = p.board.footprints()[2].key().unwrap();
    Arc::make_mut(&mut p.board).definitions.remove(&bad);
    crate::persistence::check_board_document_size(&p).unwrap();
    s.open_snapshot(Arc::new(p));
    let id = s.project().unwrap().board.footprints()[0].id;
    let e = unchanged(&mut s, |s| {
        run(s, PREVIEW, json!({"intent":{"command":crate::DELETE_FOOTPRINTS,"params":{"footprints":[format!("footprint-{id}")]}}}))
    });
    assert!(e.to_string().contains("during save"), "{e}");
    assert!(s.update_plan.is_none());
}

/// The native transform fits before latching; only the finalized F1 candidate may authorize
/// approval. Choose slack on either side of the lost-record growth using actual native encodings.
#[test]
fn a_final_bytes_copper_latch_fit_and_overflow() {
    let mut s = board::track::tests::routed_fixture();
    let front = board::track::tests::copper_layers(&s)[0];
    board::track::tests::route(
        &mut s,
        front,
        &[crate::Point { x_nm: 40_000_000, y_nm: 40_000_000 }, crate::Point { x_nm: 41_000_000, y_nm: 40_000_000 }],
    )
    .unwrap();
    let edge = pick(&mut s, 0, "pad.edge", Some("1"));
    let point = pick(&mut s, 1, "fp.origin", None);
    add(&mut s, "point_edge", edge, point);
    let fp = &s.project().unwrap().board.footprints()[0];
    let args = json!({"footprint":format!("footprint-{}",fp.id),"x_nm":fp.x_nm,"y_nm":fp.y_nm,"rotation":37});
    let start = bytes(&s);
    let path = test_board_path(&start);
    let native_growth = {
        let next = s.project().unwrap().clone();
        let mut intermediate = next;
        Arc::make_mut(&mut intermediate.board).footprints[0].rotation = 37;
        encode_documents(Arc::new(intermediate)).unwrap()[&path].len() - start[&path].len()
    };
    let fit = fill_board(&s, native_growth + 60);
    let overflow = fill_board(&s, native_growth + 4);
    let intent = json!({"intent":{"command":crate::SET_FOOTPRINT_TRANSFORM,"params":args}});
    s.open_snapshot(overflow);
    let before = bytes(&s);
    let mut raw = s.project().unwrap().clone();
    Arc::make_mut(&mut raw.board).footprints[0].rotation = 37;
    assert_eq!(encode_documents(Arc::new(raw)).unwrap()[&path].len(), MAX_DOCUMENT_BYTES - 4);
    let observer: Arc<super::tests::Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let error = unchanged(&mut s, |s| run(s, PREVIEW, intent.clone()));
    assert!(error.to_string().contains("size limit"), "{error}");
    assert!(s.update_plan.is_none() && observer.0.lock().unwrap().is_empty());
    assert_eq!(bytes(&s), before);
    s.open_snapshot(fit);
    let report = run(&mut s, PREVIEW, intent).unwrap()["relation_preview"].clone();
    let certified = s.checked_generic_candidate().unwrap().clone();
    let certified_docs = encode_documents(certified.clone()).unwrap();
    assert!(certified_docs[&path].len() <= MAX_DOCUMENT_BYTES);
    let (_, ops) = super::probes::trace(|| run(&mut s, APPLY, decision(&report)).unwrap());
    assert_eq!(ops, [super::probes::Op::Publish(super::RelationStage::Outer)]);
    assert!(Arc::ptr_eq(&certified, &s.snapshot().unwrap()));
    assert_eq!(bytes(&s), certified_docs);
    assert_eq!(super::certify(s.snapshot().unwrap()).unwrap(), report["candidate_digest"].as_str().unwrap());
    assert!(matches!(s.project().unwrap().board.relations.as_ref().unwrap().relations[0].state, State::Lost { .. }));
    assert_eq!(s.project().unwrap().board.footprints()[0].rotation, 37);
}
