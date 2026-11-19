use super::*;
use crate::{
    ADD_TRACKS, ADD_VIA, Session,
    board::{
        geometry::Contour,
        teardrop::{Length, Parameters},
        track::tests::{copper_layers, routed_fixture, view},
    },
    persistence::encode_documents,
    schematic::Point,
};
use serde_json::json;

/// A synthetic via and route. All copper enters the fixture through the shared commands.
fn fixture(count: i64, with_obstacles: bool) -> (Session, Vec<ViaTeardrop>, Vec<(Uuid, Contour)>) {
    let mut session = routed_fixture();
    let layer = copper_layers(&session)[0];
    let mut intents = Vec::new();
    let mut obstacles = Vec::new();
    for i in 0..count {
        let y_nm = 20_000_000 + i * 2_000_000;
        let center = Point { x_nm: 0, y_nm };
        let end = Point { x_nm: 600_000, y_nm };
        session
            .apply(ADD_VIA, &json!({"at":center,"via_diameter_nm":500_000,"via_drill_nm":300_000,"expected_revision":session.revision()}))
            .unwrap();
        add_track(&mut session, layer, center, end);
        let board = view(&session);
        intents.push(ViaTeardrop {
            id: Uuid::from_u128((i + 1) as u128),
            board: board.target,
            via: board.vias().iter().find(|via| via.at == center).unwrap().id,
            track: board.tracks().iter().find(|track| track.start == center).unwrap().id,
            parameters: Parameters { length: Length::Percent(20), max_length_nm: None, width_pct: 80, max_width_nm: None },
        });
        if with_obstacles && (i == 1 || i == 7) {
            let start = Point { x_nm: 260_000, y_nm: y_nm - 220_000 };
            let end = Point { x_nm: start.x_nm, y_nm: start.y_nm - 50_000 };
            add_track(&mut session, layer, start, end);
            let track = view(&session).tracks().iter().find(|track| track.start == start).unwrap();
            obstacles.push((track.id, Contour::segment(track.start, track.end, track.width_nm).unwrap()));
        }
    }
    (session, intents, obstacles)
}

/// Add one segment with an explicit width; neither the test nor planner edits model fields.
fn add_track(session: &mut Session, layer: Uuid, start: Point, end: Point) {
    session.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":100_000,"points":[start,end],"expected_revision":session.revision()})).unwrap();
}

/// Ten vias give eight additions and two reports with real obstacle references and measured gaps.
/// The narrow test evaluator checks only the fixture's two obstacle tracks, not all board rules.
#[test]
fn ten_vias_report_two_clearance_blocks() {
    let (session, intents, obstacles) = fixture(10, true);
    let board = view(&session);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let edits: Vec<_> = intents.iter().rev().copied().map(Edit::Add).collect();
    let mut seen = Vec::new();
    let diff = plan(board, &[], &edits, |addition, retained, accepted| {
        assert!(retained.is_empty());
        assert!(accepted.iter().all(|prior| prior.intent.id < addition.intent.id));
        seen.push(addition.intent.id);
        let quad = Contour { core: addition.copper.vertices, radius: 0. };
        let block = obstacles.iter().find_map(|(id, obstacle)| {
            let actual_nm = quad.gap(obstacle);
            // Fixture policy only: the flank leaves 32 µm while the original via and track clear 35 µm.
            (actual_nm < 35_000.).then(|| RuleBlock { rule: Rule::CopperClearance, other_object: id.to_string(), required_nm: 35_000, actual_nm })
        });
        Ok::<_, ()>(block)
    })
    .unwrap();
    assert_eq!(seen, intents.iter().map(|intent| intent.id).collect::<Vec<_>>());
    let expected: Vec<_> = intents.iter().enumerate().filter(|(i, _)| *i != 1 && *i != 7).map(|(_, intent)| *intent).collect();
    assert_eq!(diff.added.iter().map(|addition| addition.intent).collect::<Vec<_>>(), expected);
    assert!(diff.removed.is_empty());
    assert_eq!(diff.rejected.len(), 2);
    for ((rejected, (obstacle_id, obstacle)), i) in diff.rejected.iter().zip(&obstacles).zip([1, 7]) {
        assert_eq!(rejected.intent, intents[i]);
        let Refusal::Rule(reason) = &rejected.reason else { panic!("expected rule refusal") };
        assert_eq!(
            (reason.rule, reason.other_object.as_str(), reason.required_nm),
            (Rule::CopperClearance, obstacle_id.to_string().as_str(), 35_000)
        );
        assert!((reason.actual_nm - 32_000.).abs() < 0.000_001);
        let via = board.vias().iter().find(|via| via.id == rejected.intent.via).unwrap();
        let track = board.tracks().iter().find(|track| track.id == rejected.intent.track).unwrap();
        assert!(Contour::capsule(via.at, via.at, 250_000.).gap(obstacle) >= 35_000.);
        assert!(Contour::segment(track.start, track.end, track.width_nm).unwrap().gap(obstacle) >= 35_000.);
    }
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Exercise the shared command against real contours and authored board rules, then undo,
/// redo, save, reopen and remove. The 35 µm threshold is a fixture rule, not a fabrication standard.
#[test]
fn command_adds_eight_reports_two_and_round_trips_intent() {
    use crate::{
        ADD_DRAWING, CommandError, SET_RULES,
        board::{
            rules::Values,
            setup::LayerPurpose,
            teardrop::{EDIT_TEARDROPS, Record},
        },
        persistence::decode_documents,
    };

    let (mut session, intents, obstacles) = fixture(10, true);
    let target = view(&session).target;
    let outline = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let corners = [(-5_000_000, -5_000_000), (25_000_000, -5_000_000), (25_000_000, 45_000_000), (-5_000_000, 45_000_000)]
        .map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    session
        .apply(ADD_DRAWING, &json!({"board":target.board,"layer":outline,"points":corners,"closed":true,"expected_revision":session.revision()}))
        .unwrap();
    let values = Values { clearance_nm: 35_000, ..Values::default() };
    session.apply(SET_RULES, &json!({"board":target.board,"rules":values,"expected_revision":session.revision()})).unwrap();
    let before = session.snapshot().unwrap();
    let before_docs = encode_documents(before.clone()).unwrap();
    let revision = session.revision();
    let add: Vec<_> = intents.iter().rev().copied().map(Record::from).collect();
    let mut params = json!({"board":target.board,"expected_revision":revision,"edits":{"add":add,"remove":[]},"dry_run":true});
    let preview = session.execute(EDIT_TEARDROPS, &params).unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    params["dry_run"] = json!(false);
    let applied = session.execute(EDIT_TEARDROPS, &params).unwrap();
    let diff = &applied["teardrops"];
    let expected: Vec<_> = intents.iter().copied().enumerate().filter(|(i, _)| *i != 1 && *i != 7).map(|(_, intent)| Record::from(intent)).collect();
    assert_eq!(diff["added"], json!(expected));
    assert_eq!(diff["copper"].as_array().unwrap().len(), 8);
    assert_eq!(diff["rejected"].as_array().unwrap().len(), 2);
    for (report, ((obstacle, _), i)) in diff["rejected"].as_array().unwrap().iter().zip(obstacles.iter().zip([1, 7])) {
        assert_eq!(report["intent"], json!(Record::from(intents[i])));
        assert_eq!(report["reason"]["kind"], "rule");
        let reason = &report["reason"]["detail"];
        assert_eq!(reason["rule"], "copper_clearance");
        assert_eq!(reason["other_object"], obstacle.to_string());
        assert_eq!(reason["required_nm"], 35_000);
        assert!((reason["actual_nm"].as_f64().unwrap() - 32_000.).abs() < 0.000_001);
    }
    for key in ["added", "removed", "copper", "rejected"] {
        assert_eq!(preview["teardrops"][key], diff[key]);
    }
    assert_eq!(view(&session).teardrops(), expected.as_slice());
    let applied_revision = session.revision();
    assert!(matches!(session.execute(EDIT_TEARDROPS, &params), Err(CommandError::StaleRevision { current }) if current == applied_revision));
    let blocked_only = json!({"board":target.board,"expected_revision":applied_revision,"edits":{"add":[Record::from(intents[1])],"remove":[]}});
    let current = session.snapshot().unwrap();
    assert_eq!(session.execute(EDIT_TEARDROPS, &blocked_only).unwrap()["teardrops"]["rejected"], json!([diff["rejected"][0]]));
    assert!(std::sync::Arc::ptr_eq(&current, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), applied_revision);
    let path = current.board_document_path().unwrap();
    let saved = encode_documents(current).unwrap();
    let mut board_json: serde_json::Value = serde_json::from_slice(saved.get(&path).unwrap()).unwrap();
    assert_eq!(board_json["format"], "pcbcraft-board/37");
    assert_eq!(board_json["teardrops"], json!(expected));
    let mut wrong_version = saved.clone();
    board_json["format"] = json!("pcbcraft-board/19");
    wrong_version.insert(path, serde_json::to_vec(&board_json).unwrap());
    assert!(matches!(decode_documents(&wrong_version), Err(error) if error.contains("teardrops require board format 37")));
    assert_eq!(view(&session).require_linear_copper().unwrap_err(), crate::board::teardrop::UNSUPPORTED);
    session.execute("edit.undo", &serde_json::Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_docs);
    session.execute("edit.redo", &serde_json::Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    session.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(view(&session).teardrops(), expected.as_slice());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    let remove: Vec<_> = expected.iter().map(|record| record.id).collect();
    let removed = session
        .execute(EDIT_TEARDROPS, &json!({"board":target.board,"expected_revision":session.revision(),"edits":{"add":[],"remove":remove}}))
        .unwrap();
    assert_eq!(removed["teardrops"]["removed"], json!(expected));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_docs);
}

/// A malformed second report fails the batch even after the first addition was accepted.
#[test]
fn refuses_malformed_rule_reports_without_returning_a_partial_plan() {
    let (session, intents, _) = fixture(2, false);
    let [first, second] = intents.as_slice() else { panic!("fixture intents") };
    let board = view(&session);
    let edits = [Edit::Add(*second), Edit::Add(*first)];
    let clearance = RuleBlock { rule: Rule::CopperClearance, other_object: first.track.to_string(), required_nm: 35_000, actual_nm: 34_999. };
    let contact = RuleBlock { rule: Rule::ConflictingContact, required_nm: 0, actual_nm: 0., ..clearance.clone() };
    let evaluate = |report: RuleBlock| {
        plan(board, &[], &edits, |candidate, _, accepted| {
            if candidate.intent == *second {
                assert_eq!(accepted.len(), 1);
                assert_eq!(accepted[0].intent, *first);
                Ok::<_, ()>(Some(report.clone()))
            } else {
                Ok(None)
            }
        })
    };
    let edge_spacing = RuleBlock { rule: Rule::BoardEdgeClearance, other_object: "board-outline".into(), ..clearance.clone() };
    let edge_crossing = RuleBlock { rule: Rule::BoardEdgeCrossing, required_nm: 0, actual_nm: 0., ..edge_spacing.clone() };
    let outside = RuleBlock { rule: Rule::OutsideBoardOutline, actual_nm: 40_000., ..edge_spacing.clone() };
    for valid in
        [clearance.clone(), contact.clone(), RuleBlock { actual_nm: -1., ..contact.clone() }, edge_spacing.clone(), edge_crossing.clone(), outside]
    {
        let diff = evaluate(valid.clone()).unwrap();
        assert_eq!(diff.rejected, vec![Rejected { intent: *second, reason: Refusal::Rule(valid) }]);
    }
    for invalid in [
        RuleBlock { actual_nm: 35_000., ..clearance.clone() },
        RuleBlock { actual_nm: 35_000., ..edge_spacing.clone() },
        RuleBlock { actual_nm: 40_000., ..edge_spacing.clone() },
        RuleBlock { actual_nm: 0., required_nm: 0, ..edge_spacing.clone() },
        RuleBlock { actual_nm: -1., ..edge_spacing },
        RuleBlock { actual_nm: 1., ..edge_crossing },
        RuleBlock { actual_nm: f64::NAN, ..clearance.clone() },
        RuleBlock { actual_nm: f64::NEG_INFINITY, ..contact.clone() },
        RuleBlock { required_nm: -1, ..contact.clone() },
        RuleBlock { other_object: " ".into(), ..contact.clone() },
        RuleBlock { actual_nm: 1., ..contact },
    ] {
        assert_eq!(evaluate(invalid), Err(Error::InvalidRuleReport(second.id)));
    }
}

/// A requested removal is included in the diff and absent from the evaluator's retained set.
#[test]
fn removes_without_requiring_geometry_and_reports_a_bad_new_site() {
    let (session, intents, _) = fixture(4, false);
    let [remove, keep, add, ..] = intents[..] else { panic!("fixture intents") };
    let bad = ViaTeardrop { track: Uuid::from_u128(100), ..intents[3] };
    let diff = plan(view(&session), &[keep, remove], &[Edit::Add(bad), Edit::Add(add), Edit::Remove(remove.id)], |candidate, retained, accepted| {
        assert_eq!(candidate.intent, add);
        assert_eq!(retained, &[keep]);
        assert!(accepted.is_empty());
        Ok::<_, ()>(None)
    })
    .unwrap();
    assert_eq!(diff.removed, vec![remove]);
    assert_eq!(diff.added[0].intent, add);
    assert_eq!(diff.rejected, vec![Rejected { intent: bad, reason: Refusal::Shape(ShapeError::MissingTrack) }]);
}

/// Invalid batches never reach rule evaluation, and incomplete evaluation yields no partial diff.
#[test]
fn rejects_bad_batches_and_fails_closed_if_rules_are_unavailable() {
    let (session, intents, _) = fixture(2, false);
    let board = view(&session);
    let [a, b, ..] = intents[..] else { panic!("fixture intents") };
    let unavailable = |_: &Addition, _: &[ViaTeardrop], _: &[Addition]| -> Result<Option<RuleBlock>, ()> { panic!("rule call before preflight") };
    let same_site = ViaTeardrop { id: b.id, ..a };
    let foreign = ViaTeardrop { board: BoardTarget { board: Uuid::nil(), ..board.target }, ..a };
    assert_eq!(plan(board, &[a], &[], unavailable), Ok(Plan { board: board.target, added: vec![], removed: vec![], rejected: vec![] }));
    assert_eq!(plan(board, &vec![a; MAX_INTENTS + 1], &[], unavailable), Err(Error::TooManyIntents));
    for (edits, error) in [
        ([Edit::Add(a), Edit::Add(a)], Error::DuplicateId(a.id)),
        ([Edit::Add(a), Edit::Add(same_site)], Error::DuplicateSite { via: a.via, track: a.track }),
        ([Edit::Add(a), Edit::Add(foreign)], Error::WrongBoard(a.id)),
        ([Edit::Add(a), Edit::Remove(b.id)], Error::MissingRemoval(b.id)),
    ] {
        assert_eq!(plan(board, &[], &edits, unavailable), Err(error));
    }
    assert_eq!(
        plan(board, &[], &[Edit::Add(b), Edit::Add(a)], |candidate, _, accepted| {
            if candidate.intent == a {
                Ok(None)
            } else {
                assert_eq!(accepted[0].intent, a);
                Err("policy unavailable")
            }
        }),
        Err(Error::RuleEvaluation { id: b.id, cause: "policy unavailable" }),
    );
}
