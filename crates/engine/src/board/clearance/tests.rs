//! Original command-built fixtures traverse the same native geometry as GUI/MCP operations.
use super::*;
use crate::{
    board::{
        definition::PadShape,
        drc::{
            self,
            contour_tests::{fixture, pose},
            short_tests::{Bridge, bridged_split, coincident_named_pads},
        },
        track::tests::{copper_layers, routed_fixture, view},
    },
    persistence::encode_documents,
};
use serde_json::json;

fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Repeated views share one immutable geometry/index; source changes and reopen cannot adopt
/// stale data. Diagnostic clearances can reuse geometry without reusing result authority.
#[test]
fn query_cache_reuses_unchanged_source_and_rejects_changed_snapshots() {
    let mut s = routed_fixture();
    let a = query(&s, 200_000);
    let b = query(&s, 0);
    assert!(Arc::ptr_eq(&a.geometry, &b.geometry));
    assert!(Arc::ptr_eq(&a.geometry.index(), &b.geometry.index()));
    let layer = copper_layers(&s)[0];
    s.apply(
        crate::ADD_TRACKS,
        &json!({"board":view(&s).target.board,"layer":layer,"width_nm":250_000,
        "points":[p(50_000_000,50_000_000),p(51_000_000,50_000_000)],"expected_revision":s.revision()}),
    )
    .unwrap();
    let changed = query(&s, 200_000);
    assert!(!Arc::ptr_eq(&a.geometry, &changed.geometry));
    assert!(!a.authority.is_current(&s));
    s.apply("edit.undo", &json!({})).unwrap();
    let undone = query(&s, 200_000);
    assert_eq!(undone.objects().len(), a.objects().len());
    assert!(!changed.authority.is_current(&s));
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(query(&s, 200_000).objects().len(), changed.objects().len());
    let snapshot = s.snapshot().unwrap();
    s.open_snapshot(snapshot);
    assert!(!changed.authority.is_current(&s));
    assert!(!Arc::ptr_eq(&changed.geometry, &query(&s, 200_000).geometry));
}

/// Contention affects only cache reuse; a reader must not wait behind another cache holder.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn query_cache_contention_does_not_block_a_valid_query() {
    let s = routed_fixture();
    let guard = s.clearance_cache.lock().unwrap();
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let session = &s;
        scope.spawn(move || {
            let _ = send.send(query(session, 200_000).objects().len());
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(guard);
        assert!(result.is_ok(), "query waited on a cache lock");
    });
}

/// Source replacements, additions and deletions keep stable identities and exhaustive neighbors,
/// preserve the base index, and cancel by dropping disposable data without document edits.
#[test]
fn overlay_changes_are_atomic_disposable_and_keep_all_exact_neighbors() {
    let s = routed_fixture();
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let q = query(&s, 200_000);
    let base_index = q.geometry.index();
    let layer = copper_layers(&s)[0];
    let original = q.objects()[0].contour.clone();
    let moved = Contour::segment(p(70_000_000, 80_000_000), p(80_000_000, 90_000_000), 60_001).unwrap();
    let id = Uuid::from_u128(100_001);
    let mut overlay = q.overlay();
    overlay.replace(0, moved.clone(), vec![layer]).unwrap();
    assert!(!overlay.candidates(&original, &[layer]).unwrap().contains(&ObstacleId::Source(0)));
    assert!(overlay.candidates(&moved, &[layer]).unwrap().contains(&ObstacleId::Source(0)));
    overlay.add(id, original.clone(), vec![layer], Some(0)).unwrap();
    let before = overlay.candidates(&original, &[layer]).unwrap();
    let result = overlay.probe(&original, &[layer], None).unwrap();
    assert!(result.is_current(&overlay, &s));
    assert!(before.contains(&ObstacleId::Added(id)));
    assert!(overlay.add(id, moved.clone(), vec![layer], None).is_err());
    assert!(overlay.replace(0, original.clone(), vec![Uuid::from_u128(999)]).is_err());
    assert!(overlay.remove(q.objects().len()).is_err());
    assert_eq!(before, overlay.candidates(&original, &[layer]).unwrap());
    assert!(result.is_current(&overlay, &s), "rejection preserves overlay authority");
    for contour in [&original, &moved] {
        let actual = overlay.probe(contour, &[layer], None).unwrap();
        let decisions: std::collections::BTreeMap<_, _> = actual.decisions.iter().map(|d| (d.obstacle, d)).collect();
        for obstacle in (0..q.objects().len()).map(ObstacleId::Source).chain([ObstacleId::Added(id)]) {
            if let Some(item) = overlay.object(obstacle).filter(|i| i.layers.contains(&layer)) {
                let gap = contour.gap(&item.contour);
                if gap <= 200_000. {
                    let decision = decisions[&obstacle];
                    assert_eq!(decision.gap_nm, gap);
                    assert_eq!(decision.touches, gap <= 0.);
                }
            }
        }
        assert!(actual.authority.is_current(&s));
    }
    overlay.remove(0).unwrap();
    assert!(!result.is_current(&overlay, &s));
    assert!(!result.is_current(&q.overlay(), &s), "a replacement gesture has different authority");
    assert!(!overlay.candidates(&moved, &[layer]).unwrap().contains(&ObstacleId::Source(0)));
    overlay.remove_added(id).unwrap();
    assert!(overlay.object(ObstacleId::Added(id)).is_none());
    drop(overlay);
    assert!(Arc::ptr_eq(&base_index, &q.geometry.index()));
    assert_eq!(q.objects()[0].contour.bounds(), original.bounds());
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
}

fn query(s: &Session, clearance_nm: i64) -> ClearanceQuery {
    s.clearance_query(view(s).target, clearance_nm).unwrap()
}

/// Compare actionable candidate decisions with all shared-layer pairs and the DRC consumer.
/// The reference deliberately bypasses all spatial pruning, retaining contacts at zero rule.
fn compare(s: &Session, clearance_nm: i64) {
    let q = query(s, clearance_nm);
    let actionable = |results: Results| results.decisions.into_iter().filter(|d| d.clearance_violation || d.touches).collect::<Vec<_>>();
    assert_eq!(actionable(q.pairs()), actionable(q.exhaustive_pairs()));
    let pairs: BTreeSet<_> = q
        .pairs()
        .decisions
        .into_iter()
        .filter(|d| d.clearance_violation)
        .map(|d| {
            let mut names = vec![q.objects()[d.first.unwrap()].name.clone(), q.objects()[d.second].name.clone()];
            names.sort();
            names
        })
        .collect();
    let report = view(s).check_design(drc::Rules { clearance_nm, ..drc::Rules::default() });
    let drc_pairs: BTreeSet<_> = report.violations.into_iter().filter(|v| v.kind == drc::Kind::Clearance).map(|v| v.items).collect();
    assert_eq!(pairs, drc_pairs);
    // Every object's probe has the same represented gap/contact as exhaustive source-pair tests,
    // while intentionally omitting the legacy physical-island clearance exemption for a probe.
    for (index, item) in q.objects().iter().enumerate() {
        let probe = q.probe(&item.contour, &item.layers, Some(index)).unwrap();
        let candidates: BTreeSet<_> = q.candidates(&item.contour, &item.layers).unwrap().into_iter().collect();
        for (other, obstacle) in q.objects().iter().enumerate().filter(|(i, _)| *i != index) {
            if !share_layers(&item.layers, &obstacle.layers) {
                continue;
            }
            let gap = item.contour.gap(&obstacle.contour);
            assert_eq!(gap, obstacle.contour.gap(&item.contour), "symmetric contour separation");
            if gap <= clearance_nm as f64 {
                assert!(candidates.contains(&other), "candidate omission: {index}, {other}, {gap}");
                let decision = probe.decisions.iter().find(|d| d.second == other).unwrap();
                assert_eq!(decision.gap_nm, gap);
                assert_eq!(decision.touches, gap <= 0.);
            }
        }
    }
}

/// All supported pads, non-quarter rotations, reflections, fractional radii, equality and both
/// sides of a boundary are compared with exhaustive evaluation on production-resolved contours.
#[test]
fn sweep_and_probes_match_exhaustive_all_pairs_for_supported_shapes() {
    for shape in [PadShape::Circle, PadShape::Oval, PadShape::RoundedRectangle, PadShape::Rectangle] {
        let height = 600_001;
        let width = if shape == PadShape::Circle { height } else { 1_000_001 };
        for (rotation, bottom) in [(0, false), (37, false), (90, true)] {
            for delta in [-1, 0, 1] {
                let mut s = fixture(shape, width, height, p(0, (height + 60_001) / 2 + 20_000 + delta), 60_001);
                pose(&mut s, rotation, bottom);
                let layers = copper_layers(&s);
                let at = p(0, -340_001);
                s.apply(
                    crate::ADD_TRACKS,
                    &json!({"board":view(&s).target.board,"layer":layers[0],"width_nm":60_001,
                    "points":[at,p(100_000,-340_001)],"expected_revision":s.revision()}),
                )
                .unwrap();
                s.apply(
                    crate::ADD_VIA,
                    &json!({"board":view(&s).target.board,"at":p(-340_001,0),
                    "via_diameter_nm":60_001,"via_drill_nm":50_000,"expected_revision":s.revision()}),
                )
                .unwrap();
                for rule in [0, 20_000, 200_000] {
                    compare(&s, rule);
                }
            }
        }
    }
}

/// Identical local display names on different sheets remain distinct memberships. Same global
/// names intentionally share membership, and opposite-side SMD copper never becomes a candidate.
#[test]
fn intended_membership_and_layers_do_not_depend_on_display_names() {
    for accepted in [false, true] {
        for (style, opposite) in [("local", false), ("global", false), ("local", true)] {
            let mut s = coincident_named_pads(3, style, opposite);
            if accepted {
                s.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"board":view(&s).target.board,"expected_revision":s.revision()})).unwrap();
            }
            compare(&s, 0);
            compare(&s, 200_000);
            let q = query(&s, 0);
            let conflicts: Vec<_> = q.pairs().decisions.into_iter().filter(|d| d.conflicting_nets).collect();
            assert_eq!(conflicts.len(), if style == "local" { if opposite { 1 } else { 3 } } else { 0 });
        }
    }
}

/// Routed conflicting-net islands keep every intended membership, and a zero rule still
/// observes short contacts. Shared-net controls and unset memberships retain clearance policy.
#[test]
fn conflicting_islands_and_unset_memberships_are_explicit() {
    for bridge in [Bridge::Track, Bridge::Vias, Bridge::Pads] {
        let s = bridged_split(bridge);
        compare(&s, 0);
        compare(&s, 200_000);
        let q = query(&s, 0);
        assert!(q.pairs().decisions.iter().any(|d| d.conflicting_nets));
        for item in q.objects().iter().filter(|o| o.name.parse::<Uuid>().is_ok()) {
            assert_eq!(item.intended_nets.len(), 2, "a conflicting island cannot choose its first net");
        }
    }
    let s = fixture(PadShape::Circle, 600_000, 600_000, p(0, 200_000), 60_000);
    let q = query(&s, 200_000);
    assert!(q.objects().iter().all(|o| o.intended_nets.is_empty()));
    assert!(q.pairs().decisions.iter().any(|d| d.clearance_violation && d.touches && !d.conflicting_nets));
    let s = coincident_named_pads(2, "global", false);
    let q = query(&s, 200_000);
    assert!(q.pairs().decisions.iter().any(|d| d.touches && !d.clearance_violation && !d.conflicting_nets));
}

/// Results survive query disposal but become stale after editing, Undo/Redo, reopening the same
/// Arc or adoption into another session. Inspection and refused operations never add history.
#[test]
fn captured_results_retain_source_and_reject_obsolete_authority() {
    let mut s = routed_fixture();
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let original = s.snapshot().unwrap();
    let revision = s.revision();
    let q = query(&s, 200_000);
    let results = q.pairs();
    drop(q);
    assert!(results.authority.is_current(&s));
    assert_eq!(results.authority.target(), view(&s).target);
    assert_eq!(results.authority.revision(), revision);
    assert_eq!(results.authority.clearance_nm(), 200_000);
    let other_rule = query(&s, 0);
    assert_eq!(other_rule.authority().clearance_nm(), 0);
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    let mut other = Session::default();
    other.open_snapshot(original.clone());
    assert!(!results.authority.is_current(&other));
    let layer = copper_layers(&s)[0];
    assert!(
        s.apply(
            crate::ADD_TRACKS,
            &json!({"board":view(&s).target.board,"layer":layer,"width_nm":250_000,
        "points":[p(0,30_000_000)],"expected_revision":revision})
        )
        .is_err()
    );
    assert!(results.authority.is_current(&s));
    s.apply(
        crate::ADD_TRACKS,
        &json!({"board":view(&s).target.board,"layer":layer,"width_nm":250_000,
        "points":[p(0,30_000_000),p(1_000_000,30_000_000)],"expected_revision":revision}),
    )
    .unwrap();
    assert!(!results.authority.is_current(&s));
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(!results.authority.is_current(&s), "restoring geometry does not restore authority");
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    s.apply("edit.redo", &json!({})).unwrap();
    assert!(!results.authority.is_current(&s));
    s.open_snapshot(original);
    assert!(!results.authority.is_current(&s));
}

/// Query preparation and probes reject malformed targets, dimensions, layers and source indices
/// before they can mutate state. A degenerate preview is supported without accepting zero-length
/// authored tracks, and exact fractional-radius tangency is independent of clearance equality.
#[test]
fn probes_validate_inputs_and_define_degenerate_boundaries() {
    let s = routed_fixture();
    let mut target = view(&s).target;
    target.board = Uuid::from_u128(9);
    assert!(s.clearance_query(target, 0).is_err());
    target = view(&s).target;
    target.project = Uuid::from_u128(9);
    assert!(s.clearance_query(target, 0).is_err());
    for rule in [-1, crate::schematic::MAX_CENTER_NM + 1] {
        assert!(s.clearance_query(view(&s).target, rule).is_err());
    }
    let q = query(&s, 0);
    let c = Contour::segment(p(0, 0), p(0, 0), 60_001).unwrap();
    let d = Contour::segment(p(60_001, 0), p(60_001, 0), 60_001).unwrap();
    assert_eq!(c.gap(&d), 0.);
    assert!(c.touches(&d));
    assert_eq!(c.gap(&Contour::segment(p(60_002, 0), p(60_002, 0), 60_001).unwrap()), 1.);
    for (start, width) in [(p(0, 0), 0), (p(crate::schematic::MAX_CENTER_NM + 1, 0), 60_001)] {
        assert!(Contour::segment(start, start, width).is_err());
    }
    assert!(q.candidates(&c, &[]).is_err());
    assert!(q.candidates(&c, &[Uuid::from_u128(9)]).is_err());
    let technical = view(&s).layers().into_iter().find(|l| l.purpose != super::super::setup::LayerPurpose::Copper).unwrap().id;
    assert!(q.candidates(&c, &[technical]).is_err());
    assert!(q.probe(&c, &copper_layers(&s), Some(q.objects().len())).is_err());
}

/// Compatibility display labels cannot exempt unresolved copper on separate islands.
/// Only derived presentation metadata is varied; the board and unknown electrical membership
/// come from the original command-built fixture and remain unchanged.
#[test]
fn equal_display_names_do_not_exempt_unresolved_membership() {
    let s = fixture(PadShape::Circle, 600_000, 600_000, p(0, 349_999), 60_000);
    let mut q = query(&s, 20_000);
    assert!(q.objects().iter().all(|item| item.intended_nets.is_empty()));
    q.geometry = Arc::new(BoardGeometry::new(view(&s)));
    for item in &mut Arc::get_mut(&mut q.geometry).unwrap().items {
        item.net = Some("DISPLAY_ONLY".into());
    }
    let decisions: Vec<_> = q.geometry.clearance_pairs(20_000).collect();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].gap_nm, 19_999.);
    assert!(decisions[0].clearance_violation);
    assert!(!decisions[0].conflicting_nets);
}

/// An unnamed route must discover both contacted intended nets before deciding short safety;
/// zero required clearance cannot turn a bridge into a legal preview. The analogous command
/// rejects it atomically through the same represented-contour contact path.
#[test]
fn unknown_probe_bridging_two_nets_agrees_with_commit_short_rejection() {
    let mut s = routed_fixture();
    let q = query(&s, 0);
    let footprint = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
    let (start, end) = (footprint.pads[0].center, footprint.pads[1].center);
    let layer = copper_layers(&s)[0];
    let contour = Contour::segment(start, end, 250_000).unwrap();
    let result = q.probe(&contour, &[layer], None).unwrap();
    assert!(result.decisions.iter().filter(|d| d.conflicting_nets).count() >= 2);
    assert!(result.decisions.iter().all(|d| !d.clearance_violation), "zero rule does not replace short detection");
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    assert!(
        s.apply(
            crate::ADD_TRACKS,
            &json!({"board":view(&s).target.board,"layer":layer,"width_nm":250_000,
        "points":[start,end],"expected_revision":revision})
        )
        .is_err()
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    assert!(q.authority().is_current(&s));
}

/// The broad phase must handle long diagonals, coincident bounds and tiny gaps near both
/// coordinate limits. Contacts and their nearby clearance boundaries retain fractional radii.
#[test]
fn candidate_bounds_cover_long_diagonals_and_coordinate_extremes() {
    let mut s = routed_fixture();
    let layers = copper_layers(&s);
    let limit = crate::schematic::MAX_CENTER_NM - 20_000_000;
    for (index, sign) in [-1, 1].into_iter().enumerate() {
        let at = p(sign * limit, sign * limit);
        s.apply(
            crate::ADD_TRACKS,
            &json!({"board":view(&s).target.board,"layer":layers[index],"width_nm":250_001,
            "points":[at,p(at.x_nm+10_000_000,at.y_nm+10_000_000)],"expected_revision":s.revision()}),
        )
        .unwrap();
        s.apply(
            crate::ADD_VIA,
            &json!({"board":view(&s).target.board,"at":p(at.x_nm+5_000_000,at.y_nm+5_000_000),
            "via_diameter_nm":600_001,"via_drill_nm":300_000,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    for rule in [0, 1, 200_000] {
        compare(&s, rule);
    }
}

/// Direct pad contact also carries intended memberships to routed copper, even when that track
/// reaches only the outer side of one pad. Logical renaming creates the pre-existing short via
/// commands after the originally legal placement and route, without modifying project files.
#[test]
fn routed_membership_includes_conflicts_reached_through_direct_pad_contact() {
    let mut s = coincident_named_pads(2, "global", false);
    s.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"board":view(&s).target.board,"expected_revision":s.revision()})).unwrap();
    let snapshot = s.snapshot().unwrap();
    let footprints: Vec<_> = snapshot.board_view(view(&s).target).unwrap().footprints().collect();
    let first = footprints[0].pads.iter().find(|pad| pad.net.is_some()).unwrap();
    let c = first.copper.unwrap();
    let shift = (c[1].x_nm - c[0].x_nm) / 2;
    let moved = &footprints[1];
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"board":view(&s).target.board,"footprint":moved.id,
        "x_nm":moved.position.x_nm+shift,"y_nm":moved.position.y_nm,"rotation":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    let start = p(c[0].x_nm + 10_000, first.center.y_nm);
    s.apply(
        crate::ADD_TRACKS,
        &json!({"board":view(&s).target.board,"layer":copper_layers(&s)[0],"width_nm":10_000,
        "points":[start,p(start.x_nm-2_000_000,start.y_nm)],"expected_revision":s.revision()}),
    )
    .unwrap();
    let sheet = s.project().unwrap().sheets().last().unwrap().target;
    let label = s.project().unwrap().for_sheet(sheet).unwrap().net_labels()[0].clone();
    s.apply(
        crate::net_labels::SET,
        &json!({"sheet":sheet.id(),"id":label.object_id(),"name":"OTHER","style":"global",
        "x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"rotation":label.rotation,"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::board::update::PREVIEW_UPDATE, &json!({"board":view(&s).target.board,"expected_revision":s.revision()})).unwrap();
    let token = s.cached_board_update().unwrap().plan_token;
    s.apply(crate::board::update::APPLY_UPDATE, &json!({"board":view(&s).target.board,"plan_token":token,"expected_revision":s.revision()})).unwrap();
    let q = query(&s, 0);
    assert_eq!(q.objects().iter().find(|item| item.name.parse::<Uuid>().is_ok()).unwrap().intended_nets.len(), 2);
    compare(&s, 0);
}
