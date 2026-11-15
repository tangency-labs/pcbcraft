//! Class consumers exercise command-built original boards and the production native codecs.
use super::*;
use crate::board::{
    clearance::{Decision, ObstacleId, Results},
    drc::{self, Kind, Report},
    track::tests::{copper_layers, routed_fixture, view},
};
use crate::{
    Session,
    nets::{Anchor, AnchorKind, ClassTarget},
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
    schematic::Point,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Every authored fixture edit uses the ordinary revision-checked command boundary.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    if !matches!(command, "project.new" | "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// Capture the newly created stable definition; names remain editable fixture input only.
fn class(session: &mut Session, name: &str, rules: Value) -> Uuid {
    let before: BTreeSet<_> = session.project().unwrap().net_classes().definitions.iter().map(|class| class.id).collect();
    apply(session, net_classes::CREATE, json!({"name":name,"rules":rules}));
    session.project().unwrap().net_classes().definitions.iter().find(|class| !before.contains(&class.id)).unwrap().id
}

/// Use the exact current physical component and terminal identity, including cross-sheet cases.
fn assign(session: &mut Session, index: usize, terminal: &str, classes: &[Uuid]) {
    let project = session.project().unwrap();
    let component = *project.components.keys().nth(index).unwrap();
    let anchor = Anchor {
        kind: AnchorKind::Terminal,
        project: project.id,
        scope: project.net_scope(),
        occurrence: Uuid::nil(),
        object: component,
        terminal: terminal.into(),
    };
    apply(session, crate::nets::SET_CLASSES, json!({"target":ClassTarget::Derived{anchor},"classes":classes}));
}

/// Resolve the real terminal-to-pad map; this fixture intentionally swaps pad numbers.
fn pad(session: &Session, index: usize, terminal: &str) -> (String, Point) {
    let project = session.project().unwrap();
    let component = *project.components.keys().nth(index).unwrap();
    let terminal = project.component_terminal(component, terminal).unwrap();
    let pad = project.pad_for_terminal(terminal).unwrap();
    let address = pad.inspect();
    (format!("{}:{}", address["footprint"].as_str().unwrap(), address["pad"].as_str().unwrap()), project.board.pad_position(pad).unwrap())
}

/// Two accepted nets get different dimensional intent without changing accepted geometry.
fn classified() -> (Session, Uuid, Uuid) {
    let mut session = routed_fixture();
    let power = class(
        &mut session,
        "Power",
        json!({"clearance_nm":500000,"min_track_width_nm":400000,"preferred_track_width_nm":500000,
        "min_via_diameter_nm":800000,"preferred_via_diameter_nm":900000,"preferred_via_drill_nm":400000,"min_annular_ring_nm":200000}),
    );
    let signal = class(&mut session, "Signal", json!({"clearance_nm":250000,"preferred_track_width_nm":200000}));
    assign(&mut session, 0, "1", &[power]);
    assign(&mut session, 1, "2", &[signal]);
    (session, power, signal)
}

/// Compare actionable indexed decisions with the exhaustive production contour reference.
fn actionable(results: Results) -> Vec<Decision> {
    results.decisions.into_iter().filter(|decision| decision.touches || decision.clearance_violation || decision.conflicting_nets).collect()
}

/// One immutable source produces identical synchronous/incremental checks, including provenance.
fn reports(session: &Session) -> Report {
    let report = view(session).check_design(rules::Values::default());
    let mut job = session.design_check_job(&json!({"board":view(session).target.board})).unwrap();
    while !job.step(7) {}
    assert_eq!(job.take_report().unwrap(), report);
    assert_eq!(session.check_design(&json!({"board":view(session).target.board})).unwrap(), json!(report));
    report
}

/// Folder and packed reopen preserve rule and binding authority without any source library.
fn reopen(session: &Session) {
    let project = session.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    assert_eq!(encode_documents(decode_packed(&encode_packed(project).unwrap()).unwrap().project).unwrap(), documents);
}

/// Different contacts supply different defaults, and the exact same policies reach commands,
/// preparation inspection, authored copper and the read-only MCP result.
#[test]
fn route_defaults_and_explicit_dimensions_use_the_addressed_net() {
    let (mut session, power, signal) = classified();
    let layer = copper_layers(&session)[0];
    for (component, terminal, width, class_id) in [(0, "1", 500000, power), (1, "2", 200000, signal)] {
        let (name, at) = pad(&session, component, terminal);
        let policy = view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap();
        assert_eq!(policy.track_width(None).unwrap(), width);
        assert_eq!(policy.resolution.classes, vec![class_id]);
        assert_eq!(policy.bindings.len(), 1);
        let snapshot = session.snapshot().unwrap();
        let revision = session.revision();
        let query = session.execute(ROUTE_RULES, &json!({"board":view(&session).target.board,"at":at,"layer":layer})).unwrap();
        assert_eq!(query["route_rules"]["applied"]["resolution"]["classes"], json!([class_id]));
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
        let end = Point { x_nm: at.x_nm, y_nm: at.y_nm + 2000000 };
        let mut job = session
            .prepare_copper_edit(
                crate::ADD_TRACKS,
                &json!({"board":view(&session).target.board,"layer":layer,"points":[at,end],"expected_revision":revision}),
            )
            .unwrap();
        while !job.step(7) {}
        let prepared = job.take_result().unwrap().unwrap();
        let context = prepared.class_rules().unwrap();
        assert_eq!(context.rules_for(&name).unwrap().resolution.classes, vec![class_id]);
        session.commit_copper_edit(prepared).unwrap();
        assert!(view(&session).tracks().iter().any(|track| track.start == at && track.width_nm == width));
    }
    let at = pad(&session, 0, "1").1;
    let policy = view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap();
    assert_eq!(policy.via_sizes(None, None).unwrap(), (900000, 400000));
    apply(&mut session, crate::ADD_VIA, json!({"at":at}));
    assert!(view(&session).vias().iter().any(|via| via.at == at && via.diameter_nm == 900000 && via.drill_nm == 400000));
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let power_track = view(&session).tracks().iter().find(|track| track.width_nm == 500000).unwrap().id;
    for (command, params) in [
        (crate::SET_TRACK_WIDTH, json!({"tracks":[power_track],"width_nm":200000})),
        (crate::ADD_TRACKS, json!({"layer":layer,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm-2000000}],"width_nm":200000})),
        (crate::SET_VIA_SIZE, json!({"vias":[view(&session).vias()[0].id],"via_diameter_nm":600000,"via_drill_nm":300000})),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(session.revision());
        assert!(session.apply(command, &params).is_err());
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    }
    apply(&mut session, crate::SET_TRACK_WIDTH, json!({"tracks":[power_track],"width_nm":450000}));
    assert_eq!(view(&session).tracks().iter().find(|track| track.id == power_track).unwrap().width_nm, 450000);
    reopen(&session);
}

/// Candidate expansion uses the strongest captured requirement, while each pair/probe uses its
/// own symmetric threshold. Weaker request baselines retain explicit provenance and class limits.
#[test]
fn pairs_probes_and_overlays_match_exhaustive_class_spacing() {
    let (session, power, signal) = classified();
    let layer = copper_layers(&session)[0];
    let (name, at) = pad(&session, 0, "1");
    let q = session.clearance_query_with_rules(view(&session).target, &json!({"clearance_nm":0})).unwrap();
    assert_eq!(q.authority().clearance_nm(), 500000);
    assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
    let index = q.objects().iter().position(|item| item.name == name).unwrap();
    let bounds = q.objects()[index].contour.bounds();
    let point = Point { x_nm: bounds[2].round() as i64 + 300000, y_nm: at.y_nm };
    let contour = super::super::geometry::Contour::capsule(point, point, 0.);
    let result = q.probe(&contour, &[layer], None).unwrap();
    let decision = result.decisions.iter().find(|decision| decision.second == index).unwrap();
    assert!(decision.clearance_violation);
    assert_eq!(decision.clearance_nm, 500000);
    assert_eq!(decision.gap_nm, 300000.);
    let policy = q.authority().class_rules().unwrap().rules_for(&name).unwrap();
    assert!(
        policy.resolution.contributions[&Property::ClearanceNm]
            .iter()
            .any(|source| matches!(source.source, net_classes::Source::Request) && source.value_nm == 0)
    );
    let pair = session.project().unwrap().resolve_class_clearance(&[power], &[signal], Some(view(&session).target)).unwrap();
    assert_eq!(pair.clearance_nm, decision.clearance_nm);
    let id = Uuid::from_u128(888);
    let remote = Point { x_nm: 80000000, y_nm: 90000000 };
    let mut overlay = q.overlay();
    overlay.add(id, super::super::geometry::Contour::capsule(remote, remote, 100000.), vec![layer], Some(index)).unwrap();
    let probe = Point { x_nm: remote.x_nm + 400000, y_nm: remote.y_nm };
    let result = overlay.probe(&super::super::geometry::Contour::capsule(probe, probe, 0.), &[layer], None).unwrap();
    let decision = result.decisions.iter().find(|decision| decision.obstacle == ObstacleId::Added(id)).unwrap();
    assert!(decision.clearance_violation);
    assert_eq!(decision.clearance_nm, 500000);
    assert_eq!(result.rules_for(ObstacleId::Added(id)).unwrap().resolution.classes, vec![power]);
    assert!(result.is_current(&overlay, &session));
    overlay.remove_added(id).unwrap();
    assert!(!result.is_current(&overlay, &session));
    assert!(q.probe(&contour, &[copper_layers(&session)[1]], None).unwrap().decisions.iter().all(|decision| decision.second != index));
}

/// Assigning rules does not resize old copper. Both checkers report its real dimensions with
/// class identity/revision and origin rather than claiming the board-default result passed.
#[test]
fn checkers_apply_width_via_and_edge_rules_with_provenance() {
    let mut session = routed_fixture();
    drc::tests::outline(&mut session);
    let at = pad(&session, 0, "1").1;
    let layer = copper_layers(&session)[0];
    apply(&mut session, crate::ADD_TRACKS, json!({"layer":layer,"width_nm":100000,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm+2000000}]}));
    apply(&mut session, crate::ADD_VIA, json!({"at":at,"via_diameter_nm":600000,"via_drill_nm":300000}));
    let power = class(
        &mut session,
        "Power",
        json!({"min_track_width_nm":400000,"preferred_track_width_nm":500000,
        "min_via_diameter_nm":800000,"preferred_via_diameter_nm":900000,"preferred_via_drill_nm":400000,"min_annular_ring_nm":200000,"edge_clearance_nm":10000000,"clearance_nm":500000}),
    );
    assign(&mut session, 0, "1", &[power]);
    let report = reports(&session);
    assert_eq!(report.counts.get(&Kind::TrackWidth), Some(&1));
    assert_eq!(report.counts.get(&Kind::ViaDiameter), Some(&1));
    assert_eq!(report.counts.get(&Kind::AnnularRing), Some(&1));
    assert!(report.counts.get(&Kind::EdgeClearance).copied().unwrap_or(0) > 0);
    let context = report.class_rules.as_ref().unwrap();
    let track = &view(&session).tracks()[0];
    let applied = context.rules_for(&track.id.to_string()).unwrap();
    assert_eq!(applied.resolution.classes, vec![power]);
    assert!(
        applied.resolution.contributions[&Property::MinTrackWidthNm]
            .iter()
            .any(|source| matches!(source.source,net_classes::Source::Class{rule,revision:1} if rule==power))
    );
    let q = session.clearance_query_with_rules(view(&session).target, &json!({})).unwrap();
    let pairs: BTreeSet<_> = q
        .pairs()
        .decisions
        .into_iter()
        .filter(|decision| decision.clearance_violation)
        .map(|decision| {
            let mut names = vec![q.objects()[decision.first.unwrap()].name.clone(), q.objects()[decision.second].name.clone()];
            names.sort();
            names
        })
        .collect();
    assert_eq!(
        report.violations.iter().filter(|finding| finding.kind == Kind::Clearance).map(|finding| finding.items.clone()).collect::<BTreeSet<_>>(),
        pairs
    );
    reopen(&session);
}

/// Preference conflicts stay local to omitted routing defaults; explicit valid dimensions and
/// unrelated spacing/track checks remain usable and retain the unresolved source reports.
#[test]
fn preference_conflicts_do_not_block_unrelated_consumers_or_nets() {
    let (mut session, power, signal) = classified();
    let conflict = class(&mut session, "Alternative signal", json!({"preferred_track_width_nm":300000}));
    assign(&mut session, 1, "2", &[signal, conflict]);
    let layer = copper_layers(&session)[0];
    let power_at = pad(&session, 0, "1").1;
    let signal_at = pad(&session, 1, "2").1;
    let policy = view(&session).route_rules_at(signal_at, Some(layer)).unwrap().unwrap();
    assert!(policy.track_width(None).is_err());
    assert_eq!(policy.track_width(Some(250000)).unwrap(), 250000);
    apply(&mut session, crate::ADD_TRACKS, json!({"layer":layer,"points":[power_at,Point{x_nm:power_at.x_nm,y_nm:power_at.y_nm+2000000}]}));
    let snapshot = session.snapshot().unwrap();
    assert!(session.apply(crate::ADD_TRACKS,&json!({"layer":layer,"points":[signal_at,Point{x_nm:signal_at.x_nm,y_nm:signal_at.y_nm+2000000}],"expected_revision":session.revision()})).is_err());
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    apply(
        &mut session,
        crate::ADD_TRACKS,
        json!({"layer":layer,"points":[signal_at,Point{x_nm:signal_at.x_nm,y_nm:signal_at.y_nm+2000000}],"width_nm":250000}),
    );
    let report = reports(&session);
    assert!(!report.counts.contains_key(&Kind::NetClassConflict));
    assert!(report.class_rules.as_ref().unwrap().policies.iter().any(|policy| !policy.resolution.conflicts.is_empty()));
    let q = session.clearance_query_with_rules(view(&session).target, &json!({})).unwrap();
    assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
    assert!(view(&session).route_rules_at(power_at, Some(layer)).unwrap().unwrap().resolution.classes.contains(&power));
}

/// Class edits invalidate every captured authority even across Undo/reopen. Refused copper edits
/// preserve history and pending board review, while successful rule edits leave physical bytes alone.
#[test]
fn rule_edits_invalidate_queries_jobs_and_pending_review_authority() {
    let (mut session, power, _) = classified();
    let layer = copper_layers(&session)[0];
    let at = pad(&session, 0, "1").1;
    apply(&mut session, crate::ADD_TRACKS, json!({"layer":layer,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm+2000000}]}));
    let track = view(&session).tracks()[0].id;
    let q = session.clearance_query_with_rules(view(&session).target, &json!({})).unwrap();
    let mut check = session.design_check_job(&json!({})).unwrap();
    let source = check.source().clone();
    let params =
        json!({"layer":layer,"width_nm":500000,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm-2000000}],"expected_revision":session.revision()});
    let mut job = session.prepare_copper_edit(crate::ADD_TRACKS, &params).unwrap();
    while !job.step(7) {}
    let prepared = job.take_result().unwrap().unwrap();
    apply(&mut session, crate::PREVIEW_UPDATE, json!({}));
    let before = session.snapshot().unwrap();
    let review = session.board_update_report();
    let documents = encode_documents(before.clone()).unwrap();
    assert!(session.apply(crate::SET_TRACK_WIDTH, &json!({"tracks":[track],"width_nm":200000,"expected_revision":session.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(review, session.board_update_report());
    apply(
        &mut session,
        net_classes::UPDATE,
        json!({"class":power,"name":"Renamed power","rules":{"clearance_nm":700000,"min_track_width_nm":500000,"preferred_track_width_nm":600000}}),
    );
    let current = session.snapshot().unwrap();
    assert_eq!(
        encode_documents(current.clone()).unwrap()[&current.board_document_path().unwrap()],
        documents[&before.board_document_path().unwrap()]
    );
    assert!(!q.authority().is_current(&session));
    assert!(!source.is_current(&session));
    assert!(!prepared.source().is_current(&session));
    assert!(session.commit_copper_edit(prepared).is_err());
    assert!(Arc::ptr_eq(&current, &session.snapshot().unwrap()));
    assert!(session.board_update_report().is_none());
    while !check.step(7) {}
    assert!(check.take_report().is_some());
    assert!(!source.is_current(&session));
    let policy = view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap();
    assert_eq!(policy.track_width(None).unwrap(), 600000);
    assert_eq!(policy.clearance_nm().unwrap(), 700000);
    assert!(
        policy.resolution.contributions[&Property::ClearanceNm]
            .iter()
            .any(|source| matches!(source.source,net_classes::Source::Class{rule,revision:2} if rule==power))
    );
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents);
    assert!(!q.authority().is_current(&session));
    apply(&mut session, "edit.redo", json!({}));
    reopen(&session);
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    session.open_snapshot(decode_documents(&saved).unwrap());
    assert!(!q.authority().is_current(&session));
    assert_eq!(view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap().track_width(None).unwrap(), 600000);
    let label = session.project().unwrap().net_labels()[0].clone();
    apply(
        &mut session,
        crate::net_labels::SET,
        json!({"id":label.object_id(),"name":"RENAMED_ACCEPTED","style":"global",
        "x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"rotation":label.rotation}),
    );
    let pending_policy = view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap();
    let tracks = view(&session).tracks().to_vec();
    let review = session.execute(crate::PREVIEW_UPDATE, &json!({"expected_revision":session.revision()})).unwrap();
    apply(&mut session, crate::APPLY_UPDATE, json!({"plan_token":review["board_update_preview"]["plan_token"]}));
    assert_eq!(view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap(), pending_policy);
    assert_eq!(view(&session).tracks(), tracks);
    reports(&session);
    reopen(&session);
}

/// An impossible via policy affects a classified via, not pads/tracks or another net's routing.
/// Reports preserve the contradiction instead of silently selecting a source or weakening a bound.
#[test]
fn hard_conflicts_are_limited_to_consumed_dimensions_and_memberships() {
    let (mut session, _, signal) = classified();
    let layer = copper_layers(&session)[0];
    let at = pad(&session, 1, "2").1;
    apply(&mut session, crate::ADD_VIA, json!({"at":at}));
    let via = view(&session).vias()[0].id;
    let minimum = class(&mut session, "Large drill", json!({"min_via_drill_nm":700000}));
    let maximum = class(&mut session, "Small drill", json!({"max_via_drill_nm":600000}));
    assign(&mut session, 1, "2", &[signal, minimum, maximum]);
    let endpoint = Point { x_nm: at.x_nm, y_nm: at.y_nm + 2000000 };
    apply(&mut session, crate::ADD_TRACKS, json!({"layer":layer,"points":[at,endpoint]}));
    let policy = view(&session).route_rules_at(at, Some(layer)).unwrap().unwrap();
    assert_eq!(policy.track_width(None).unwrap(), 200000);
    assert!(policy.via_sizes(Some(1200000), Some(650000)).is_err());
    let snapshot = session.snapshot().unwrap();
    assert!(
        session
            .apply(crate::ADD_VIA, &json!({"at":endpoint,"via_diameter_nm":1200000,"via_drill_nm":650000,"expected_revision":session.revision()}))
            .is_err()
    );
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    let power_at = pad(&session, 0, "1").1;
    apply(&mut session, crate::ADD_TRACKS, json!({"layer":layer,"points":[power_at,Point{x_nm:power_at.x_nm,y_nm:power_at.y_nm+2000000}]}));
    let report = reports(&session);
    let conflicts: Vec<_> = report.violations.iter().filter(|finding| finding.kind == Kind::NetClassConflict).collect();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].items, vec![via.to_string()]);
    let q = session.clearance_query_with_rules(view(&session).target, &json!({})).unwrap();
    assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
    reopen(&session);
}

/// Equal local display names do not merge class authority. Contacting several intended nets
/// retains all sources for probes/shorts, while unknown copper receives no invented assignment.
#[test]
fn equal_names_and_conflicting_islands_preserve_distinct_identity() {
    let mut session = drc::short_tests::coincident_named_pads(2, "local", false);
    let power = class(&mut session, "Power", json!({"clearance_nm":500000}));
    let signal = class(&mut session, "Signal", json!({"clearance_nm":250000}));
    assign(&mut session, 0, "1", &[power]);
    assign(&mut session, 1, "1", &[signal]);
    let (name, at) = pad(&session, 0, "1");
    let unknown = pad(&session, 0, "2").0;
    let layer = copper_layers(&session)[0];
    let q = session.clearance_query_with_rules(view(&session).target, &json!({"clearance_nm":0})).unwrap();
    let context = q.authority().class_rules().unwrap();
    assert_eq!(context.rules_for(&name).unwrap().resolution.classes, vec![power]);
    assert!(context.rules_for(&unknown).is_none());
    let probe = q.probe(&super::super::geometry::Contour::capsule(at, at, 0.), &[layer], None).unwrap();
    assert!(probe.decisions.iter().any(|decision| decision.conflicting_nets));
    assert_eq!(probe.probe_rules.as_ref().unwrap().resolution.classes.iter().copied().collect::<BTreeSet<_>>(), BTreeSet::from([power, signal]));
    let report = reports(&session);
    let short = report.violations.iter().find(|finding| finding.kind == Kind::Short).unwrap();
    assert_eq!(short.nets.len(), 2);
    assert!(short.nets.iter().all(|net| net.name.ends_with("::LOCAL")));
    assert!(view(&session).net_class_target_for_copper(&name).is_err(), "conflicting physical contact cannot choose a class target");
    let before = session.snapshot().unwrap();
    assert!(
        session
            .apply(
                crate::ADD_TRACKS,
                &json!({"layer":layer,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm+2000000}],"expected_revision":session.revision()})
            )
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
}

/// Class spacing retains analytic contour boundaries, equality, rotation/side transforms and
/// layer separation for every currently supported pad shape.
#[test]
fn class_spacing_uses_exact_contours_and_strict_boundaries() {
    use crate::board::definition::PadShape;
    for shape in [PadShape::Circle, PadShape::Oval, PadShape::RoundedRectangle, PadShape::Rectangle] {
        for (rotation, bottom) in [(0, false), (90, false), (0, true), (90, true)] {
            let (width, height) = if shape == PadShape::Circle { (800000, 800000) } else { (1000000, 600000) };
            let mut session = drc::contour_tests::fixture(shape, width, height, Point { x_nm: 10000000, y_nm: 0 }, 200000);
            let terminal = session.project().unwrap().symbols()[0].terminals()[0];
            let at = session.project().unwrap().terminal_position(terminal).unwrap();
            apply(&mut session, crate::net_labels::ADD, json!({"name":"Shape","style":"local","x_nm":at.x_nm,"y_nm":at.y_nm}));
            let class = class(&mut session, "Spacing", json!({"clearance_nm":500000}));
            assign(&mut session, 0, "1", &[class]);
            drc::contour_tests::pose(&mut session, rotation, bottom);
            let layer = copper_layers(&session)[usize::from(bottom)];
            let (name, center) = pad(&session, 0, "1");
            let q = session.clearance_query_with_rules(view(&session).target, &json!({"clearance_nm":0})).unwrap();
            let index = q.objects().iter().position(|item| item.name == name).unwrap();
            let right = q.objects()[index].contour.bounds()[2].round() as i64;
            for gap in [499999, 500000] {
                let point = Point { x_nm: right + gap, y_nm: center.y_nm };
                let result = q.probe(&super::super::geometry::Contour::capsule(point, point, 0.), &[layer], None).unwrap();
                let decision = result.decisions.iter().find(|decision| decision.second == index).unwrap();
                assert_eq!(decision.gap_nm, gap as f64, "{shape:?} {rotation} {bottom}");
                assert_eq!(decision.clearance_violation, gap < 500000);
                assert_eq!(decision.clearance_nm, 500000);
            }
            assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
        }
    }
}

/// Removing a binding revokes captured policy and restores ordinary board defaults. A net or
/// definition bearing the old display name cannot recreate the removed assignment.
#[test]
fn assignment_removal_revokes_captured_rules_without_name_inference() {
    let (mut session, power, _) = classified();
    let layer = copper_layers(&session)[0];
    let (name, at) = pad(&session, 0, "1");
    let q = session.clearance_query_with_rules(view(&session).target, &json!({})).unwrap();
    let preparation = session.copper_preparation(view(&session).target).unwrap();
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assign(&mut session, 0, "1", &[]);
    assert!(!q.authority().is_current(&session));
    assert!(!preparation.source().is_current(&session));
    assert!(view(&session).route_rules_at(at, Some(layer)).unwrap().is_none());
    let after = session.snapshot().unwrap();
    assert_eq!(encode_documents(after.clone()).unwrap()[&after.board_document_path().unwrap()], documents[&after.board_document_path().unwrap()]);
    assert_eq!(q.authority().class_rules().unwrap().rules_for(&name).unwrap().resolution.classes, vec![power]);
    let component = *session.project().unwrap().components.keys().next().unwrap();
    let terminal = session.project().unwrap().component_terminal(component, "1").unwrap();
    let named = session.project().unwrap().terminal_position(terminal).unwrap();
    apply(&mut session, crate::net_labels::ADD, json!({"name":"Power","style":"global","x_nm":named.x_nm,"y_nm":named.y_nm}));
    assert!(view(&session).route_rules_at(at, Some(layer)).unwrap().is_none());
    let before = session.snapshot().unwrap();
    for params in [json!({"board":Uuid::from_u128(999),"at":at,"layer":layer}), json!({"at":at,"layer":Uuid::from_u128(999)})] {
        assert!(session.execute(ROUTE_RULES, &params).is_err());
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    apply(&mut session, "edit.undo", json!({}));
    apply(&mut session, "edit.undo", json!({}));
    let undone = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(undone.keys().collect::<Vec<_>>(), documents.keys().collect::<Vec<_>>());
    for (path, bytes) in &documents {
        if path == "project.json" {
            // Allocated numeric IDs are never reused after Undo; only that high-water metadata advances.
            let mut before: Value = serde_json::from_slice(bytes).unwrap();
            let mut after: Value = serde_json::from_slice(&undone[path]).unwrap();
            assert!(after["numeric_id_high_water"].as_u64() >= before["numeric_id_high_water"].as_u64());
            before.as_object_mut().unwrap().remove("numeric_id_high_water");
            after.as_object_mut().unwrap().remove("numeric_id_high_water");
            assert_eq!(after, before);
        } else {
            assert_eq!(undone[path], *bytes, "{path}");
        }
    }
    assert!(!q.authority().is_current(&session));
    reopen(&session);
}

/// Classified pad contact work yields with a small budget and can be dropped without changing
/// source authority. Draining the same edit under another budget retains identical policy.
#[test]
fn classified_preparation_yields_and_cancellation_preserves_the_source() {
    let (session, _, _) = classified();
    let layer = copper_layers(&session)[0];
    let at = pad(&session, 0, "1").1;
    let snapshot = session.snapshot().unwrap();
    let params = json!({"layer":layer,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm+2000000}],
        "width_nm":500000,"expected_revision":session.revision()});
    let mut cancelled = session.prepare_copper_edit(crate::ADD_TRACKS, &params).unwrap();
    assert!(!cancelled.step(0));
    for _ in 0..3 {
        assert!(!cancelled.step(1));
    }
    assert!(cancelled.take_result().unwrap().is_none());
    drop(cancelled);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    let mut policies = Vec::new();
    for budget in [1, 7, usize::MAX] {
        let mut job = session.prepare_copper_edit(crate::ADD_TRACKS, &params).unwrap();
        let mut steps = 0;
        while !job.step(budget) {
            steps += 1;
            assert!(steps < 1000);
        }
        let prepared = job.take_result().unwrap().unwrap();
        policies.push(prepared.class_rules().unwrap().policies.clone());
    }
    assert!(policies.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
}

/// A single explicit via operand may repair a preferred pair, but cannot select a winner
/// among differing class preferences or weaken any hard bound. Commands retain the same rule.
#[test]
fn explicit_via_operand_repairs_only_unambiguous_preferences() {
    let (mut session, _, _) = classified();
    let ring = class(&mut session, "Ring", json!({"min_annular_ring_nm":200000}));
    assign(&mut session, 0, "1", &[ring]);
    let at = pad(&session, 0, "1").1;
    let policy = view(&session).route_rules_at(at, None).unwrap().unwrap();
    assert!(policy.via_sizes(None, None).is_err());
    assert_eq!(policy.via_sizes(Some(800000), None).unwrap(), (800000, 300000));
    assert_eq!(policy.via_sizes(None, Some(200000)).unwrap(), (600000, 200000));
    assert!(policy.via_sizes(Some(650000), None).is_err());
    for (operand, value, expected) in [("via_diameter_nm", 800000, (800000, 300000)), ("via_drill_nm", 200000, (600000, 200000))] {
        let mut params = json!({"at":at});
        params[operand] = json!(value);
        apply(&mut session, crate::ADD_VIA, params);
        let via = &view(&session).vias()[0];
        assert_eq!((via.diameter_nm, via.drill_nm), expected);
        reopen(&session);
        apply(&mut session, "edit.undo", json!({}));
    }
    let small = class(&mut session, "Drill A", json!({"preferred_via_drill_nm":200000}));
    let large = class(&mut session, "Drill B", json!({"preferred_via_drill_nm":300000}));
    assign(&mut session, 0, "1", &[ring, small, large]);
    let policy = view(&session).route_rules_at(at, None).unwrap().unwrap();
    assert!(policy.via_sizes(Some(900000), None).is_err());
    let before = session.snapshot().unwrap();
    assert!(session.apply(crate::ADD_VIA, &json!({"at":at,"via_diameter_nm":900000,"expected_revision":session.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let a = class(&mut session, "Diameter A", json!({"preferred_via_diameter_nm":800000}));
    let b = class(&mut session, "Diameter B", json!({"preferred_via_diameter_nm":900000}));
    assign(&mut session, 0, "1", &[ring, a, b]);
    assert!(view(&session).route_rules_at(at, None).unwrap().unwrap().via_sizes(None, Some(200000)).is_err());
}

/// Schematic terminals, pads and newly routed copper address exactly the same current net.
/// Copper detached from all intended membership is unsupported; unknown object IDs fail.
#[test]
fn selection_targets_match_schematic_and_refuse_unknown_or_floating_copper() {
    let (mut s, _, _) = classified();
    let project = s.project().unwrap();
    let component = *project.components.keys().next().unwrap();
    let terminal = project.component_terminal(component, "1").unwrap();
    let logical = project.net_class_target_for_endpoint(project.sheet_target(), crate::WireEndpoint::Terminal(terminal)).unwrap();
    let (address, at) = pad(&s, 0, "1");
    assert_eq!(view(&s).net_class_target_for_copper(&address).unwrap(), logical);
    let layer = copper_layers(&s)[0];
    apply(&mut s, crate::ADD_TRACKS, json!({"layer":layer,"points":[at,Point{x_nm:at.x_nm,y_nm:at.y_nm+2_000_000}]}));
    let id = view(&s).tracks().last().unwrap().id;
    assert_eq!(view(&s).net_class_target_for_copper(&id.to_string()).unwrap(), logical);
    let before = s.snapshot().unwrap();
    assert!(view(&s).net_class_target_for_copper("missing").is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    apply(&mut s, crate::ADD_TRACKS, json!({"layer":layer,"points":[Point{x_nm:90_000_000,y_nm:90_000_000},Point{x_nm:95_000_000,y_nm:90_000_000}]}));
    let id = view(&s).tracks().iter().find(|track| track.start.x_nm == 90_000_000).unwrap().id;
    assert_eq!(view(&s).net_class_target_for_copper(&id.to_string()).unwrap(), None);
}

/// Accepted copper cannot silently choose a surviving branch while schematic connectivity
/// has split into distinct current memberships awaiting Update PCB.
#[test]
fn pending_board_split_refuses_selection_assignment() {
    let (mut s, _, _) = classified();
    let (name, _) = pad(&s, 0, "1");
    assert!(view(&s).net_class_target_for_copper(&name).unwrap().is_some());
    let wires: Vec<_> = s.project().unwrap().wires().iter().map(|wire| wire.id()).collect();
    for wire in wires {
        apply(&mut s, crate::DELETE_WIRE, json!({"wire":wire}));
    }
    let before = s.snapshot().unwrap();
    assert!(view(&s).net_class_target_for_copper(&name).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}
