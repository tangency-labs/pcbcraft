use super::{
    catalog::{self, Measure},
    commands::*,
    *,
};
use crate::{
    CommandError, Project, Session,
    board::{self, update},
    persistence::{Documents, decode_documents, encode_documents, test_board_path},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Execute the same addressed native/relationship commands that production clients discover.
pub(super) fn run(s: &mut Session, command: &str, mut params: Value) -> Result<Value, CommandError> {
    if crate::command(command).unwrap().params.iter().any(|p| p.name == "expected_revision") {
        params["expected_revision"] = json!(s.revision());
    }
    s.execute(command, &params)
}

/// Real pinned resistor footprints on a native board, optionally with accepted physical history.
pub(super) fn fixture(baseline: bool) -> Session {
    let mut s = Session::default();
    run(&mut s, "project.new", json!({"create_board":true})).unwrap();
    for (i, x) in [0, 10_000_000, 20_000_000].into_iter().enumerate() {
        run(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0})).unwrap();
        let id = *s.project().unwrap().components.keys().next_back().unwrap();
        let d = board::definition::basic_components(if i == 0 { "resistor-0402" } else { "resistor-0603" }).unwrap();
        run(&mut s, crate::ASSIGN_FOOTPRINT, json!({"component":format!("component-{id}"),"library":d.library,"definition":d.id,"definition_revision":d.revision,"mapping":{"1":"1","2":"2"}})).unwrap();
        run(&mut s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":format!("component-{id}"),"x_nm":x + 3_000_000,"y_nm":2_000_000})).unwrap();
    }
    if baseline {
        run(&mut s, update::INITIALIZE_BASELINE, json!({})).unwrap();
    }
    s
}

/// Select a reference exactly as the engine's public catalog emits it.
pub(super) fn pick(s: &mut Session, index: usize, tag: &str, pad: Option<&str>) -> Value {
    let id = s.project().unwrap().board.footprints()[index].id;
    let response = run(s, FEATURES, json!({"fp":format!("footprint-{id}")})).unwrap();
    response["features"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|item| {
            let r = &item["reference"];
            (r["tag"] == tag && pad.is_none_or(|pad| r["pad"] == pad)).then(|| r.clone())
        })
        .unwrap()
}

pub(super) fn add(s: &mut Session, kind: &str, a: Value, b: Value) -> Value {
    run(s, ADD, json!({"relation":{"kind":kind,"refs":[a,b]}})).unwrap()["id"].clone()
}

pub(super) fn origin() -> Value {
    json!({"tag":"board.origin"})
}
pub(super) fn bytes(s: &Session) -> Documents {
    encode_documents(s.snapshot().unwrap()).unwrap()
}
pub(super) fn reviewed(s: &mut Session, command: &str, params: Value) -> Value {
    run(s, PREVIEW, json!({"intent":{"command":command,"params":params}})).unwrap()["relation_preview"].clone()
}

/// Build exactly the two disclosed decision sets; tests deliberately corrupt them separately.
pub(super) fn decision(preview: &Value) -> Value {
    let disclosure = &preview["disclosure"];
    let ids = |key: &str| disclosure[key].as_array().unwrap().iter().map(|row| row["id"].clone()).collect::<Vec<_>>();
    json!({"token":preview["token"], "accept_loss":ids("losses"), "confirm_remaps":ids("remaps")})
}

/// Compare immutable snapshot, revision and both history cursors across a refusal.
pub(super) fn unchanged(s: &mut Session, attempt: impl FnOnce(&mut Session) -> Result<Value, CommandError>) -> CommandError {
    let before = (s.snapshot().unwrap(), s.revision(), s.undo.len(), s.redo.len());
    let error = attempt(s).unwrap_err();
    assert!(Arc::ptr_eq(&before.0, &s.snapshot().unwrap()));
    assert_eq!((before.1, before.2, before.3), (s.revision(), s.undo.len(), s.redo.len()));
    error
}

/// Catalog follows the same native rectangle projection across side, turns and odd sizes.
#[test]
fn a_cat_ref_transforms_and_closed_catalog() {
    let s = fixture(false);
    let base = s.project().unwrap();
    for (bottom, angle) in [false, true].into_iter().flat_map(|b| [0, 90, 180, 270, 37].map(|angle| (b, angle))) {
        for pad_turn in [0, 90, 30] {
            let mut p = base.clone();
            let board = Arc::make_mut(&mut p.board);
            let f = &mut board.footprints[0];
            (f.bottom, f.rotation, f.x_nm, f.y_nm) = (bottom, angle, 0, 0);
            let def = Arc::make_mut(board.definitions.get_mut(&f.key().unwrap()).unwrap());
            let pad = &mut def.pads[0];
            (pad.center, pad.width_nm, pad.height_nm, pad.rotation, pad.shape, pad.corner_radius_nm) =
                (Point { x_nm: 2_000_000, y_nm: 1_000_000 }, 123_457, 765_431, pad_turn, board::definition::PadShape::Rectangle, 0);
            let number = pad.number.clone();
            let expected = board::view::rectangle_points(f, pad.center, pad.width_nm, pad.height_nm, pad.rotation);
            let center = f.point(pad.center);
            let available = catalog::features(&p, &p.board.footprints()[0]);
            assert!(
                available.iter().any(|(r, w)| matches!(r, Reference::PadCenter { pad, .. } if pad == &number) && *w == Witness::Point { at: center })
            );
            if bottom && angle == 0 {
                assert_eq!(center, Point { x_nm: -2_000_000, y_nm: 1_000_000 });
            }
            let corners: Vec<_> = available
                .iter()
                .filter_map(|(r, w)| match (r, w) {
                    (Reference::PadCorner { pad, .. }, Witness::Point { at }) if pad == &number => Some(*at),
                    _ => None,
                })
                .collect();
            assert_eq!(corners, vec![expected[3], expected[2], expected[1], expected[0]]);
            let edges = available.iter().filter(|(r, _)| matches!(r, Reference::PadEdge {pad,..} if pad == &number)).count();
            assert_eq!(edges, if angle % 90 == 0 && pad_turn % 90 == 0 { 4 } else { 0 });
        }
    }
    let mut p = base.clone();
    let board = Arc::make_mut(&mut p.board);
    let f = &board.footprints[0];
    let def = Arc::make_mut(board.definitions.get_mut(&f.key().unwrap()).unwrap());
    let d = &mut def.pads[0];
    d.shape = board::definition::PadShape::RoundedRectangle;
    d.width_nm = d.height_nm;
    d.corner_radius_nm = d.width_nm / 2;
    let number = d.number.clone();
    let features = catalog::features(&p, &p.board.footprints()[0]);
    assert!(!features.iter().any(|(r, _)| r.pad() == Some(number.as_str()) && !matches!(r, Reference::PadCenter { .. })));
}

/// Signed projection always follows the ordered pair and never the outward normal; slant is a reference.
#[test]
fn a_cat_ref_ordered_measurement_and_refusal() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    let b = pick(&mut s, 1, "pad.edge", Some("1"));
    let a: Reference = serde_json::from_value(a).unwrap();
    let b: Reference = serde_json::from_value(b).unwrap();
    let p = s.project().unwrap();
    let Some(Measure::Projection { signed_nm: d, .. }) = catalog::measure(p, Kind::PointEdge, &[a.clone(), b.clone()]) else { panic!("point-edge") };
    let Some(Measure::Projection { signed_nm: r, .. }) = catalog::measure(p, Kind::PointEdge, &[b.clone(), a.clone()]) else { panic!("edge-point") };
    assert_ne!(d, 0);
    assert_eq!(d, -r);
    assert!(catalog::measure(p, Kind::Horizontal, &[a.clone(), a.clone()]).is_none());
    assert!(catalog::measure(p, Kind::Aligned, &[Reference::BoardOrigin {}, a]).is_some());
    let edge = |r: &Reference, side: Side| match r {
        Reference::PadEdge { fp, def, pad, .. } => Reference::PadEdge { fp: *fp, def: def.clone(), pad: pad.clone(), side },
        _ => panic!(),
    };
    let a: Reference = serde_json::from_value(pick(&mut s, 0, "pad.edge", Some("1"))).unwrap();
    let p = s.project().unwrap();
    assert!(catalog::measure(p, Kind::Gap, &[edge(&a, Side::Top), edge(&b, Side::Left)]).is_none());
    assert!(catalog::measure(p, Kind::Gap, &[edge(&a, Side::Right), edge(&b, Side::Left)]).is_some());
}

/// F1 round-trips the original closed tags, adds no clipboard bytes, and downgrades only once
/// the final row has gone. Known driving rows and mismatched/partial headers are refused intact.
#[test]
fn a_format_p1_clipboard_and_driving_row_reader() {
    let mut s = fixture(false);
    let fp = s.project().unwrap().board.footprints()[0].id;
    let items = json!({"footprints":[format!("footprint-{fp}")]});
    let copy_before = run(&mut s, board::clipboard::COPY, json!({"items":items})).unwrap()["clipboard"].clone();
    let a = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), a);
    assert_eq!(run(&mut s, board::clipboard::COPY, json!({"items":items})).unwrap()["clipboard"], copy_before);
    let docs = bytes(&s);
    let path = test_board_path(&docs);
    let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/39");
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    for (key, value) in [("format", json!("pcbcraft-board/35")), ("format", json!("pcbcraft-board/38")), ("relations", Value::Null)] {
        let mut changed = board.clone();
        changed[key] = value;
        let mut doc = docs.clone();
        doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
        assert!(decode_documents(&doc).is_err());
    }
    let mut changed = board.clone();
    changed["relations"]["relations"][0]["driving"] = json!(true);
    let mut doc = docs.clone();
    doc.insert(path.clone(), serde_json::to_vec(&changed).unwrap());
    let untouched = doc.clone();
    assert!(decode_documents(&doc).unwrap_err().contains("reference-only reader"));
    assert_eq!(doc, untouched);
    run(&mut s, EDIT, json!({"id":id,"edit":{"suppressed":true}})).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "suppressed");
    let future = json!({"tag":"outline.edge","drawing":Uuid::from_u128(27)});
    assert!(serde_json::from_value::<Reference>(future).is_err());
    run(&mut s, DELETE, json!({"ids":[id]})).unwrap();
    let docs = bytes(&s);
    assert_ne!(serde_json::from_slice::<Value>(&docs[&test_board_path(&docs)]).unwrap()["format"], "pcbcraft-board/39");
}

/// DEC-1415: an exact reviewed loss survives native save/reopen, Undo/Redo and reappearance
/// of a datum, never resolves from its witness, and never moves the surviving placement.
#[test]
fn a_native_outline_datum_lost_witness_reopen_no_motion() {
    let mut s = board::track::tests::routed_fixture();
    board::drc::tests::outline(&mut s);
    run(&mut s, crate::SET_PLACEMENT_ORIGIN, json!({"origin":{"kind":"outline_lower_left"}})).unwrap();
    let a = pick(&mut s, 0, "pad.center", Some("1"));
    add(&mut s, "horizontal", origin(), a);
    let shown = run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"]["witnesses"].clone();
    let before = s.project().unwrap().board.footprints().to_vec();
    let drawing = s.project().unwrap().board.drawings.as_ref().unwrap().items[0].id;
    let items = json!({"drawings":[drawing]});
    let error = unchanged(&mut s, |s| run(s, board::clipboard::DELETE, json!({"items":items})));
    assert_eq!(error.details()["reason"], "would_lose");
    let p = reviewed(&mut s, board::clipboard::CUT, json!({"items":items}));
    assert_eq!(p["disclosure"]["losses"][0]["last_resolved"], shown);
    let response = run(&mut s, APPLY, decision(&p)).unwrap();
    assert!(response["clipboard"].is_string());
    assert_eq!(s.project().unwrap().board.footprints(), before.as_slice());
    let stored = bytes(&s);
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), p["candidate_digest"].as_str().unwrap());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(bytes(&s), stored);
    s.open_snapshot(decode_documents(&stored).unwrap());
    assert_eq!(bytes(&s), stored);
    board::drc::tests::outline(&mut s);
    let result = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(result["relations"][0]["status"], "lost");
    assert_eq!(result["relations"][0]["last_resolved"], shown);
    assert!(result["relations"][0].get("measure").is_none());
    assert_eq!(s.project().unwrap().board.footprints(), before.as_slice());
}

/// A structurally out-of-range display record can't be admitted; it is never resolved as native geometry.
#[test]
fn dec_1415_witness_coordinate_and_arity_caps() {
    let mut s = fixture(false);
    let ref_b = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), ref_b);
    let docs = bytes(&s);
    let path = test_board_path(&docs);
    for which in [0, 1] {
        let mut bad = docs.clone();
        let mut board: Value = serde_json::from_slice(&docs[&path]).unwrap();
        let witness = &mut board["relations"]["relations"][0]["last_resolved"];
        if which == 0 {
            witness[0]["at"]["x_nm"] = json!(MAX_WITNESS_NM + 1);
        } else {
            witness.as_array_mut().unwrap().push(json!({"kind":"point","at":{"x_nm":0,"y_nm":0}}));
        }
        bad.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
}

#[derive(Debug, Default)]
struct Observe(std::sync::Mutex<Vec<crate::edits::CommittedEdit>>);
impl crate::edits::EditObserver for Observe {
    fn opened(&self, _: Arc<Project>) {}
    fn committed(&self, edit: crate::edits::CommittedEdit) {
        self.0.lock().unwrap().push(edit);
    }
}

/// Before approval there is no clipboard result, publication or observer event. Invalid, cancelled,
/// wrong-route and replaced capabilities cannot publish and cannot revoke the current review.
#[test]
fn a_generic_exact_cancel_and_engine_result_bounds() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), a);
    let fp = s.project().unwrap().board.footprints()[0].id;
    let items = json!({"footprints":[format!("footprint-{fp}")]});
    let args = json!({"items":items});
    let selection = board::clipboard::BoardSelection::parse(&items).unwrap();
    let expected = board::clipboard::BoardFragment::capture(s.project().unwrap(), &selection, true).unwrap().to_text().unwrap();
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    unchanged(&mut s, |s| run(s, board::clipboard::CUT, args.clone()));
    let expected_revision = s.revision();
    assert!(s.apply(board::clipboard::CUT, &json!({"items":items,"expected_revision":expected_revision})).is_err());
    let old = reviewed(&mut s, board::clipboard::CUT, args.clone());
    assert!(old.get("clipboard").is_none());
    run(&mut s, CANCEL, json!({"token":old["token"]})).unwrap();
    unchanged(&mut s, |s| run(s, APPLY, decision(&old)));
    let live = reviewed(&mut s, board::clipboard::CUT, args.clone());
    unchanged(&mut s, |s| run(s, CANCEL, json!({"token":old["token"]})));
    unchanged(&mut s, |s| run(s, update::CANCEL_UPDATE, json!({"plan_token":live["token"]})));
    unchanged(&mut s, |s| run(s, PREVIEW, json!({"intent":{"command":"project.new","params":{}}})));
    unchanged(&mut s, |s| {
        s.relation_command(PREVIEW, &json!({"expected_revision":expected_revision,"intent":{"command":"pcb.cut","params":args}}), 1)
    });
    unchanged(&mut s, |s| s.relation_command(LIST, &json!({}), 1));
    let mut invalid = decision(&live);
    invalid["accept_loss"] = json!([]);
    unchanged(&mut s, |s| run(s, APPLY, invalid));
    let mut decision = decision(&live);
    decision["expected_revision"] = json!(expected_revision);
    unchanged(&mut s, |s| s.relation_command(APPLY, &decision, 100));
    assert!(observer.0.lock().unwrap().is_empty());
    unchanged(&mut s, |s| s.apply(APPLY, &decision).map(|v| json!(v)));
    let response = s.relation_command(APPLY, &decision, usize::MAX).unwrap();
    assert_eq!(response["clipboard"].as_str(), Some(expected.as_str()));
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    assert_eq!(observer.0.lock().unwrap()[0].command, board::clipboard::CUT);
    unchanged(&mut s, |s| run(s, APPLY, decision.clone()));
}

/// The base Arc is part of authority even if save adoption changes neither revision nor board ID.
#[test]
fn a_preview_bind_metadata_arc_and_replacement() {
    let mut s = fixture(false);
    let a = pick(&mut s, 0, "fp.origin", None);
    add(&mut s, "horizontal", origin(), a);
    let fp = s.project().unwrap().board.footprints()[0].id;
    let args = json!({"footprints":[format!("footprint-{fp}")]});
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, args.clone());
    let snapshot = s.snapshot().unwrap();
    let rev = s.revision();
    assert!(s.adopt_saved_metadata(&snapshot, &Arc::new(snapshot.renamed("After Save").unwrap())));
    assert_eq!(s.revision(), rev);
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
    let p = reviewed(&mut s, crate::DELETE_FOOTPRINTS, args.clone());
    let replacement = reviewed(&mut s, crate::DELETE_FOOTPRINTS, args);
    unchanged(&mut s, |s| run(s, APPLY, decision(&p)));
    let mut extra = decision(&replacement);
    extra["intent"] = json!({});
    unchanged(&mut s, |s| run(s, APPLY, extra));
    run(&mut s, APPLY, decision(&replacement)).unwrap();
}

/// Native ECO stays lazy without authored rows and returns the exact old command envelope.
/// On the checked route, retained/suppressed rows still require candidate certification.
#[test]
fn a_eco_compat_lazy_checked_and_target() {
    for related in [false, true] {
        let mut s = fixture(true);
        if related {
            let a = pick(&mut s, 0, "fp.origin", None);
            let id = add(&mut s, "horizontal", origin(), a);
            run(&mut s, EDIT, json!({"id":id,"edit":{"suppressed":true}})).unwrap();
        }
        let sy = s.project().unwrap().symbols.last().unwrap().id();
        run(&mut s, crate::DELETE_SYMBOL, json!({"symbol":sy})).unwrap();
        let before = s.snapshot().unwrap();
        let target = before.board_target().unwrap();
        let report = run(&mut s, update::PREVIEW_UPDATE, json!({"board":target.board})).unwrap()["board_update_preview"].clone();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(report.get("candidate_digest").is_some(), related);
        unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"board":Uuid::from_u128(42),"plan_token":report["plan_token"]})));
        unchanged(&mut s, |s| run(s, CANCEL, json!({"token":report["plan_token"]})));
        run(&mut s, update::CANCEL_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
        let report = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
        run(&mut s, update::APPLY_UPDATE, json!({"plan_token":report["plan_token"]})).unwrap();
        if related {
            assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), report["candidate_digest"].as_str().unwrap());
        }
        let noop = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
        let unchanged = s.snapshot().unwrap();
        run(&mut s, update::APPLY_UPDATE, json!({"plan_token":noop["plan_token"],"accept_loss":[],"confirm_remaps":[]})).unwrap();
        assert!(Arc::ptr_eq(&unchanged, &s.snapshot().unwrap()));
    }
}

/// A single checked ECO can disclose two independent decisions: a retired placement and
/// a still-offered reference moved to a different exact definition revision.
#[test]
fn a_preview_first_eco_requires_both_decisions() {
    let mut s = fixture(true);
    let kept = pick(&mut s, 0, "fp.origin", None);
    let retired = pick(&mut s, 2, "fp.origin", None);
    add(&mut s, "horizontal", origin(), kept);
    add(&mut s, "horizontal", origin(), retired);
    let fp = &s.project().unwrap().board.footprints()[0];
    let component = fp.component;
    let library = board::definition::basic_components("resistor-0603").unwrap();
    run(
        &mut s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":format!("component-{component}"),"library":library.library,
        "definition":library.id,"definition_revision":library.revision,"mapping":{"1":"1","2":"2"}}),
    )
    .unwrap();
    let symbol = s.project().unwrap().symbols.last().unwrap().id();
    run(&mut s, crate::DELETE_SYMBOL, json!({"symbol":symbol})).unwrap();
    let r = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
    let d = &r["relation_detail"];
    let loss = d["losses"][0]["id"].clone();
    let remap = d["remaps"][0]["id"].clone();
    assert!(loss.is_string() && remap.is_string());
    let token = &r["plan_token"];
    for args in [
        json!({"plan_token":token}),
        json!({"plan_token":token,"accept_loss":[loss]}),
        json!({"plan_token":token,"confirm_remaps":[remap]}),
        json!({"plan_token":token,"accept_loss":[loss,loss],"confirm_remaps":[remap]}),
    ] {
        unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, args));
    }
    run(&mut s, update::APPLY_UPDATE, json!({"plan_token":token,"accept_loss":[loss],"confirm_remaps":[remap]})).unwrap();
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), r["candidate_digest"].as_str().unwrap());
}

/// Reference rows compose with native arc-bearing reads/saves, but do not broaden edit eligibility.
#[test]
fn a_format_arc_read_save_and_edit_refusal() {
    let mut s = board::track::tests::routed_fixture();
    let a = pick(&mut s, 0, "fp.origin", None);
    let row = add(&mut s, "horizontal", origin(), a);
    let layer = board::track::tests::copper_layers(&s)[0];
    let target = s.project().unwrap().board_target().unwrap();
    run(
        &mut s,
        board::arc::SET_ARCS,
        json!({"board":target.board,"arcs":[{"id":Uuid::from_u128(22),"layer":layer,"width_nm":250_000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5_000_000,"y_nm":0},"end":{"x_nm":0,"y_nm":5_000_000},"direction":"clockwise"}]}),
    )
    .unwrap();
    let saved = bytes(&s);
    s.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(bytes(&s), saved);
    let err = unchanged(&mut s, |s| run(s, EDIT, json!({"id":row,"edit":{"suppressed":true}})));
    assert_eq!(err.details()["reason"], "arc_board");
    assert_eq!(err.to_string(), board::arc::UNSUPPORTED);
    assert_eq!(bytes(&s), saved);
}

/// Projected and lifecycle deletion must settle only after the canonical outer candidate.
#[test]
fn a_preview_edge_projected_and_sheet_retirement() {
    for accepted in [false, true] {
        for command in [crate::DELETE_SYMBOL, crate::DELETE_ITEMS, crate::clipboard::CUT, crate::sheets::DELETE_SHEET] {
            let mut s = fixture(accepted);
            let reference = pick(&mut s, 0, "fp.origin", None);
            add(&mut s, "horizontal", origin(), reference);
            let component = s.project().unwrap().board.footprints()[0].component;
            let symbol = s.project().unwrap().symbols.iter().find(|s| s.component == component).unwrap().id();
            let sheet = s.project().unwrap().resolve_sheet(None).unwrap().sheet;
            if command == crate::sheets::DELETE_SHEET {
                run(&mut s, crate::sheets::CREATE_SHEET, json!({"name":"Keep"})).unwrap();
            }
            let args = match command {
                crate::DELETE_SYMBOL => json!({"symbol":symbol,"sheet":sheet}),
                crate::sheets::DELETE_SHEET => json!({"sheet":sheet}),
                _ => json!({"items":{"symbols":[symbol]},"sheet":sheet}),
            };
            if accepted {
                run(&mut s, command, args).unwrap();
                assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "reference");
                let preview = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
                let loss = preview["relation_detail"]["losses"][0]["id"].clone();
                run(&mut s, update::APPLY_UPDATE, json!({"plan_token":preview["plan_token"],"accept_loss":[loss],"confirm_remaps":[]})).unwrap();
            } else {
                let err = unchanged(&mut s, |s| run(s, command, args.clone()));
                assert_eq!(err.details()["reason"], "would_lose");
                let p = reviewed(&mut s, command, args);
                let response = run(&mut s, APPLY, decision(&p)).unwrap();
                assert_eq!(response.get("clipboard").is_some(), command == crate::clipboard::CUT);
            }
            assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
        }
    }
}

/// An ECO splitting accepted memberships marks only the affected physical island. The same
/// logical change is valid with disconnected lands; its geometry and reference do not move.
#[test]
fn a_net_eco_local_split_short_and_disconnected_control() {
    for joined in [false, true] {
        let mut s = board::track::tests::routed_fixture();
        let project = s.project().unwrap();
        let component = project.symbols[2].component;
        let b = project.board.footprints().iter().position(|f| f.component == component).unwrap();
        let a = (0..3).find(|i| *i != b).unwrap();
        let center = |i: usize| {
            let f = &project.board.footprints()[i];
            f.pad_position(crate::schematic::PinNumber::new("2").unwrap(), &project.board).unwrap()
        };
        let (a_at, b_at) = (center(a), center(b));
        let symbol = project.symbols[2].id();
        let terminal = project.symbols[2].terminals()[0];
        let at = project.terminal_position(terminal).unwrap();
        let wire = project.wires.iter().find(|w| w.endpoints().contains(&crate::schematic::WireEndpoint::Terminal(terminal))).unwrap();
        let wire_id = wire.id();
        if joined {
            let front = board::track::tests::copper_layers(&s)[0];
            let y = a_at.y_nm.max(b_at.y_nm) + 2_000_000;
            board::track::tests::route(&mut s, front, &[a_at, Point { x_nm: a_at.x_nm, y_nm: y }, Point { x_nm: b_at.x_nm, y_nm: y }, b_at]).unwrap();
        }
        let first = pick(&mut s, a, "pad.center", Some("2"));
        add(&mut s, "horizontal", origin(), first);
        run(&mut s, crate::DELETE_WIRE, json!({"wire":wire_id})).unwrap();
        run(&mut s, crate::net_labels::ADD, json!({"name":"SEPARATE","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm})).unwrap();
        assert_eq!(s.project().unwrap().symbols[2].id(), symbol);
        let before = s.snapshot().unwrap();
        if joined {
            let err = unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({})));
            assert_eq!(err.details()["reason"], "affected_short");
        } else {
            let p = run(&mut s, update::PREVIEW_UPDATE, json!({})).unwrap()["board_update_preview"].clone();
            run(&mut s, update::APPLY_UPDATE, json!({"plan_token":p["plan_token"]})).unwrap();
            assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), p["candidate_digest"].as_str().unwrap());
        }
        let after = s.project().unwrap();
        assert_eq!(before.board.footprints(), after.board.footprints());
        assert_eq!(before.board.tracks, after.board.tracks);
        assert_eq!(before.board.vias, after.board.vias);
    }
}

/// Compare the complete retained v1 base and require the only changes to existing contracts
/// to be optional ECO inputs. Contents are exact at the pre-P1.1 pin, not reconstructed.
#[test]
fn a_eco_compat_exact_base_command_contracts() {
    use sha2::{Digest, Sha256};
    let frozen = include_bytes!("base-command-contracts.json");
    assert_eq!(format!("{:x}", Sha256::digest(frozen)), "8dbb7516f1af185be75259810a5e5bea9def36783dec81b062665fc0e6995c5f");
    let base: Value = serde_json::from_slice(frozen).unwrap();
    let current: Value = serde_json::from_str(include_str!("../../../../../docs/schemas/command-contracts.json")).unwrap();
    let described: Vec<_> = crate::CATALOG.iter().map(crate::Command::to_json).collect();
    crate::command_api::check_snapshot(&current, "commands", &described, crate::command_api::COMMAND_API_VERSION).unwrap();
    let changes = crate::command_api::compare_snapshots(&base, &current).unwrap();
    assert!(!changes.is_empty());
    assert_eq!(base["command_api_version"], current["command_api_version"]);
    for (name, prior) in base["commands"].as_object().unwrap() {
        let mut now = current["commands"][name].clone();
        let props = now["params"]["properties"].as_object_mut().unwrap();
        match name.as_str() {
            update::PREVIEW_UPDATE => {
                assert!(props.remove("remap_choices").is_some());
            }
            update::APPLY_UPDATE => {
                assert!(props.remove("accept_loss").is_some());
                assert!(props.remove("confirm_remaps").is_some());
            }
            _ => {}
        }
        assert_eq!(prior, &now, "native command contract drift: {name}");
    }
}

/// Guard every authorized nested native staging location; none is a client-facing input.
#[test]
fn a_nest_raw_sites_are_explicit_and_unadvertised() {
    let sources = [
        include_str!("../clipboard.rs"),
        include_str!("../../part_instances/commands.rs"),
        include_str!("../../sheets.rs"),
        include_str!("../../persistence/power_migration.rs"),
        include_str!("../../lib.rs"),
    ];
    for (source, expected) in sources.into_iter().zip([1, 2, 1, 1, 0]) {
        assert_eq!(source.matches("RelationStage::Raw").count(), expected);
    }
    let mut s = fixture(false);
    unchanged(&mut s, |s| run(s, PREVIEW, json!({"intent":{"command":"pcb.cut","params":{"items":{},"relation_stage":"Raw"}}})));
}
